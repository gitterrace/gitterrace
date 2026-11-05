//! The background refresher: watches the repository, updates the index and tells open tabs.

use std::{
    sync::{Arc, Weak, atomic::Ordering},
    time::Duration,
};

use crate::app::events::Events;
use crate::app::{App, blocking};
use crate::live::Change;

impl App {
    /// Reacts to the parts of the repository that moved: drops stale caches and, when a tab is
    /// listening, tells it. Costs nothing but the cache reset while nobody listens.
    async fn changed(self: &Arc<Self>, kinds: Vec<&'static str>, v: u64) {
        self.invalidate(&kinds);
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
}

/// Keeps the commit index and the open tabs current. One fingerprint of cheap `stat` calls per
/// `watch.interval_ms` says whether HEAD, a ref or the index moved; only then is HEAD resolved
/// again. The working tree scans (a gix status) run only while a tab listens, and then only when
/// the OS reports a file change or `watch.rescan_secs` passes. Where file events are unavailable
/// they run every interval, as polling. With the watcher off nothing runs on a timer: a page that
/// finds the index behind queues a refresh.
pub(crate) async fn keep_index_fresh(app: Arc<App>) {
    let mut watcher: Option<gitterrace_git::Watcher> = None;
    let mut seen = None;
    let mut last: Option<String> = None;
    let mut check_head = true;
    let mut wt_seen: Option<u64> = None;
    let mut dirty_seen: Option<bool> = None;
    let mut events = Events::default();
    loop {
        // With `index.build_on_start` off the index waits for the first page that needs it;
        // the watcher below keeps running either way.
        // `watch.*` can change while running: the interval and the on switch are read per round.
        let cfg = app.cfg();
        let every = cfg
            .watch
            .enabled
            .then(|| Duration::from_millis(cfg.watch.interval_ms));
        if every.is_none() {
            watcher = None;
            seen = None;
        } else if watcher.is_none() {
            let mut w = app.repo.watcher();
            let p = w.parts();
            app.version.store(p.digest(), Ordering::Release);
            seen = Some(p);
            watcher = Some(w);
        }
        let listening = every.is_some() && app.changes.receiver_count() > 0;
        events.follow(&app, listening, cfg.watch.rescan_secs).await;
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
        let tick = async {
            match every {
                Some(d) => tokio::time::sleep(d).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            () = app.wake.notified() => continue,
            () = events.nudged.notified() => {}
            () = tick => {}
        }
        let Some(w) = watcher.as_mut() else { continue };
        let now = w.parts();
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
            let a = Arc::clone(&app);
            let sig = tokio::task::spawn_blocking(move || {
                gitterrace_git::worktree_signature(&a.repo).ok()
            })
            .await
            .ok()
            .flatten();
            app.wt_scans.fetch_add(1, Ordering::Release);
            if wt_seen.is_some() && sig.is_some() && sig != wt_seen {
                kinds.push("worktree");
            }
            if sig.is_some() {
                wt_seen = sig;
            }
        }
        if !kinds.is_empty() {
            check_head |= kinds.iter().any(|k| matches!(*k, "head" | "refs"));
            app.changed(kinds, now.digest()).await;
        }
        seen = Some(now);
    }
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
        Ok(Err(e)) => return tracing::warn!("index update for {rev} failed: {e}"),
        Err(e) => return tracing::warn!("index task failed: {e}"),
    }
    let Some(head) = warm else { return };
    app.mark_ready();
    let (a, h) = (Arc::clone(app), head.to_owned());
    // The languages walk reads the whole first-parent chain, so it runs here, off the
    // request path. What it keeps is flat in history length.
    tokio::task::spawn_blocking(move || {
        let _warm = a.warming();
        if let Ok(Some(r)) = a.source(&h) {
            a.chain(&h, &*r, &crate::languages::path_filter(&a, &h));
        }
    });
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
