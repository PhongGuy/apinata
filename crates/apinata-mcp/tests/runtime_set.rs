//! `set_runtime_variable` and `clear_runtime_variables`: session variables an agent seeds, which
//! the next send uses, and that never echo a value or reach a file.

use crate::common;

use common::{ROOT, TestServer};
use serde_json::json;
use wiremock::matchers::{header, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn files(uri: &str) -> Vec<(String, String)> {
    let me = format!(
        "info:\n  name: me\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{uri}/me\"\n  headers:\n    - name: Authorization\n      value: \"Bearer {{{{token}}}}\"\n"
    );
    vec![
        ("opencollection.yml".to_owned(), ROOT.to_owned()),
        ("me.yml".to_owned(), me),
    ]
}

/// Every file under `dir` with its bytes, keyed by relative path.
fn snapshot(dir: &std::path::Path) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
    fn walk(
        root: &std::path::Path,
        dir: &std::path::Path,
        out: &mut std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>,
    ) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let rel = path.strip_prefix(root).unwrap().to_path_buf();
                out.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    let mut out = std::collections::BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

async fn mock() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(path("/me"))
        .and(header("authorization", "Bearer seeded-value"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    Mock::given(path("/me"))
        .respond_with(ResponseTemplate::new(401))
        .with_priority(10)
        .mount(&s)
        .await;
    s
}

#[tokio::test]
async fn a_seeded_variable_resolves_in_the_next_send_and_clears() {
    let s = mock().await;
    let server = TestServer::start_owned(&files(&s.uri())).await;
    // Unresolved before it is set: an error, never a literal `{{token}}`.
    let before = server.err("send_request", json!({"path": "me.yml"})).await;
    assert!(before.contains("token"), "{before}");

    let set = server
        .ok(
            "set_runtime_variable",
            json!({"name": "token", "value": "seeded-value"}),
        )
        .await;
    assert_eq!(set, json!({"name": "token", "replaced": false}));
    assert!(!set.to_string().contains("seeded-value"));
    let sent = server.ok("send_request", json!({"path": "me.yml"})).await;
    assert_eq!(sent["status"], 200);

    let again = server
        .ok(
            "set_runtime_variable",
            json!({"name": "token", "value": "seeded-value"}),
        )
        .await;
    assert_eq!(again["replaced"], true);

    let cleared = server
        .ok(
            "clear_runtime_variables",
            json!({"names": ["token", "other"]}),
        )
        .await;
    assert_eq!(cleared, json!({"cleared": 1}));
    assert!(!cleared.to_string().contains("seeded-value"));
    let after = server.err("send_request", json!({"path": "me.yml"})).await;
    assert!(after.contains("token"), "{after}");
}

#[tokio::test]
async fn clearing_without_names_removes_every_variable() {
    let s = mock().await;
    let server = TestServer::start_owned(&files(&s.uri())).await;
    for n in ["a", "b"] {
        server
            .ok("set_runtime_variable", json!({"name": n, "value": "v"}))
            .await;
    }
    assert_eq!(
        server.ok("clear_runtime_variables", json!({})).await,
        json!({"cleared": 2})
    );
    assert_eq!(
        server.ok("list_runtime_variables", json!({})).await,
        json!({"variables": []})
    );
}

#[tokio::test]
async fn names_a_reference_can_never_resolve_are_invalid_params() {
    let s = mock().await;
    let server = TestServer::start_owned(&files(&s.uri())).await;
    let before = snapshot(server.root());
    for bad in [
        "{{tok}}", " tok ", "tok ", " tok", "{tok}", "a b", "a}}", "", "   ",
    ] {
        let err = server
            .try_call("set_runtime_variable", json!({"name": bad, "value": "v"}))
            .await
            .expect_err(bad);
        assert!(err.contains("-32602"), "{bad:?}: {err}");
        assert!(err.contains("name"), "{bad:?}: {err}");
    }
    assert_eq!(snapshot(server.root()), before, "a refusal touched files");
    assert_eq!(
        server.ok("list_runtime_variables", json!({})).await,
        json!({"variables": []})
    );
}

#[tokio::test]
async fn plain_dotted_and_dashed_names_are_accepted() {
    let s = mock().await;
    let server = TestServer::start_owned(&files(&s.uri())).await;
    for good in ["tok", "api.key", "my-var_1"] {
        let set = server
            .ok("set_runtime_variable", json!({"name": good, "value": "v"}))
            .await;
        assert_eq!(set["name"], good);
    }
}
