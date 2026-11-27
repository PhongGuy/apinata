//! Imported template tags: one with an equivalent is sent converted, and one that needs a value
//! blocks sending instead of going out as text.

use crate::common::{apinata, column, run_json};
use std::fs;
use std::path::Path;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const EXPORT: &str = r#"{"_type": "export", "__export_format": 4, "resources": [
    {"_id": "wrk_1", "_type": "workspace", "parentId": null, "name": "Tags"},
    {"_id": "req_1", "_type": "request", "parentId": "wrk_1", "name": "Templated",
     "method": "GET", "url": "{{ _.host }}/now?t={% now 'millis', '' %}",
     "headers": [], "body": {}, "authentication": {}}]}"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_now_tag_is_reported_and_running_it_sends_nothing() {
    let s = MockServer::start().await;
    let into = tempfile::tempdir().unwrap();
    let out = import(EXPORT, into.path());
    let host = format!("host={}", s.uri());
    let collection = into.path().join("Tags");
    let (code, run) = run_json(&collection, &["--no-keychain", "--env-var", &host]);
    assert_eq!(code, 2, "{run}");
    assert_eq!(column(&run, "error"), ["variable `now` is not defined"]);
    assert!(s.received_requests().await.expect("recorded").is_empty());
    let report = String::from_utf8_lossy(&out.stderr);
    assert!(report.contains("`now`"), "{report}");
}

/// Import the Insomnia export `text` into `into`; the import must succeed.
fn import(text: &str, into: &Path) -> std::process::Output {
    let input = tempfile::tempdir().unwrap();
    let export = input.path().join("insomnia.json");
    fs::write(&export, text).unwrap();
    let out = apinata()
        .args(["import", "collection", "--no-keychain"])
        .arg(&export)
        .arg("--into")
        .arg(into)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    out
}

const CHAINED: &str = r#"{"_type": "export", "__export_format": 4, "resources": [
    {"_id": "wrk_1", "_type": "workspace", "parentId": null, "name": "Chain"},
    {"_id": "req_me", "_type": "request", "parentId": "wrk_1", "name": "Me", "metaSortKey": 2,
     "method": "GET", "url": "{{ _.host }}/me/{% uuid 'v4' %}",
     "headers": [{"name": "X-Sum", "value": "{% hash 'md5', 'hex', 'abc' %}"},
                 {"name": "X-Name", "value": "{% faker 'randomFirstName' %}"}],
     "body": {}, "authentication": {"type": "bearer",
        "token": "{% response 'body', 'req_login', 'b64::JC50b2tlbg==::46b', 'never', 60 %}"}},
    {"_id": "req_login", "_type": "request", "parentId": "wrk_1", "name": "Log in",
     "metaSortKey": 1, "method": "POST", "url": "{{ _.host }}/login",
     "headers": [], "body": {}, "authentication": {}}]}"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn converted_tags_run_and_a_response_tag_chains_the_requests() {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/login"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"token": "t0k"})))
        .mount(&s)
        .await;
    let into = tempfile::tempdir().unwrap();
    import(CHAINED, into.path());
    let host = format!("host={}", s.uri());
    let (code, run) = run_json(
        &into.path().join("Chain"),
        &["--no-keychain", "--env-var", &host],
    );
    assert_eq!(code, 0, "{run}");
    assert_eq!(column(&run, "error"), ["null", "null"], "{run}");
    let me = s.received_requests().await.expect("recorded").remove(1);
    let sent = |name: &str| {
        me.headers
            .get(name)
            .expect(name)
            .to_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(sent("authorization"), "Bearer t0k");
    assert_eq!(sent("x-sum"), "900150983cd24fb0d6963f7d28e17f72");
    let id = me.url.path().trim_start_matches("/me/");
    assert_eq!(id.len(), 36, "{}", me.url);
    let name = sent("x-name");
    assert!(!name.is_empty() && !name.contains('{'), "{name}");
}
