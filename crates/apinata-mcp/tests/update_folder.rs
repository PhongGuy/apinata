//! `update_folder`: `folder.yml` and `opencollection.yml` edits, refusals, and every folder and
//! collection case of the config edit parity corpus replayed as an agent would send it.

use crate::common;

use common::{ROOT, TestServer, files_under, patch, request, research};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

#[tokio::test]
async fn writes_folder_yml_or_the_collection_file() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("plain/a.yml", &request("a", 1, "GET", "http://localhost/a")),
    ])
    .await;
    let doc = server
        .ok(
            "update_folder",
            json!({"path": "plain", "auth": {"type": "bearer", "token": "{{t}}"}}),
        )
        .await;
    assert_eq!(doc["auth"], json!({"type": "bearer", "token": "{{t}}"}));
    assert!(
        server.root().join("plain/folder.yml").is_file(),
        "created on first save"
    );
    let doc = server
        .ok("update_folder", json!({"path": "", "docs": "# Shop"}))
        .await;
    assert_eq!(doc["docs"], "# Shop");
    let root = fs::read_to_string(server.root().join("opencollection.yml")).expect("read");
    assert!(root.contains("# Shop"), "{root}");
}

#[tokio::test]
async fn an_unchanged_section_on_a_folder_without_folder_yml_creates_no_file() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("plain/a.yml", &request("a", 1, "GET", "http://localhost/a")),
    ])
    .await;
    let doc = server
        .ok(
            "update_folder",
            json!({"path": "plain", "docs": "", "headers": [], "auth": {"type": "none"}}),
        )
        .await;
    assert_eq!(doc["docs"], "");
    assert!(!server.root().join("plain/folder.yml").exists());
}

#[tokio::test]
async fn reports_a_broken_folder_file_and_a_missing_folder() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("bad/folder.yml", "info: [unclosed\n"),
    ])
    .await;
    let err = server
        .err("update_folder", json!({"path": "bad", "docs": "x"}))
        .await;
    assert!(err.contains("folder.yml"), "{err}");
    let err = server
        .err("update_folder", json!({"path": "nope", "docs": "x"}))
        .await;
    assert!(err.contains("nope"), "{err}");
    let text = fs::read_to_string(server.root().join("bad/folder.yml")).expect("read");
    assert_eq!(text, "info: [unclosed\n");
}

#[tokio::test]
async fn a_refused_edit_leaves_no_draft_for_the_next_save() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("plain/a.yml", &request("a", 1, "GET", "http://localhost/a")),
    ])
    .await;
    // Headers apply first; the unsupported auth mode is then refused.
    let err = server
        .err(
            "update_folder",
            json!({"path": "plain",
            "headers": [{"name": "X-Stale", "value": "1", "enabled": true}],
            "auth": {"type": "unsupported", "mode": "hawk"}}),
        )
        .await;
    assert!(err.contains("hawk"), "{err}");
    assert!(!server.root().join("plain/folder.yml").exists());
    server
        .ok("update_folder", json!({"path": "plain", "docs": "notes"}))
        .await;
    let text = fs::read_to_string(server.root().join("plain/folder.yml")).expect("read");
    assert!(!text.contains("X-Stale"), "{text}");
}

#[tokio::test]
async fn oauth2_is_written_and_reads_back_as_oauth2() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("plain/a.yml", &request("a", 1, "GET", "http://localhost/a")),
    ])
    .await;
    let auth = json!({"type": "oauth2", "flow": "client_credentials",
        "authorization_url": "", "access_token_url": "https://id.test/token",
        "refresh_token_url": "", "callback_url": "", "client_id": "app",
        "client_secret": "{{cs}}", "client_auth": "body", "username": "", "password": "",
        "scope": "read", "state": "", "pkce": false, "token_id": "main",
        "token_placement": {"type": "header", "prefix": "Bearer"},
        "token_source": "access_token", "auto_fetch": true, "auto_refresh": false,
        "params": [{"stage": "accessTokenRequest", "name": "aud", "value": "api",
            "placement": "body"}]});
    let doc = server
        .ok("update_folder", json!({"path": "plain", "auth": auth}))
        .await;
    assert_eq!(doc["auth"], auth);
    let text = fs::read_to_string(server.root().join("plain/folder.yml")).expect("read");
    assert!(
        text.contains("    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: https://id.test/token\n"),
        "{text}"
    );
    let read = server.ok("read_folder", json!({"path": "plain"})).await;
    assert_eq!(read["auth"], auth);
}

