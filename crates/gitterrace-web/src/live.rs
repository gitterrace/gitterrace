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
use tokio::sync::{broadcast, watch};

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
    pub(crate) on: &'static str,
}

impl Mode {
    pub(crate) const NOTICE: Self = Self { how: "notice", on: "head refs" };
    pub(crate) const REFS: Self = Self { how: "notice", on: "head refs stash worktrees" };
    pub(crate) const WORKTREE: Self = Self { how: "refresh", on: "head refs index worktree" };

    /// A page about a full commit id never changes.
    pub(crate) fn pinned_to(rev: &str) -> bool {
        rev.len() >= 40 && rev.bytes().all(|b| b.is_ascii_hexdigit())
    }
}

#[derive(Deserialize)]
pub(crate) struct EventsQuery {
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
    let data = serde_json::to_string(body).unwrap_or_else(|_| "{}".to_owned());
    Event::default().event("change").data(data)
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

/// The 503 for a stream past its slot limit.
fn too_many() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(header::RETRY_AFTER, "30")],
        "too many open pages for live updates\n",
    )
        .into_response()
}

/// Streams `first`, then every change from `rx`. A receiver that lagged gets `lagged()`, since
/// one notice stands for the ones missed. `keep` is released when the stream drops.
fn sse(
    rx: broadcast::Receiver<Arc<Change>>,
    stop: watch::Receiver<bool>,
    first: Option<Event>,
    lagged: impl Fn() -> Event + Send + Sync + 'static,
    keep: impl Send + 'static,
) -> Response {
    let body = stream::unfold(
        (rx, stop, first, keep, Arc::new(lagged)),
        |(mut rx, mut stop, first, keep, lagged)| async move {
            let mut first = first;
            let out = if let Some(e) = first.take() {
                Some(e)
            } else {
                loop {
                    tokio::select! {
                        got = rx.recv() => match got {
                            Ok(c) => break Some(event(&c)),
                            Err(broadcast::error::RecvError::Lagged(_)) => break Some(lagged()),
                            Err(broadcast::error::RecvError::Closed) => break None,
                        },
                        gone = stop.changed() => {
                            if gone.is_err() || *stop.borrow() {
                                break None;
                            }
                        }
                    }
                }
            };
            out.map(|e| (Ok::<_, Infallible>(e), (rx, stop, first, keep, lagged)))
        },
    );
    crate::guard::no_store(
        Sse::new(body).keep_alive(KeepAlive::new().interval(HEARTBEAT).text("hb")).into_response(),
    )
}

/// `GET /events`: one stream per tab. Refused with 503 past `server.live_streams`.
pub(crate) async fn events(State(app): State<Arc<App>>, Query(q): Query<EventsQuery>) -> Response {
    let Some(slot) = app.streams.try_take() else {
        return too_many();
    };
    let rx = app.changes.subscribe();
    app.wake.notify_one();
    // A page that was rendered before the newest change missed it, which also covers a
    // reconnect after a pause.
    let now = app.version();
    let behind = q.v.and_then(|v| v.parse::<u64>().ok()).is_some_and(|v| v != now);
    let wt = (q.wt.as_deref() == Some("1")).then(|| WorktreeWatch::new(&app));
    let stop = app.stop.clone();
    let first = behind.then(|| everything(now));
    sse(rx, stop, first, move || everything(app.version()), (slot, wt))
}

/// `GET /events` beside the repositories home: the same `change` stream, carrying only the
/// `repos` kind, so an open home lists a repository that joined or left. Refused with 503 past
/// `server.live_streams`.
pub(crate) fn home_events(env: &Env, q: &EventsQuery) -> Response {
    let Some(slot) = env.home_streams.try_take() else {
        return too_many();
    };
    let rx = env.home.subscribe();
    // A home rendered before the newest list change missed it, which also covers a reconnect.
    let now = env.home_version();
    let behind = q.v.as_deref().and_then(|v| v.parse::<u64>().ok()).is_some_and(|v| v != now);
    let first = behind.then(|| event(&Change::repos(now)));
    let env = env.clone();
    sse(rx, env.stopping(), first, move || event(&Change::repos(env.home_version())), slot)
}
