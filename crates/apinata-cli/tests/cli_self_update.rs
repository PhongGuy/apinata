//! `apinata self-update` against a local server; installs act on a temporary copy only.
#![cfg(unix)]
// The helpers serve only the debug-gated tests below.
#![cfg_attr(not(debug_assertions), allow(dead_code, unused_imports))]

use crate::common;

use apinata_core::update::{Product, target_id};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use common::apinata;
use predicates::str::contains;
use sha2::Digest as _;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use wiremock::matchers::{any, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const PAGE: &str = "https://example.com/download";
const NEW: &[u8] = b"new binary";

// A key generated for this test run only.
struct Key(minisign::KeyPair);

impl Key {
    fn generate() -> Key {
        Key(minisign::KeyPair::generate_unencrypted_keypair().unwrap())
    }
    fn line(&self) -> String {
        STANDARD.encode(self.0.pk.to_box().unwrap().to_string())
    }
    fn sign(&self, data: &[u8]) -> String {
        let sig = minisign::sign(
            Some(&self.0.pk),
            &self.0.sk,
            std::io::Cursor::new(data),
            Some("t"),
            None,
        );
        STANDARD.encode(sig.unwrap().to_string())
    }
}

async fn release(s: &MockServer, signer: &Key, version: &str) {
    let sha: String = sha2::Sha256::digest(NEW)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let manifest = format!(
        r#"{{"version":"{version}","notes":"Faster runs.","targets":{{"{}":{{"url":"{}/artifact","format":"binary","size":{},"sha256":"{sha}"}}}}}}"#,
        target_id(Product::Cli),
        s.uri(),
        NEW.len()
    );
    let ok = |body: Vec<u8>| ResponseTemplate::new(200).set_body_bytes(body);
    Mock::given(method("GET"))
        .and(path("/latest.json"))
        .respond_with(ok(manifest.clone().into_bytes()))
        .mount(s)
        .await;
    Mock::given(method("GET"))
        .and(path("/latest.json.sig"))
        .respond_with(ok(signer.sign(manifest.as_bytes()).into_bytes()))
        .mount(s)
        .await;
    Mock::given(method("GET"))
        .and(path("/artifact"))
        .respond_with(ok(NEW.to_vec()))
        .mount(s)
        .await;
}

// A temporary install: dir/bin/apinata holding "old", executable.
fn install() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("bin/apinata");
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::write(&exe, b"old").unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    (dir, exe)
}

fn self_update(s: &MockServer, trusted: &Key, exe: &Path) -> assert_cmd::Command {
    let mut c = apinata();
    c.arg("self-update")
        .env(
            "APINATA_UPDATE_MANIFEST_URL",
            format!("{}/latest.json", s.uri()),
        )
        .env("APINATA_DOWNLOAD_PAGE_URL", PAGE)
        .env("APINATA_UPDATE_TEST_KEYS", trusted.line())
        .env("APINATA_UPDATE_TEST_EXE", exe)
        .env_remove("APPIMAGE");
    c
}

async fn artifact_fetches(s: &MockServer) -> usize {
    s.received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/artifact")
        .count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unconfigured_builds_say_so_and_make_no_request() {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&s)
        .await;
    apinata()
        .arg("self-update")
        .env(
            "APINATA_UPDATE_MANIFEST_URL",
            format!("{}/latest.json", s.uri()),
        )
        .env_remove("APINATA_DOWNLOAD_PAGE_URL")
        .env_remove("APINATA_UPDATE_TEST_KEYS")
        .assert()
        .code(3)
        .stderr(contains("Update checks are not configured for this build"));
    s.verify().await;
}

// The update test seams exist only in debug builds.
#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn check_reports_an_available_update_with_exit_2() {
    let (s, k, (_d, exe)) = (MockServer::start().await, Key::generate(), install());
    release(&s, &k, "99.0.0").await;
    self_update(&s, &k, &exe)
        .arg("--check")
        .assert()
        .code(2)
        .stdout(contains("Apinata 99.0.0 is available"))
        .stdout(contains("Faster runs."));
    assert_eq!(std::fs::read(&exe).unwrap(), b"old");
    assert_eq!(artifact_fetches(&s).await, 0);
}

// The update test seams exist only in debug builds.
#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_running_version_is_up_to_date() {
    let (s, k, (_d, exe)) = (MockServer::start().await, Key::generate(), install());
    release(&s, &k, env!("CARGO_PKG_VERSION")).await;
    self_update(&s, &k, &exe)
        .assert()
        .code(0)
        .stdout(contains("is up to date"));
}

