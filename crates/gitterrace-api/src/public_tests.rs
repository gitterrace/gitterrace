use axum::http::StatusCode;

use crate::ENDPOINTS;
use crate::tests::{fixture, get};

/// Class guard: an endpoint whose summary says it is refused on a public server is refused by a
/// committed-only handle, whatever it composes. A new one, or a refactor that skips the
/// helpers, fails here, and `work_state` is covered because its summary says so.
#[tokio::test]
async fn every_endpoint_documented_as_public_refused_answers_403_on_a_public_handle() {
    let (d, s) = fixture();
    std::fs::write(d.path().join("a.txt"), "secret\n").expect("write");
    let s = s.committed_only();
    let mut checked = Vec::new();
    for e in ENDPOINTS.iter().filter(|e| e.summary.contains("Refused on a public server")) {
        let (st, v) = get(&s, e.path).await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{}: {v}", e.path);
        assert!(!v.to_string().contains("secret"), "{}", e.path);
        checked.push(e.path);
    }
    assert!(checked.contains(&"/work_state"), "{checked:?}");
    assert!(checked.contains(&"/worktree"), "{checked:?}");
}
