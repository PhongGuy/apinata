//! `import_collection` with `update` and no input, on a served collection whose only OpenAPI
//! record is the reference app's. Links are wiremock on 127.0.0.1.

use crate::common::{TestServer, files_under};
use serde_json::json;
use std::fs;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const V1: &str = include_str!("../../apinata-core/tests/fixtures/openapi/update-v1.json");
const V2: &str = include_str!("../../apinata-core/tests/fixtures/openapi/update-v2.json");

/// The v1 import served with its `extensions` swapped for a reference record naming `source`.
async fn reference_only(source: &str, group_by: &str) -> TestServer {
    use apinata_core::MemoryStore;
    use apinata_core::import::{CollectionOptions, parse_collections, write_collection};
    let parent = tempfile::tempdir().unwrap();
    let bundles = parse_collections(None, V1.as_bytes(), &CollectionOptions::default()).unwrap();
    write_collection(
        parent.path(),
        &bundles[0].collection,
        &[],
        &MemoryStore::new(),
    )
    .unwrap();
    let mut files: Vec<(String, String)> = files_under(&parent.path().join("Shop"));
    for (rel, text) in &mut files {
        if rel != "opencollection.yml" {
            continue;
        }
        let mut kept = String::new();
        let mut skipping = false;
        for line in text.lines() {
            if line.starts_with("extensions:") {
                skipping = true;
                continue;
            }
            if skipping && (line.starts_with(' ') || line.is_empty()) {
                continue;
            }
            skipping = false;
            kept.push_str(line);
            kept.push('\n');
        }
        *text = format!(
            "{kept}extensions:\n  bruno:\n    openapi:\n      - sourceUrl: {source}\n        groupBy: {group_by}\n"
        );
    }
    TestServer::start_owned(&files).await
}

#[tokio::test]
async fn a_recorded_file_is_adopted_with_its_grouping() {
    let server = reference_only("spec.json", "path").await;
    fs::write(server.root().join("spec.json"), V2).unwrap();
    let out = server
        .ok("import_collection", json!({"update": true}))
        .await;
    assert_eq!(out["added"], json!(["users/listUsers.yml"]), "{out}");
    assert_eq!(out["differs"][0]["path"], "people/getUser.yml", "{out}");
    assert_eq!(out["pruned"], json!([]));
    let root = fs::read_to_string(server.root().join("opencollection.yml")).unwrap();
    assert!(
        root.contains("      - sourceUrl: spec.json\n        groupBy: path\n"),
        "{root}"
    );
    assert!(root.contains("  apinata:"), "{root}");
}

#[tokio::test]
async fn a_recorded_local_link_is_fetched() {
    let upstream = MockServer::start().await;
    Mock::given(path("/spec.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(V2))
        .mount(&upstream)
        .await;
    let server = reference_only(&format!("{}/spec.json", upstream.uri()), "tags").await;
    let out = server
        .ok("import_collection", json!({"update": true}))
        .await;
    assert_eq!(out["added"], json!(["people/listUsers.yml"]), "{out}");
}

#[tokio::test]
async fn a_recorded_non_local_link_still_needs_approval() {
    let server = reference_only("http://specs.example.invalid/api.json", "tags").await;
    let before = files_under(server.root());
    let message = server
        .err("import_collection", json!({"update": true}))
        .await;
    assert!(message.contains("Approval requested"), "{message}");
    assert_eq!(server.inbox().pending().unwrap().len(), 1);
    assert_eq!(files_under(server.root()), before);
}

#[tokio::test]
async fn a_recorded_file_outside_the_collection_is_refused() {
    let server = reference_only("../spec.json", "tags").await;
    let message = server
        .err("import_collection", json!({"update": true}))
        .await;
    assert!(message.contains("recorded spec"), "{message}");
    assert!(message.contains("outside"), "{message}");
}

#[tokio::test]
async fn a_missing_recorded_file_is_a_clear_error() {
    let server = reference_only("gone.json", "tags").await;
    let message = server
        .err("import_collection", json!({"update": true}))
        .await;
    assert!(message.contains("recorded spec"), "{message}");
    assert!(message.contains("gone.json"), "{message}");
}

#[tokio::test]
async fn an_unreachable_recorded_link_is_a_clear_error() {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let server = reference_only(&format!("http://127.0.0.1:{port}/spec.json"), "tags").await;
    let message = server
        .err("import_collection", json!({"update": true}))
        .await;
    assert!(message.contains("recorded spec"), "{message}");
    assert!(message.contains("cannot fetch"), "{message}");
}

#[tokio::test]
async fn an_update_with_no_input_and_no_recorded_source_is_refused() {
    let server = TestServer::start(&[("opencollection.yml", crate::common::ROOT)]).await;
    let message = server
        .err("import_collection", json!({"update": true}))
        .await;
    assert!(message.contains("`url` and `text`"), "{message}");
}

#[tokio::test]
async fn a_folder_update_without_input_is_refused_and_changes_nothing() {
    let server = reference_only("spec.json", "tags").await;
    fs::write(server.root().join("spec.json"), V2).unwrap();
    let before = files_under(server.root());
    let message = server
        .err(
            "import_collection",
            json!({"update": true, "into": "people"}),
        )
        .await;
    assert!(
        message.contains("give a spec file or link to update a folder"),
        "{message}"
    );
    assert_eq!(files_under(server.root()), before);
}