// The update test seams exist only in debug builds.
#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn yes_installs_into_the_standalone_copy() {
    let (s, k, (_d, exe)) = (MockServer::start().await, Key::generate(), install());
    release(&s, &k, "99.0.0").await;
    self_update(&s, &k, &exe)
        .arg("--yes")
        .assert()
        .code(0)
        .stdout(contains("Installed Apinata 99.0.0."));
    assert_eq!(std::fs::read(&exe).unwrap(), NEW);
    assert_eq!(
        std::fs::metadata(&exe).unwrap().permissions().mode() & 0o777,
        0o755
    );
}

// The update test seams exist only in debug builds.
#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_declined_prompt_installs_nothing() {
    let (s, k, (_d, exe)) = (MockServer::start().await, Key::generate(), install());
    release(&s, &k, "99.0.0").await;
    self_update(&s, &k, &exe)
        .write_stdin("n\n")
        .assert()
        .code(0)
        .stdout(contains("[y/N]"))
        .stdout(contains("Nothing was installed."));
    assert_eq!(std::fs::read(&exe).unwrap(), b"old");
    assert_eq!(artifact_fetches(&s).await, 0);
}

// The update test seams exist only in debug builds.
#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_copy_shipped_with_the_app_is_never_replaced() {
    let (s, k, (_d, exe)) = (MockServer::start().await, Key::generate(), install());
    std::fs::write(exe.with_file_name("apinata-app"), b"app").unwrap();
    release(&s, &k, "99.0.0").await;
    self_update(&s, &k, &exe)
        .arg("--yes")
        .assert()
        .code(0)
        .stdout(contains("update the app instead"))
        .stdout(contains(PAGE));
    assert_eq!(std::fs::read(&exe).unwrap(), b"old");
    assert_eq!(artifact_fetches(&s).await, 0);
}

// The update test seams exist only in debug builds.
#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn check_on_a_copy_shipped_with_the_app_says_how_to_update() {
    let (s, k, (_d, exe)) = (MockServer::start().await, Key::generate(), install());
    std::fs::write(exe.with_file_name("apinata-app"), b"app").unwrap();
    release(&s, &k, "99.0.0").await;
    self_update(&s, &k, &exe)
        .arg("--check")
        .assert()
        .code(0)
        .stdout(contains("Apinata 99.0.0 is available"))
        .stdout(contains("update the app instead"))
        .stdout(contains(PAGE));
    assert_eq!(artifact_fetches(&s).await, 0);
}

// The update test seams exist only in debug builds.
#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_release_signed_by_another_key_is_an_error() {
    let (s, signer, (_d, exe)) = (MockServer::start().await, Key::generate(), install());
    release(&s, &signer, "99.0.0").await;
    self_update(&s, &Key::generate(), &exe)
        .arg("--yes")
        .assert()
        .code(3)
        .stderr(contains("unknown key"));
    assert_eq!(std::fs::read(&exe).unwrap(), b"old");
}

/// Publishes 99.0.0 with a notes asset at `/v99/notes.json` listing `notes`; `served` is what
/// that URL answers.
async fn release_with_notes(s: &MockServer, signer: &Key, notes: &[u8], served: ResponseTemplate) {
    let hex = |data: &[u8]| -> String {
        sha2::Sha256::digest(data)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    };
    let manifest = serde_json::json!({
        "version": "99.0.0",
        "notes": "Faster runs.",
        "notes_asset": {"url": format!("{}/v99/notes.json", s.uri()),
                        "size": notes.len(), "sha256": hex(notes)},
        "targets": {target_id(Product::Cli): {"url": format!("{}/artifact", s.uri()),
                    "format": "binary", "size": NEW.len(), "sha256": hex(NEW)}}
    })
    .to_string();
    let ok = |body: Vec<u8>| ResponseTemplate::new(200).set_body_bytes(body);
    Mock::given(method("GET"))
        .and(path("/latest.json"))
        .respond_with(ok(manifest.clone().into_bytes()))
        .mount(s)
        .await;
    Mock::given(method("GET"))
        .and(path("/latest.json.sig"))
        .respond_with(ok(signer.sign(manifest.as_bytes()).into_bytes()))
        .mount(s)
        .await;
    Mock::given(method("GET"))
        .and(path("/v99/notes.json"))
        .respond_with(served)
        .mount(s)
        .await;
}

