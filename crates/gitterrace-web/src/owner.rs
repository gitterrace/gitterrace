//! Proof that a write comes from the person running the server, not from another user of the
//! machine. Loopback is shared: any local user can fetch any page, so no page may hold a secret
//! and no header a client sets itself can count as proof.
//!
//! The secret is the per-process token in the 0600 file the add route already uses. A browser
//! gets proof by a launch link: `git terrace` reads the file, asks the server for a one-time
//! ticket (`POST /_launch`, secret in a header, no browser markers) and opens
//! `/_launch?t=TICKET`. Redeeming it sets an `HttpOnly`, `SameSite=Strict` cookie holding a hash of
//! the secret. Every write and the settings page demand that cookie, besides the same-origin
//! checks that stop other sites. A ticket works once and dies after a minute, so a copy that
//! reaches a process list or a history is no use later.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::{
    Router,
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use gitterrace_actions::Token;

/// How long a ticket opens the launch link.
const TICKET_TTL: Duration = Duration::from_secs(60);
/// Unredeemed tickets kept: the oldest goes first, so a flood cannot grow memory. Not
/// configurable: one launch needs one ticket, and the bound only caps a flood.
const MAX_TICKETS: usize = 16;

pub(crate) struct Owner {
    token: Arc<Token>,
    tickets: Mutex<Vec<(Token, Instant)>>,
}

#[cfg(test)]
thread_local! {
    /// The cookie of the last owner built on this thread, for the tests to send.
    static LAST_COOKIE: std::cell::RefCell<String> =
        const { std::cell::RefCell::new(String::new()) };
}

/// The cookie pair (`name=value`) the last server built on this thread accepts.
#[cfg(test)]
pub(crate) fn test_cookie() -> String {
    LAST_COOKIE.with(|c| c.borrow().clone())
}

impl Owner {
    pub(crate) const fn new(token: Arc<Token>) -> Self {
        Self { token, tickets: Mutex::new(Vec::new()) }
    }

    /// Makes `test_cookie` answer for this owner.
    #[cfg(test)]
    pub(crate) fn remember_for_tests(&self) {
        LAST_COOKIE.with(|c| *c.borrow_mut() = self.cookie());
    }

    /// The cookie pair a launched browser holds.
    pub(crate) fn cookie(&self) -> String {
        format!("{}={}", self.token.cookie_name(), self.token.session())
    }

    /// Whether the request carries the owner cookie.
    pub(crate) fn proven(&self, h: &HeaderMap) -> bool {
        let name = self.token.cookie_name();
        h.get_all(header::COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(';'))
            .filter_map(|kv| kv.trim().strip_prefix(&name)?.strip_prefix('='))
            .any(|v| self.token.verify_session(v))
    }

    fn tickets(&self) -> std::sync::MutexGuard<'_, Vec<(Token, Instant)>> {
        let mut g = self.tickets.lock().unwrap_or_else(PoisonError::into_inner);
        g.retain(|(_, at)| at.elapsed() < TICKET_TTL);
        g
    }

    fn mint(&self) -> Option<String> {
        let t = Token::generate().ok()?;
        let out = t.as_str().to_owned();
        let mut g = self.tickets();
        if g.len() >= MAX_TICKETS {
            g.remove(0);
        }
        g.push((t, Instant::now()));
        Some(out)
    }

    /// Spends `offered`: true once for a live ticket.
    fn redeem(&self, offered: &str) -> bool {
        let mut g = self.tickets();
        let Some(i) = g.iter().position(|(t, _)| t.verify(offered)) else {
            return false;
        };
        g.remove(i);
        true
    }
}

/// Mounted only on a loopback bind, never public.
pub(crate) fn router(owner: Arc<Owner>) -> Router {
    Router::new().route("/_launch", get(redeem).post(mint)).with_state(owner)
}

#[derive(serde::Deserialize)]
struct Launch {
    #[serde(default)]
    t: String,
    #[serde(default)]
    to: String,
}

/// A same-site path the launch may land on, `/` for anything else: never another host, never
/// a header break.
fn landing(to: &str) -> &str {
    let plain = to.bytes().all(|b| (0x20..0x7f).contains(&b) && b != b'\\');
    if to.starts_with('/') && !to.starts_with("//") && plain { to } else { "/" }
}

/// `GET /_launch?t=TICKET&to=/path`: spends the ticket, sets the owner cookie, lands on `to`.
async fn redeem(State(owner): State<Arc<Owner>>, Query(q): Query<Launch>) -> Response {
    if !owner.redeem(&q.t) {
        return (StatusCode::FORBIDDEN, "forbidden: launch link used or expired\n").into_response();
    }
    let cookie = format!("{}; Path=/; HttpOnly; SameSite=Strict", owner.cookie());
    let mut res = StatusCode::SEE_OTHER.into_response();
    let h = res.headers_mut();
    h.insert(
        header::LOCATION,
        HeaderValue::from_str(landing(&q.to)).unwrap_or_else(|_| HeaderValue::from_static("/")),
    );
    if let Ok(v) = HeaderValue::from_str(&cookie) {
        h.insert(header::SET_COOKIE, v);
    }
    crate::guard::no_store(res)
}

#[derive(serde::Serialize)]
struct Ticket<'a> {
    ticket: &'a str,
}

/// `POST /_launch`: a ticket for the holder of the secret in the 0600 file. A browser cannot
/// ask: it cannot read the file, and any browser marker is refused as on the add route.
async fn mint(State(owner): State<Arc<Owner>>, headers: HeaderMap) -> Response {
    if let Some(why) = crate::add::refusal(&headers, &owner.token) {
        return (StatusCode::FORBIDDEN, why).into_response();
    }
    owner.mint().map_or_else(
        || StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        |t| crate::guard::no_store(axum::Json(Ticket { ticket: &t }).into_response()),
    )
}

#[cfg(test)]
mod tests {
    use super::{Owner, landing};
    use gitterrace_actions::Token;
    use std::sync::Arc;

    fn owner() -> Owner {
        Owner::new(Arc::new(Token::generate().expect("rng")))
    }

    #[test]
    fn a_ticket_works_once() {
        let o = owner();
        let t = o.mint().expect("ticket");
        assert!(!o.redeem("nope"));
        assert!(o.redeem(&t));
        assert!(!o.redeem(&t), "spent");
    }

    #[test]
    fn unredeemed_tickets_are_bounded() {
        let o = owner();
        let first = o.mint().expect("ticket");
        for _ in 0..super::MAX_TICKETS {
            o.mint().expect("ticket");
        }
        assert!(!o.redeem(&first), "the oldest was dropped");
        assert!(o.tickets().len() <= super::MAX_TICKETS);
    }

    #[test]
    fn only_a_same_site_path_is_a_landing() {
        assert_eq!(landing("/r/x/log/HEAD?a=1"), "/r/x/log/HEAD?a=1");
        for bad in
            ["", "x", "//evil.example", "http://evil.example", "/a\\b", "/a\r\nSet-Cookie: x"]
        {
            assert_eq!(landing(bad), "/", "{bad:?}");
        }
    }

    #[test]
    fn the_cookie_proves_only_its_own_server() {
        let (a, b) = (owner(), owner());
        let mut h = axum::http::HeaderMap::new();
        h.insert("cookie", format!("x=1; {}; y=2", a.cookie()).parse().expect("v"));
        assert!(a.proven(&h));
        assert!(!b.proven(&h));
        assert!(!a.proven(&axum::http::HeaderMap::new()));
    }
}
