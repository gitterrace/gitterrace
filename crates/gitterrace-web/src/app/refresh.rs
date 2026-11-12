//! The background refresher: watches the repository, updates the index and tells open tabs.

use std::{
    sync::{Arc, Weak, atomic::Ordering},
    time::Duration,
};

use crate::app::events::{POLL_FALLBACK, Scope, Watch};
use crate::app::{App, blocking};
use crate::live::Change;

impl App {
    /// Reacts to the parts of the repository that moved: drops stale caches and, when a tab is
    /// listening, tells it. Costs nothing but the cache reset while nobody listens.
    async fn changed(self: &Arc<Self>, kinds: Vec<&'static str>, v: u64) {
        self.invalidate(&kinds);
        if kinds.iter().any(|k| matches!(*k, "head" | "refs")) {
            self.warm_related();
        }
        self.version.store(v, Ordering::Release);
        if self.changes.receiver_count() == 0 {
            return;
        }
        let change = blocking(self, move |a| {
            Ok(Change {
                kinds,
                v,
                banner: crate::views::banner(&a.repo.head_state(), &a.base),
                dirty: a.dirty(),
            })
        })
        .await;
        if let Ok(c) = change {
            // An error only says every listener left meanwhile.
            self.changes.send(Arc::new(c)).ok();
        }
    }

    /// Warms the root sets behind the log's related links off the request path: this
    /// repository's, and any peer's not warmed lately. A page only reads what this caches.
    /// The walk takes no job slot and not the page's handle: it opens its own, so a page never
    /// queues behind it. The repository id comes first: permalinks need it, so the root walks
    /// queue behind it and never the other way round.
    fn warm_related(self: &Arc<Self>) {
        let app = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            app.repo.id();
            app.env.related.warm(&app.key);
        });
    }
}