fn notes_json() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"schema": 1, "releases": [
        {"version": "98.0.0", "previous": "97.0.0", "date": "2026-11-01",
         "groups": [{"kind": "fixes", "title": "Fixes", "items": [{"text": "Fixed a crash"}]}]},
        {"version": "99.0.0", "previous": "98.0.0", "date": "2026-12-01",
         "intro": "A faster release.",
         "groups": [
            {"kind": "features", "title": "New features",
             "items": [{"text": "Search the history", "pr": 1364}, {"text": "...and 2 more"}]},
            {"kind": "performance", "title": "Performance", "items": [{"text": "Faster runs"}]}]}
    ]}))
    .unwrap()
}

fn printed(out: &std::process::Output) -> String {
    String::from_utf8(out.stdout.clone())
        .unwrap()
        .replace(env!("CARGO_PKG_VERSION"), "<current>")
}

// The update test seams exist only in debug builds.
#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn check_lists_the_notes_of_every_newer_release() {
    let (s, k, (_d, exe)) = (MockServer::start().await, Key::generate(), install());
    let notes = notes_json();
    release_with_notes(
        &s,
        &k,
        &notes,
        ResponseTemplate::new(200).set_body_bytes(notes.clone()),
    )
    .await;
    let out = self_update(&s, &k, &exe).arg("--check").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    insta::assert_snapshot!(printed(&out), @r#"
    Apinata 99.0.0 is available (this is <current>).

    99.0.0 (2026-12-01)
      A faster release.
      New features
        - Search the history (#1364)
        - ...and 2 more
      Performance
        - Faster runs

    98.0.0 (2026-11-01)
      Fixes
        - Fixed a crash

    Older versions: see https://example.com/download
    "#);
}

// The update test seams exist only in debug builds.
#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn check_says_when_the_notes_could_not_be_loaded() {
    let (s, k, (_d, exe)) = (MockServer::start().await, Key::generate(), install());
    release_with_notes(&s, &k, &notes_json(), ResponseTemplate::new(404)).await;
    let out = self_update(&s, &k, &exe).arg("--check").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    insta::assert_snapshot!(printed(&out), @r#"
    Apinata 99.0.0 is available (this is <current>).
    Release notes couldn't be loaded.
      Faster runs.
    "#);
}

// The update test seams exist only in debug builds.
#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn check_prints_json_with_the_notes_or_the_reason() {
    let (s, k, (_d, exe)) = (MockServer::start().await, Key::generate(), install());
    let notes = notes_json();
    release_with_notes(
        &s,
        &k,
        &notes,
        ResponseTemplate::new(200).set_body_bytes(notes.clone()),
    )
    .await;
    let out = (self_update(&s, &k, &exe).args(["--check", "--format", "json"]))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    insta::assert_snapshot!(printed(&out), @r#"
    {
      "current": "<current>",
      "available": "99.0.0",
      "notes": [
        {
          "version": "99.0.0",
          "previous": "98.0.0",
          "date": "2026-12-01",
          "intro": "A faster release.",
          "groups": [
            {
              "kind": "features",
              "title": "New features",
              "items": [
                {
                  "text": "Search the history",
                  "pr": 1364
                },
                {
                  "text": "...and 2 more"
                }
              ]
            },
            {
              "kind": "performance",
              "title": "Performance",
              "items": [
                {
                  "text": "Faster runs"
                }
              ]
            }
          ]
        },
        {
          "version": "98.0.0",
          "previous": "97.0.0",
          "date": "2026-11-01",
          "groups": [
            {
              "kind": "fixes",
              "title": "Fixes",
              "items": [
                {
                  "text": "Fixed a crash"
                }
              ]
            }
          ]
        }
      ],
      "older_not_listed": true,
      "notes_error": null
    }
    "#);

    let (s, k) = (MockServer::start().await, Key::generate());
    release_with_notes(&s, &k, &notes, ResponseTemplate::new(404)).await;
    let out = (self_update(&s, &k, &exe).args(["--check", "--format", "json"]))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    insta::assert_snapshot!(printed(&out), @r#"
    {
      "current": "<current>",
      "available": "99.0.0",
      "notes": null,
      "older_not_listed": false,
      "notes_error": "the notes answered HTTP 404"
    }
    "#);
}

#[test]
fn format_needs_check() {
    apinata()
        .args(["self-update", "--format", "json"])
        .assert()
        .code(3);
}
