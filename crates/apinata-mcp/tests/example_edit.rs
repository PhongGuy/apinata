//! `rename_example` and `delete_example`: a request's saved examples renamed and removed by
//! name, refused for an unknown, taken or ambiguous name without touching the file.

use crate::common;

use common::{ROOT, TestServer};
use serde_json::json;
use std::fs;

const USERS: &str = "info:\n  name: r\n  type: http\n  seq: 1\n\nhttp:\n  method: GET\n  url: http://localhost/r\n\nexamples:\n  - name: ok\n    response:\n      status: 200\n  - name: missing\n    response:\n      status: 404\n";

fn disk(server: &TestServer) -> String {
    fs::read_to_string(server.root().join("r.yml")).unwrap()
}

#[tokio::test]
async fn rename_example_renames_it_in_the_file() {
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", USERS)]).await;
    let done = server
        .ok(
            "rename_example",
            json!({"path": "r.yml", "example": "ok", "name": " fine "}),
        )
        .await;
    assert_eq!(done, json!({"path": "r.yml", "name": "fine"}));
    assert_eq!(disk(&server), USERS.replace("name: ok", "name: fine"));
}

#[tokio::test]
async fn delete_example_removes_only_that_example() {
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", USERS)]).await;
    let done = server
        .ok("delete_example", json!({"path": "r.yml", "example": "ok"}))
        .await;
    assert_eq!(done, json!({"path": "r.yml", "removed": "ok"}));
    let text = disk(&server);
    assert!(
        !text.contains("name: ok") && text.contains("name: missing"),
        "{text}"
    );
}

#[tokio::test]
async fn unknown_taken_and_ambiguous_names_are_refused() {
    let both_ok = USERS.replace("name: missing", "name: ok");
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("r.yml", USERS),
        ("d.yml", &both_ok),
    ])
    .await;
    let gone = json!({"path": "r.yml", "example": "gone"});
    assert!(
        server
            .err("delete_example", gone)
            .await
            .contains("r.yml has no example named `gone`")
    );
    let clash = json!({"path": "r.yml", "example": "ok", "name": "missing"});
    assert!(
        server
            .err("rename_example", clash)
            .await
            .contains("r.yml already has an example named `missing`")
    );
    let ambiguous = "2 examples of d.yml are named `ok`, so none was changed; rename one in \
                     the app or edit the file, then try again";
    let err = server
        .err("delete_example", json!({"path": "d.yml", "example": "ok"}))
        .await;
    assert!(err.contains(ambiguous), "{err}");
    let err = server
        .err(
            "rename_example",
            json!({"path": "d.yml", "example": "ok", "name": "x"}),
        )
        .await;
    assert!(err.contains(ambiguous), "{err}");
    assert_eq!(disk(&server), USERS);
    assert_eq!(
        fs::read_to_string(server.root().join("d.yml")).unwrap(),
        both_ok
    );
}
