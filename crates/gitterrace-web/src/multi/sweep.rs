//! Keeps the table in step with the folders it was started on. A served repository whose
//! folder (or its `.git`) is gone is taken off, as `DELETE /_repos` would: its URLs answer 404
//! and its background tasks and watchers stop. A repository that appears under a searched
//! folder, or comes back, is added under the name it had. File events on the searched folders
//! nudge a sweep, and a timer of `watch.rescan_secs` sweeps anyway, so a lost event or a
//! watcher that would not start costs at most that long.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use gitterrace_git::{Nudge, NudgeConfig, Nudger, Repo};
use tokio::sync::Notify;

use crate::actions::{Site, Target};
use crate::multi::discover::{discover_repos, is_repo};
use crate::multi::{Added, Served, Table, url_stem};

/// Quiet time after the last event before the sweep: folds a `rm -r` into one sweep.
const DEBOUNCE: Duration = Duration::from_millis(150);
/// Longest a burst of events holds the sweep back. Not configurable: `watch.rescan_secs`
/// bounds the rest.
const MAX_DELAY: Duration = Duration::from_millis(500);
/// Most inotify watches the searched folders may hold on Linux; above it only the timer sweeps.
/// Not configurable: a folder of repositories needs a few watches per repository at most.
const MAX_WATCHES: usize = 4096;

/// What one sweep changed: the repositories taken off and the ones added, not yet opened.
#[derive(Default)]
struct Swept {
    gone: Vec<String>,
    fresh: Vec<(Arc<Served>, Repo)>,
}

/// Starts sweeping when the server was started on folders to search. Needs a tokio runtime.
pub(crate) fn spawn(table: Arc<Table>, site: Arc<Site>) {
    if table.env.search.is_empty() {
        return;
    }
    tokio::spawn(run(table, site));
}

async fn run(table: Arc<Table>, site: Arc<Site>) {
    let nudged = Arc::new(Notify::new());
    let cfg = table.env.cfg();
    let roots: Vec<PathBuf> = table
        .env
        .search
        .iter()
        .map(PathBuf::as_path)
        .map(canon)
        .collect();
    let watch = NudgeConfig {
        roots: roots.clone(),
        skip: skipper(roots.clone(), cfg.multi.depth, cfg.multi.skip_dirs.clone()),
        debounce: DEBOUNCE,
        max_delay: MAX_DELAY,
        rescan: Duration::from_secs(cfg.watch.rescan_secs),
        max_watches: MAX_WATCHES,
        os_events: true,
    };
    let on = Arc::clone(&nudged);
    // Registration walks the folders: off the async workers. The watcher lives as long as the
    // server, so it is never dropped on one either.
    let started =
        tokio::task::spawn_blocking(move || Nudger::start(watch, move |_: Nudge| on.notify_one()))
            .await;
    let _watcher = match started {
        Ok(Ok(n)) => Some(n),
        Ok(Err(e)) => {
            tracing::warn!("{e}; checking the served folders for repositories by polling");
            None
        }
        Err(e) => {
            tracing::warn!("starting the repository folder watcher: {e}");
            None
        }
    };
    loop {
        let every = Duration::from_secs(table.env.cfg().watch.rescan_secs);
        tokio::select! {
            () = nudged.notified() => {}
            () = tokio::time::sleep(every) => {}
        }
        if table.env.cfg().watch.enabled {
            sweep_now(&table, &site).await;
        }
    }
}

/// One sweep: takes off what is gone, adds and opens what appeared, and tells open tabs.
pub(crate) async fn sweep_now(table: &Arc<Table>, site: &Site) {
    let t = Arc::clone(table);
    let Ok(swept) = tokio::task::spawn_blocking(move || sweep(&t)).await else {
        return;
    };
    for name in &swept.gone {
        site.drop_target(name);
        tracing::info!("{name} is gone from disk; no longer served");
    }
    let mut changed = !swept.gone.is_empty();
    for (served, repo) in swept.fresh {
        if let Err(e) = table.router_of(&served).await {
            tracing::warn!("opening {}: {e}", served.name);
            table.remove(&served.name);
            continue;
        }
        site.push_target(Target {
            key: served.name.clone(),
            base: format!("/r/{}", served.name),
            repo,
        });
        tracing::info!(
            "found {} on disk; serving it at /r/{}/",
            served.path.display(),
            served.name
        );
        changed = true;
    }
    if changed {
        table.announce();
    }
}

fn sweep(table: &Table) -> Swept {
    let mut out = Swept::default();
    for s in table.snapshot() {
        if !is_repo(&s.root) && table.drop_gone(&s.name) {
            out.gone.push(s.name.clone());
        }
    }
    let cfg = table.env.cfg();
    let found = discover_repos(&table.env.search, cfg.multi.depth, &cfg.multi.skip_dirs);
    for path in found.into_iter().filter(|p| is_repo(p)) {
        let Ok(repo) = Repo::discover(&path) else {
            continue;
        };
        match table.add(repo.root(), &url_stem(&repo.name())) {
            Ok(Added::Fresh(served)) => out.fresh.push((served, repo)),
            Ok(Added::Present { .. }) => {}
            Err(e) => tracing::warn!("not serving {}: {e}", path.display()),
        }
    }
    out
}

/// Watches the folders a repository can sit in, `depth` levels below a searched root, and a
/// repository's `.git` entry (so its removal is seen), never further inside one: its own
/// writes, the index cache included, never change which repositories exist.
fn skipper(roots: Vec<PathBuf>, depth: usize, skip: Vec<String>) -> gitterrace_git::Skip {
    Arc::new(move |p: &Path, _is_dir: bool| {
        let Some(rel) = roots.iter().find_map(|r| p.strip_prefix(r).ok()) else {
            return true;
        };
        let parts: Vec<_> = rel
            .components()
            .map(std::path::Component::as_os_str)
            .collect();
        let Some(last) = parts.last() else {
            return false;
        };
        let named = last.to_str().is_some_and(|n| skip.iter().any(|s| s == n));
        if parts.len() > depth + 1 || named {
            return true;
        }
        // Inside a repository only its `.git` entry counts: an edit to a tracked file must not
        // cost a sweep.
        let mut at = p.to_path_buf();
        for up in 1..parts.len() {
            at.pop();
            if is_repo(&at) {
                return !(up == 1 && *last == ".git");
            }
        }
        false
    })
}

fn canon(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

#[cfg(test)]
#[path = "sweep_tests.rs"]
mod tests;
