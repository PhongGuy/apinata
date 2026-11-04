//! Kept sends carry an optional request snapshot and origin; a store written before reads unchanged.

use crate::support;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use apinata_app::last_response::{
    Limits, ResponseStore, SendExtra, StoreOp, enqueue, install, record,
};
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::TestAppContext;
use support::*;

const SNAP: &str = r#"{"info":{"name":"snapshot-only","type":"http","seq":1},"http":{"method":"GET","url":"http://127.0.0.1:9/k"}}"#;
const KEY: &str = "untitled-17600000000000000001.draft";

fn kept(item: &str) -> RequestResult {
    let response = Response::new(200)
        .with_text_body("k")
        .with_url("http://127.0.0.1:9/k");
    RequestResult::from_parts(ip(item), "k", Outcome::Passed, Some(response), Vec::new())
}

fn json(path: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

// The one send file of the store, `<id>.0.json`.
fn send_file(dir: &Path) -> PathBuf {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.to_string_lossy().ends_with(".0.json"))
        .expect("a send file")
}

#[test]
fn a_send_keeps_its_snapshot_in_its_own_file_and_its_origin_in_the_index() {
    let dir = tempfile::tempdir().unwrap();
    let store = ResponseStore::new(dir.path().join("r"));
    let root = Path::new("/c");
    let extra = SendExtra {
        request: Some(SNAP.into()),
        origin: Some("run".into()),
    };
    let at = UNIX_EPOCH + Duration::from_secs(5);
    store
        .save_send(
            root,
            &ip(KEY),
            &kept(KEY),
            &[],
            at,
            Limits::default(),
            &extra,
        )
        .unwrap();
    assert_eq!(store.request_json(root, &ip(KEY), 0).as_deref(), Some(SNAP));
    assert_eq!(store.request_json(root, &ip(KEY), 1), None);
    let recent = store.recent(&[root.to_path_buf()], 10);
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].origin.as_deref(), Some("run"));
    let index = json(&dir.path().join("r/index.json"));
    assert_eq!(index["entries"][0]["sends"][0]["origin"], "run");
    let text = std::fs::read_to_string(dir.path().join("r/index.json")).unwrap();
    assert!(
        !text.contains("snapshot-only"),
        "the index never holds the snapshot"
    );
}

#[test]
fn a_send_without_extras_writes_neither_field() {
    let dir = tempfile::tempdir().unwrap();
    let store = ResponseStore::new(dir.path().join("r"));
    let root = Path::new("/c");
    store
        .save(
            root,
            &ip("a.yml"),
            &kept("a.yml"),
            UNIX_EPOCH + Duration::from_secs(5),
        )
        .unwrap();
    let index = json(&dir.path().join("r/index.json"));
    assert!(index["entries"][0]["sends"][0].get("origin").is_none());
    assert!(
        json(&send_file(&dir.path().join("r")))
            .get("request")
            .is_none()
    );
    assert_eq!(store.request_json(root, &ip("a.yml"), 0), None);
    assert_eq!(store.recent(&[root.to_path_buf()], 10)[0].origin, None);
}

// A version 2 store of one send of `a.yml`, as written before this plan.
fn seed_v2(dir: &Path, root: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    let index = serde_json::json!({
        "version": 2,
        "entries": [{
            "root": root.to_string_lossy(), "item": "a.yml", "id": "00000000000000aa",
            "sends": [{ "seq": 0, "saved_at_ms": 5, "status": 203, "time_ms": 42.0, "size": 3,
                        "bytes": 10, "method": "GET", "url": "http://x/ok" }]
        }]
    });
    let entry = serde_json::json!({
        "version": 1, "saved_at_ms": 5, "name": "a", "outcome": "passed",
        "status": 203, "status_text": "OK", "url": "http://x/ok", "headers": [],
        "time_ms": 42.0, "size": 3, "truncated": false, "body_dropped": false,
        "text": "old", "has_body_file": false, "assertions": [], "tests": []
    });
    std::fs::write(dir.join("index.json"), index.to_string()).unwrap();
    std::fs::write(dir.join("00000000000000aa.0.json"), entry.to_string()).unwrap();
}

#[test]
fn a_store_written_before_reads_with_no_snapshot_or_origin() {
    let dir = tempfile::tempdir().unwrap();
    let root = Path::new("/c");
    seed_v2(dir.path(), root);
    let store = ResponseStore::new(dir.path().to_path_buf());
    let recent = store.recent(&[root.to_path_buf()], 10);
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].origin, None);
    assert_eq!(store.request_json(root, &ip("a.yml"), 0), None);
    let stored = store.peek(root, &ip("a.yml")).expect("readable");
    assert_eq!(stored.result.response.unwrap().text(), "old");
}

#[gpui_kit::test]
fn the_queue_records_extras_and_peeks_a_snapshot(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = ResponseStore::new(dir.path().join("r"));
    cx.update(|cx| install(store.clone(), cx));
    let root = PathBuf::from("/c");
    let extra = SendExtra {
        request: Some(SNAP.into()),
        origin: None,
    };
    let r = root.clone();
    cx.update(|cx| record(r, ip(KEY), Arc::new(kept(KEY)), Vec::new(), extra, cx));
    wait_real(cx, |_| !store.history(&root, &ip(KEY)).is_empty());
    let (reply, answer) = async_channel::bounded(1);
    let op = StoreOp::PeekRequest {
        root: root.clone(),
        item: ip(KEY),
        seq: 0,
        reply,
    };
    cx.update(|cx| enqueue(op, cx));
    cx.run_until_parked();
    assert_eq!(answer.try_recv().ok().flatten().as_deref(), Some(SNAP));
}

// A send kept before the encoded-colon rule reads back without the password after it.
#[test]
fn a_kept_snapshot_with_an_encoded_colon_url_password_reads_back_without_it() {
    let dir = tempfile::tempdir().unwrap();
    let store = ResponseStore::new(dir.path().join("r"));
    let root = Path::new("/c");
    let snap = SNAP.replace("http://127", "http://u%3Ahunter2pw@127");
    let extra = SendExtra {
        request: Some(snap),
        origin: None,
    };
    store
        .save_send(
            root,
            &ip(KEY),
            &kept(KEY),
            &[],
            UNIX_EPOCH,
            Limits::default(),
            &extra,
        )
        .unwrap();
    let json = store.request_json(root, &ip(KEY), 0).expect("kept");
    assert!(!json.contains("hunter2pw"), "{json}");
    assert!(json.contains("http://u@127.0.0.1:9/k"), "{json}");
}
