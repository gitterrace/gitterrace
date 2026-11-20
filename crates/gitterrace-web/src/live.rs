//! Live updates: `GET /events` is a server-sent event stream per repository. The watcher in
//! `app.rs` sends one `change` event whenever a part of the repository moved; `static/live.js`
//! decides what each page does about it.

use std::{convert::Infallible, sync::Arc, time::Duration};

use axum::{
    extract::{Query, State},
    http::{StatusCode, header},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures_util::stream;
use serde::{Deserialize, Serialize};
use tokio::sync::{OwnedSemaphorePermit, broadcast, watch};

use crate::app::{App, Env};

/// Seconds between the comment lines that keep an idle stream open through proxies and tell the
/// browser the server is alive.
const HEARTBEAT: Duration = Duration::from_secs(15);

/// Every part of the repository, for a stream that cannot say what it missed.
const ALL: [&str; 6] = ["head", "refs", "stash", "worktrees", "index", "worktree"];

/// The kind that says the settings changed, see `static/live.js`.
const CONFIG: &str = "config";

/// The kind that says a repository joined or left the server, see `static/live.js`.
const REPOS: &str = "repos";

/// What moved, and the state the chrome shows, read once for all listeners.
pub(crate) struct Change {
    pub(crate) kinds: Vec<&'static str>,
    /// Digest of the fingerprint, as a decimal string: it does not fit a JavaScript number.
    pub(crate) v: u64,
    pub(crate) banner: Option<String>,
    /// The Working tree chip in the top bar.
    pub(crate) dirty: bool,
}

impl Change {
    /// The settings were reloaded: pages rendered before it show stale values. Carries no chrome.
    pub(crate) fn config(v: u64) -> Self {
        Self { kinds: vec![CONFIG], v, banner: None, dirty: false }
    }

    /// A repository joined or left the server: the switcher and the home change. No chrome.
    pub(crate) fn repos(v: u64) -> Self {
        Self { kinds: vec![REPOS], v, banner: None, dirty: false }
    }
}

/// How a page reacts to a change, read by `live.js` from `data-live` and `data-live-on`.
#[derive(Clone, Copy)]
pub(crate) struct Mode {
    /// `refresh` re-renders the page in place, `notice` shows a quiet "Updated, refresh" note,
    /// empty ignores changes and never opens a stream.
    pub(crate) how: &'static str,
    /// Space separated kinds the page shows.
    pub(crate) on: &'static str,
}

impl Mode {
    /// Lists of commits and anything that follows HEAD.
    pub(crate) const NOTICE: Self = Self { how: "notice", on: "head refs" };
    /// Branches, tags, stashes and worktrees are all counted on the refs page.
    pub(crate) const REFS: Self = Self { how: "notice", on: "head refs stash worktrees" };
    pub(crate) const WORKTREE: Self = Self { how: "refresh", on: "head refs index worktree" };

    /// A page about a full commit id never changes.
    pub(crate) fn pinned_to(rev: &str) -> bool {
        rev.len() >= 40 && rev.bytes().all(|b| b.is_ascii_hexdigit())
    }
}

#[derive(Deserialize)]
pub(crate) struct EventsQuery {
    /// Fingerprint digest the page was rendered at.
    v: Option<String>,
    /// `1` when the page shows the working tree, which turns the working tree scan on.
    wt: Option<String>,
}

/// Counts the open streams that need the working tree scanned; released on drop.
pub(crate) struct WorktreeWatch(Arc<App>);

impl WorktreeWatch {
    pub(crate) fn new(app: &Arc<App>) -> Self {
        app.wt_watchers.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        app.wake.notify_one();
        Self(Arc::clone(app))
    }
}

impl Drop for WorktreeWatch {
    fn drop(&mut self) {
        self.0.wt_watchers.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

/// The body of a `change` event for a settings or repository-list change: no chrome.
// Fields are declared in alphabetical order: this was a serde_json map, which sorts keys.
#[derive(Serialize)]
struct Bare<'a> {
    kinds: &'a [&'static str],
    v: String,
}

/// The body of a `change` event that also carries the chrome.
// Fields are declared in alphabetical order: this was a serde_json map, which sorts keys.
#[derive(Serialize)]
struct Full<'a> {
    banner: &'a Option<String>,
    dirty: bool,
    kinds: &'a [&'static str],
    v: String,
}

fn change_event<T: Serialize>(body: &T) -> Event {
    // Both bodies are plain strings and bools, which always serialize.
    Event::default().event("change").data(serde_json::to_string(body).unwrap_or_default())
}

