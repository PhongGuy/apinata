//! An imported template tag that needs a value blocks sending instead of going out as text.

use crate::common::{apinata, column, run_json};
use std::fs;
use wiremock::MockServer;

const EXPORT: &str = r#"{"_type": "export", "__export_format": 4, "resources": [
    {"_id": "wrk_1", "_type": "workspace", "parentId": null, "name": "Tags"},
    {"_id": "req_1", "_type": "request", "parentId": "wrk_1", "name": "Templated",
     "method": "GET", "url": "{{ _.host }}/now?t={% now 'iso-8601', '' %}",
     "headers": [], "body": {}, "authentication": {}}]}"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_now_tag_is_reported_and_running_it_sends_nothing() {
    let s = MockServer::start().await;
    let input = tempfile::tempdir().unwrap();
    let into = tempfile::tempdir().unwrap();
    let export = input.path().join("insomnia.json");
    fs::write(&export, EXPORT).unwrap();
    let out = apinata()
        .args(["import", "collection", "--no-keychain"])
        .arg(&export)
        .arg("--into")
        .arg(into.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let host = format!("host={}", s.uri());
    let collection = into.path().join("Tags");
    let (code, run) = run_json(&collection, &["--no-keychain", "--env-var", &host]);
    assert_eq!(code, 2, "{run}");
    assert_eq!(column(&run, "error"), ["variable `now` is not defined"]);
    assert!(s.received_requests().await.expect("recorded").is_empty());
    let report = String::from_utf8_lossy(&out.stderr);
    assert!(report.contains("`now`"), "{report}");
}