#[tokio::test]
async fn is_annotated_as_a_closed_world_non_destructive_write() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let tools = server.tools().await;
    let tool = tools
        .iter()
        .find(|t| t.name == "update_folder")
        .expect("listed");
    let hints = tool.annotations.clone().expect("annotated");
    assert_eq!(hints.read_only_hint, None);
    assert_eq!(hints.destructive_hint, Some(false));
    assert_eq!(hints.open_world_hint, Some(false));
}

#[tokio::test]
async fn every_folder_and_collection_case_matches_bruno() {
    let corpus = research("2026-10-09-config-edit-parity");
    let cases: Value =
        serde_json::from_str(&fs::read_to_string(corpus.join("cases.json")).expect("cases"))
            .expect("json");
    // Names and seqs are not ConfigDoc sections; environments have their own tools.
    let shared = [
        "folder-docs",
        "folder-lists-",
        "collection-docs",
        "collection-lists-",
    ];
    let mut failures = Vec::new();
    let mut replayed = 0;
    for case in cases.as_array().expect("array") {
        let id = case["id"].as_str().expect("id");
        if !shared.iter().any(|p| id.starts_with(p)) {
            continue;
        }
        let input = research("2026-10-09-edit-parity/normalized")
            .join(case["collection"].as_str().expect("collection"));
        let file = case["files"][0].as_str().expect("one file");
        let expected =
            fs::read_to_string(corpus.join("expected").join(id).join(file)).expect("expected");
        let folder = Path::new(file)
            .parent()
            .expect("parent")
            .to_str()
            .expect("utf-8");
        // A second server serves the collection with Bruno's output in place: the target.
        let files = files_under(&input);
        let mut target_files: Vec<(String, String)> = files
            .iter()
            .filter(|(rel, _)| rel != file)
            .cloned()
            .collect();
        target_files.push((file.to_owned(), expected.clone()));
        let server = TestServer::start_owned(&files).await;
        let target_server = TestServer::start_owned(&target_files).await;
        let current = server.ok("read_folder", json!({"path": folder})).await;
        let target = target_server
            .ok("read_folder", json!({"path": folder}))
            .await;
        let result = server
            .call("update_folder", patch(folder, &current, &target))
            .await;
        let actual = fs::read_to_string(server.root().join(file)).expect("read back");
        if result.is_error == Some(true) || actual != expected {
            failures.push(format!(
                "{id}: {}\n  expected {expected:?}\n  actual   {actual:?}",
                common::text(&result)
            ));
        }
        replayed += 1;
    }
    assert_eq!(replayed, 9, "folder and collection cases replayed");
    assert!(
        failures.is_empty(),
        "{} differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[tokio::test]
async fn every_section_sent_back_as_read_leaves_plain_text_docs_byte_identical() {
    // Plain-text docs read back the same, but core's setter would rewrite them as a mapping.
    let root = format!("{ROOT}docs: collection notes\n");
    let folder = "info:\n  name: plain\nrequest:\n  headers:\n    - name: X-A\n      value: \"1\"\n\
                  docs: folder notes\n";
    let server =
        TestServer::start(&[("opencollection.yml", &root), ("plain/folder.yml", folder)]).await;
    for (path, file, text) in [
        ("", "opencollection.yml", &root),
        ("plain", "plain/folder.yml", &folder.to_owned()),
    ] {
        let mut echo = server.ok("read_folder", json!({"path": path})).await;
        assert!(
            echo["docs"].as_str().expect("docs").ends_with("notes"),
            "{echo}"
        );
        echo["path"] = json!(path);
        server.ok("update_folder", echo).await;
        let after = fs::read_to_string(server.root().join(file)).expect("read");
        assert_eq!(after, *text, "{file}");
    }
}

#[tokio::test]
async fn writes_digest_and_awsv4_auth() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("plain/a.yml", &request("a", 1, "GET", "http://localhost/a")),
    ])
    .await;
    let doc = server
        .ok(
            "update_folder",
            json!({"path": "plain", "auth": {"type": "digest", "username": "u", "password": "{{pw}}"}}),
        )
        .await;
    assert_eq!(
        doc["auth"],
        json!({"type": "digest", "username": "u", "password": "{{pw}}"})
    );
    let aws = json!({"type": "awsv4", "access_key_id": "", "secret_access_key": "", "session_token": "", "service": "execute-api", "region": "eu-west-1", "profile_name": "dev"});
    let doc = server
        .ok("update_folder", json!({"path": "", "auth": aws}))
        .await;
    assert_eq!(doc["auth"], aws);
    let root = fs::read_to_string(server.root().join("opencollection.yml")).expect("read");
    assert!(
        root.contains("type: awsv4") && root.contains("profileName: dev"),
        "{root}"
    );
}

