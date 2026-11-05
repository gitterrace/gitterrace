use super::*;

#[test]
fn server_errors_keep_their_text() {
    assert_eq!(
        ServerError::NoState.to_string(),
        "no state folder to read its secret from"
    );
    assert_eq!(ServerError::NoAnswer.to_string(), "no answer");
    assert_eq!(
        ServerError::NoAdding.to_string(),
        "it predates adding repositories or does not allow it"
    );
    assert_eq!(
        ServerError::NoRemoving.to_string(),
        "it predates removing repositories"
    );
    let e = ServerError::Status {
        status: 422,
        reason: "no".into(),
    };
    assert_eq!(e.to_string(), "HTTP 422: no");
}

#[test]
fn request_bodies_keep_their_bytes() {
    let add = serde_json::to_string(&AddBody {
        path: Path::new("/r"),
    })
    .expect("json");
    assert_eq!(add, serde_json::json!({ "path": "/r" }).to_string());
    let by_name = serde_json::to_string(&RemoveBody::Name { name: "n" }).expect("json");
    assert_eq!(by_name, serde_json::json!({ "name": "n" }).to_string());
    let by_path = serde_json::to_string(&RemoveBody::Path { path: "/r".into() }).expect("json");
    assert_eq!(by_path, serde_json::json!({ "path": "/r" }).to_string());
}