/// Keeps the commit index and the open tabs current. OS file events on the git directories wake
/// it, and a fingerprint of cheap `stat` calls then says whether HEAD, a ref or the index moved;
/// only then is HEAD resolved again. A burst of git writes is one wake after the debounce. The
/// events' safety timer wakes it every `watch.rescan_secs` anyway, so a dropped event shows
/// within that bound. The working tree scans (a gix status) run only while a tab listens, and
/// then only when the OS reports a file change or `watch.rescan_secs` passes. Where file events
/// are unavailable both checks run every [`POLL_FALLBACK`]. With the watcher off nothing runs on
/// a timer: a page that finds the index behind queues a refresh.
pub(crate) async fn keep_index_fresh(app: Arc<App>) {
    let mut watcher: Option<gitterrace_git::Watcher> = None;
    let mut seen = None;
    let mut last: Option<String> = None;
    let mut check_head = true;
    let mut wt_seen: Option<u64> = None;
    let mut dirty_seen: Option<bool> = None;
    let mut events = Watch::new(Scope::Tree);
    let mut git_events = Watch::new(Scope::Git);
    loop {
        // With `index.build_on_start` off the index waits for the first page that needs it;
        // the watcher below keeps running either way.
        // `watch.*` can change while running: the rescan and the on switch are read per round.
        let cfg = app.cfg();
        let on = cfg.watch.enabled;
        if !on {
            watcher = None;
            seen = None;
        } else if watcher.is_none()
            && let Some((w, p)) = parts_off_worker(app.repo.watcher()).await
        {
            app.version.store(p.digest(), Ordering::Release);
            seen = Some(p);
            watcher = Some(w);
        }
        git_events.follow(&app, on.then_some(cfg.watch.rescan_secs));
        let listening = on && app.changes.receiver_count() > 0;
        events.follow(&app, listening.then_some(cfg.watch.rescan_secs));
        if cfg.index.build_on_start || app.demanded.load(Ordering::Acquire) {
            if app.rebuild.swap(false, Ordering::AcqRel) {
                check_head = true;
                last = None;
            }
            if check_head {
                check_head = false;
                let head = blocking(&app, |a| Ok(a.repo.resolve("HEAD")?)).await.ok();
                if head.is_some() && head != last {
                    refresh(&app, "HEAD", head.as_deref()).await;
                    last = head;
                }
            }
        }
        // Only a poll for missing events runs on this loop's own timer; otherwise the events (and
        // their safety timers) are the only wakes, so an idle server sleeps for `rescan_secs`.
        let poll = on && (git_events.polling() || (listening && events.polling()));
        // A new tab's baseline is owed now: taken at the next event, that change is never heard.
        let owed = events.pending()
            || git_events.pending()
            || (listening && wt_seen.is_none() && app.wt_watchers.load(Ordering::Acquire) > 0);
        let tick = tick(owed, poll);
        tokio::select! {
            () = app.wake.notified() => continue,
            () = events.nudged.notified() => {}
            () = git_events.nudged.notified() => {}
            () = tick => {}
        }
        // The git digest below is read every round, so the git nudge only needs clearing.
        git_events.take();
        let Some(w) = watcher.take() else { continue };
        let Some((w, now)) = parts_off_worker(w).await else {
            continue;
        };
        watcher = Some(w);
        let mut kinds = seen
            .filter(|b| *b != now)
            .map(|before| now.changed_since(&before))
            .unwrap_or_default();
        // A gix status runs only when something may have changed.
        // A tab that just opened the worktree page needs a baseline scan.
        let look =
            events.take() | (wt_seen.is_none() && app.wt_watchers.load(Ordering::Acquire) > 0);
        if app.wt_watchers.load(Ordering::Acquire) == 0 {
            wt_seen = None;
            // Any other page still shows the top bar chip: one early-exit check, and an event
            // only when the answer flips.
            if look
                && let Some(flip) = chip_flip(&app, &mut dirty_seen).await
                && kinds.is_empty()
            {
                app.set_dirty(flip);
                kinds.push("dirty");
            }
        } else if look {
            dirty_seen = None;
            if events.polling() {
                app.wt_polled.fetch_add(1, Ordering::Release);
            }
            if worktree_moved(&app, &mut wt_seen).await {
                kinds.push("worktree");
            }
        }
        if !kinds.is_empty() {
            check_head |= kinds.iter().any(|k| matches!(*k, "head" | "refs"));
            app.changed(kinds, now.digest()).await;
        }
        seen = Some(now);
    }
}

/// The watcher's fingerprint, read on the blocking pool: it is a `stat` per ref folder and a
/// directory listing, which must not stall the async workers. The watcher travels into the
/// closure and back. `None` when the blocking task panicked; the watcher is then rebuilt.
async fn parts_off_worker(
    mut w: gitterrace_git::Watcher,
) -> Option<(gitterrace_git::Watcher, gitterrace_git::Parts)> {
    tokio::task::spawn_blocking(move || {
        let p = w.parts();
        (w, p)
    })
    .await
    .map_err(|e| tracing::warn!("watcher fingerprint failed: {e}"))
    .ok()
}

/// The loop's own wake: at once for owed work, a poll interval for missing events, else never.
async fn tick(owed: bool, poll: bool) {
    if owed {
        return;
    }
    if poll {
        tokio::time::sleep(POLL_FALLBACK).await;
    } else {
        std::future::pending::<()>().await;
    }
}

/// One working tree scan for the open tabs: whether the tree moved since `seen`, which it then
/// updates. A new tab's baseline is the state its page was rendered from, not this scan, so an
/// edit made after the render and before this scan is still a change.
async fn worktree_moved(app: &Arc<App>, seen: &mut Option<u64>) -> bool {
    let a = Arc::clone(app);
    let sig = tokio::task::spawn_blocking(move || gitterrace_git::worktree_signature(&a.repo).ok())
        .await
        .ok()
        .flatten();
    app.wt_scans.fetch_add(1, Ordering::Release);
    // Every scan consumes the render slot: a render before this scan is covered by it.
    let rendered = app.wt_rendered.lock().ok().and_then(|mut g| g.take());
    if seen.is_none() {
        *seen = rendered;
    }
    let moved = seen.is_some() && sig.is_some() && sig != *seen;
    // A failed scan still counts as the baseline, so an owed baseline never spins.
    *seen = sig.or(*seen).or(Some(0));
    moved
}