#[tokio::test]
async fn writes_and_reads_oauth1_auth() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let auth = json!({"type": "oauth1", "consumer_key": "ck", "consumer_secret": "{{cs}}",
        "access_token": null, "access_token_secret": null, "callback_url": null, "verifier": null,
        "signature_method": "RSA-SHA256", "private_key": {"type": "file", "value": "keys/p.pem"},
        "timestamp": null, "nonce": null, "version": "1.0", "realm": "r", "placement": "query",
        "include_body_hash": true});
    let doc = server
        .ok("update_folder", json!({"path": "", "auth": auth}))
        .await;
    assert_eq!(doc["auth"], auth);
    let root = fs::read_to_string(server.root().join("opencollection.yml")).expect("read");
    assert!(root.contains("  auth:\n    type: oauth1\n    consumerKey: ck\n    consumerSecret: \"{{cs}}\"\n    signatureMethod: RSA-SHA256\n    privateKey:\n      type: file\n      value: keys/p.pem\n    version: \"1.0\"\n    realm: r\n    placement: query\n    includeBodyHash: true\n"), "{root}");
}

#[tokio::test]
async fn writes_and_reads_wsse_and_edgegrid_auth() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("plain/a.yml", &request("a", 1, "GET", "http://localhost/a")),
    ])
    .await;
    let wsse = json!({"type": "wsse", "username": "u", "password": "{{pw}}"});
    let doc = server
        .ok("update_folder", json!({"path": "plain", "auth": wsse}))
        .await;
    assert_eq!(doc["auth"], wsse);
    let folder = fs::read_to_string(server.root().join("plain/folder.yml")).expect("read");
    assert!(
        folder.contains("  auth:\n    type: wsse\n    username: u\n    password: \"{{pw}}\"\n"),
        "{folder}"
    );
    let eg = json!({"type": "edgegrid", "access_token": "at", "client_token": "ct", "client_secret": "{{cs}}",
        "nonce": "", "timestamp": "", "base_url": "", "headers_to_sign": "", "max_body_size": 4096});
    let doc = server
        .ok("update_folder", json!({"path": "", "auth": eg}))
        .await;
    assert_eq!(doc["auth"]["type"], "edgegrid");
    assert_eq!(doc["auth"]["max_body_size"].as_f64(), Some(4096.0));
    let root = fs::read_to_string(server.root().join("opencollection.yml")).expect("read");
    assert!(root.contains("  auth:\n    type: akamai-edgegrid\n    accessToken: at\n    clientToken: ct\n    clientSecret: \"{{cs}}\"\n    nonce: \"\"\n    timestamp: \"\"\n    baseURL: \"\"\n    headersToSign: \"\"\n    maxBodySize: 4096\n"), "{root}");
}

#[tokio::test]
async fn a_non_finite_edgegrid_max_body_size_echoed_back_leaves_the_file_alone() {
    for value in [".inf", "-.inf", ".nan"] {
        let folder = format!(
            "info:\n  name: plain\nrequest:\n  auth:\n    type: akamai-edgegrid\n    \
             accessToken: at\n    clientToken: ct\n    clientSecret: cs\n    maxBodySize: {value}\n"
        );
        let server =
            TestServer::start(&[("opencollection.yml", ROOT), ("plain/folder.yml", &folder)]).await;
        let mut echo = server.ok("read_folder", json!({"path": "plain"})).await;
        echo["path"] = json!("plain");
        server.ok("update_folder", echo).await;
        let after = fs::read_to_string(server.root().join("plain/folder.yml")).expect("read");
        assert_eq!(after, folder, "{value}");
    }
}