fn event(c: &Change) -> Event {
    if c.kinds == [CONFIG] || c.kinds == [REPOS] {
        return change_event(&Bare { kinds: &c.kinds, v: c.v.to_string() });
    }
    change_event(&Full { banner: &c.banner, dirty: c.dirty, kinds: &c.kinds, v: c.v.to_string() })
}

/// A change for a stream that missed some: everything may have moved, the chrome is unknown.
fn everything(v: u64) -> Event {
    change_event(&Bare { kinds: &ALL, v: v.to_string() })
}

struct Stream {
    rx: broadcast::Receiver<Arc<Change>>,
    stop: watch::Receiver<bool>,
    app: Arc<App>,
    first: Option<Event>,
    _slot: OwnedSemaphorePermit,
    _wt: Option<WorktreeWatch>,
}

struct HomeStream {
    rx: broadcast::Receiver<Arc<Change>>,
    stop: watch::Receiver<bool>,
    env: Env,
    first: Option<Event>,
    _slot: OwnedSemaphorePermit,
}

/// `GET /events`: one stream per tab. Refused with 503 past `server.live_streams`.
pub(crate) async fn events(State(app): State<Arc<App>>, Query(q): Query<EventsQuery>) -> Response {
    let Ok(slot) = Arc::clone(&app.streams).try_acquire_owned() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::RETRY_AFTER, "30")],
            "too many open pages for live updates\n",
        )
            .into_response();
    };
    let rx = app.changes.subscribe();
    app.wake.notify_one();
    // A page that was rendered before the newest change missed it, which also covers a
    // reconnect after a pause.
    let now = app.version();
    let behind = q.v.and_then(|v| v.parse::<u64>().ok()).is_some_and(|v| v != now);
    let state = Stream {
        rx,
        stop: app.stop.clone(),
        first: behind.then(|| everything(now)),
        _wt: (q.wt.as_deref() == Some("1")).then(|| WorktreeWatch::new(&app)),
        app,
        _slot: slot,
    };
    let body = stream::unfold(state, |mut s| async move {
        if let Some(e) = s.first.take() {
            return Some((Ok::<_, Infallible>(e), s));
        }
        loop {
            tokio::select! {
                got = s.rx.recv() => match got {
                    Ok(c) => return Some((Ok(event(&c)), s)),
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        let v = s.app.version();
                        return Some((Ok(everything(v)), s));
                    }
                    Err(broadcast::error::RecvError::Closed) => return None,
                },
                gone = s.stop.changed() => {
                    if gone.is_err() || *s.stop.borrow() {
                        return None;
                    }
                }
            }
        }
    });
    let mut res =
        Sse::new(body).keep_alive(KeepAlive::new().interval(HEARTBEAT).text("hb")).into_response();
    res.headers_mut().insert(header::CACHE_CONTROL, header::HeaderValue::from_static("no-store"));
    res
}

/// `GET /events` beside the repositories home: the same `change` stream, carrying only the
/// `repos` kind, so an open home lists a repository that joined or left. Refused with 503 past
/// `server.live_streams`.
pub(crate) fn home_events(env: &Env, q: &EventsQuery) -> Response {
    let Ok(slot) = Arc::clone(&env.home_streams).try_acquire_owned() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::RETRY_AFTER, "30")],
            "too many open pages for live updates\n",
        )
            .into_response();
    };
    let rx = env.home.subscribe();
    // A home rendered before the newest list change missed it, which also covers a reconnect.
    let now = env.home_version();
    let behind = q.v.as_deref().and_then(|v| v.parse::<u64>().ok()).is_some_and(|v| v != now);
    let state = HomeStream {
        rx,
        stop: env.stopping(),
        first: behind.then(|| event(&Change::repos(now))),
        env: env.clone(),
        _slot: slot,
    };
    let body = stream::unfold(state, |mut s| async move {
        if let Some(e) = s.first.take() {
            return Some((Ok::<_, Infallible>(e), s));
        }
        loop {
            tokio::select! {
                got = s.rx.recv() => match got {
                    Ok(c) => return Some((Ok(event(&c)), s)),
                    // Every notice says the same thing: one stands for the ones missed.
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        let v = s.env.home_version();
                        return Some((Ok(event(&Change::repos(v))), s));
                    }
                    Err(broadcast::error::RecvError::Closed) => return None,
                },
                gone = s.stop.changed() => {
                    if gone.is_err() || *s.stop.borrow() {
                        return None;
                    }
                }
            }
        }
    });
    let mut res =
        Sse::new(body).keep_alive(KeepAlive::new().interval(HEARTBEAT).text("hb")).into_response();
    res.headers_mut().insert(header::CACHE_CONTROL, header::HeaderValue::from_static("no-store"));
    res
}