/// The early-exit dirty check for the top bar chip, run only while a tab listens. Returns the
/// new answer when it differs from the last one seen.
async fn chip_flip(app: &Arc<App>, seen: &mut Option<bool>) -> Option<bool> {
    if app.changes.receiver_count() == 0 {
        *seen = None;
        return None;
    }
    let a = Arc::clone(app);
    let now = tokio::task::spawn_blocking(move || gitterrace_git::is_dirty(&a.repo))
        .await
        .ok();
    let Some(now) = now else {
        app.wt_scans.fetch_add(1, Ordering::Release);
        return None;
    };
    let flipped = seen.is_some_and(|b| b != now);
    *seen = Some(now);
    app.wt_scans.fetch_add(1, Ordering::Release);
    flipped.then_some(now)
}

/// Brings the index up to `rev`; for HEAD (`warm` holds its id) also builds what the first
/// visitor would otherwise wait for, then tells the warm-up.
async fn refresh(app: &Arc<App>, rev: &str, warm: Option<&str>) {
    let a = Arc::clone(app);
    let spec = rev.to_owned();
    let run = tokio::task::spawn_blocking(move || {
        a.index.update(&a.repo, &spec, &|p| a.set_progress(p))?;
        // Every page's chrome shows the repo id, which walks the whole history once per
        // process: walk it here, before ready, not inside the first page.
        a.repo.id();
        std::io::Result::Ok(())
    })
    .await;
    match run {
        Ok(Ok(())) => {}
        // A damaged cache met while updating: the loop sleeps until an event, so start the
        // rebuild here rather than wait for a page to meet it too.
        Ok(Err(e)) if gitterrace_index::feed::SourceError::of(&e).is_some() => {
            tracing::warn!("cache unreadable while updating {rev}, rebuilding: {e}");
            if let Err(e) = app.unreadable_now() {
                tracing::warn!("rebuilding the unreadable cache failed: {e}");
            }
            return;
        }
        Ok(Err(e)) => return tracing::warn!("index update for {rev} failed: {e}"),
        Err(e) => return tracing::warn!("index task failed: {e}"),
    }
    let Some(head) = warm else { return };
    // The tables the per-file pages share and the languages chain, built before ready: the
    // first page after ready reads them instead of racing their build (which doubled
    // hotspots on rust, and walked the chain twice for languages).
    let (a, h) = (Arc::clone(app), head.to_owned());
    let _ = tokio::task::spawn_blocking(move || {
        let _warm = a.warming();
        crate::insights::warm_shared(&a, &h);
    })
    .await;
    app.mark_ready();
    app.warm_related();
    app.refreshed.notify_one();
}

/// Waits for the index to move, then runs `warm` once per move.
pub(crate) async fn on_refreshed<F, Fut>(app: Arc<App>, mut warm: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    loop {
        app.refreshed.notified().await;
        warm().await;
    }
}

/// Releases the record sets of an app nobody browses, see [`App::release_idle`]. Checks a few
/// times per idle period, so the release lands at most a quarter period late. Holds only a weak
/// reference, so a removed repository's app is dropped and the loop ends.
pub(crate) async fn release_when_idle(app: Weak<App>, idle: Duration) {
    let mut tick = tokio::time::interval(idle / 4);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let Some(a) = app.upgrade() else { return };
        if a.release_idle(idle) {
            tracing::debug!("{}: released idle record sets", a.name);
        }
    }
}

