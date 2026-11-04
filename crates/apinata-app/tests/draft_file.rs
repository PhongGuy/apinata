use crate::support;

use apinata_app::draft_file::{self, BuildError, DraftFile, MAX_DRAFT_BYTES, MAX_DRAFTS};
use apinata_app::persist::save_json;
use apinata_core::{DraftSnapshot, ItemPath};
use std::path::Path;
use support::ip;

const REQUEST: &str = r#"{"info":{"name":"Typed","type":"http"},"http":{"method":"GET","url":"http://127.0.0.1:9/sekrit-value-1?q=abc","headers":[{"name":"X-Key","value":"sekrit-value-1"}]}}"#;

fn key(ms: u64) -> ItemPath {
    ip(&format!("untitled-{ms}000042.draft"))
}

fn built(ms: u64) -> DraftFile {
    let snap = DraftSnapshot::from_json(REQUEST).unwrap();
    draft_file::build(
        &key(ms),
        Path::new("/c"),
        &ItemPath::root(),
        &snap,
        false,
        &[],
    )
    .unwrap()
}

fn put(dir: &Path, file: &DraftFile) {
    save_json(
        &dir.join(draft_file::file_name(&file.item_key().unwrap())),
        file,
    )
    .unwrap();
}

#[test]
fn build_scrubs_every_string_and_records_where_the_draft_lives() {
    let snap = DraftSnapshot::from_json(REQUEST).unwrap();
    let secrets = ["sekrit-value-1".to_owned(), "abc".to_owned()];
    let file = draft_file::build(
        &key(1_760_600_000_000),
        Path::new("/c"),
        &ip("f"),
        &snap,
        true,
        &secrets,
    )
    .unwrap();
    assert!(!file.request.contains("sekrit-value-1"), "{}", file.request);
    assert!(file.request.contains("[redacted]"));
    assert!(
        file.request.contains("q=abc"),
        "a value under 4 characters is not scrubbed"
    );
    assert!(DraftSnapshot::from_json(&file.request).is_ok());
    assert_eq!(
        (
            file.version,
            file.key.as_str(),
            file.root.as_path(),
            file.parent.as_str()
        ),
        (
            1,
            "untitled-1760600000000000042.draft",
            Path::new("/c"),
            "f"
        )
    );
    assert_eq!(
        (file.created_ms, file.credentials_blanked),
        (1_760_600_000_000, true)
    );
    assert_eq!(
        draft_file::file_name(&key(1_760_600_000_000)),
        "untitled-1760600000000000042.json"
    );
}

#[test]
fn a_draft_over_one_mib_is_refused() {
    let big = format!(
        r#"{{"info":{{"name":"Big","type":"http"}},"http":{{"method":"GET","url":"http://127.0.0.1:9/"}},"docs":"{}"}}"#,
        "x".repeat(MAX_DRAFT_BYTES)
    );
    let snap = DraftSnapshot::from_json(&big).unwrap();
    let out = draft_file::build(
        &key(1),
        Path::new("/c"),
        &ItemPath::root(),
        &snap,
        false,
        &[],
    );
    assert_eq!(out, Err(BuildError::TooLarge));
}

#[test]
fn bad_files_are_skipped_and_left_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let good = built(1_760_600_000_005);
    put(dir.path(), &good);
    let mut v2 = built(1_760_600_000_006);
    v2.version = 2;
    put(dir.path(), &v2);
    let mut unparseable = built(1_760_600_000_007);
    unparseable.request = "{}".into();
    put(dir.path(), &unparseable);
    let mut misnamed = built(1_760_600_000_008);
    misnamed.key = key(1_760_600_000_009).as_str().to_owned();
    save_json(
        &dir.path()
            .join(draft_file::file_name(&key(1_760_600_000_008))),
        &misnamed,
    )
    .unwrap();
    std::fs::write(dir.path().join("corrupt.json"), "not json").unwrap();
    std::fs::write(
        dir.path().join("huge.json"),
        " ".repeat(MAX_DRAFT_BYTES + 1),
    )
    .unwrap();
    std::fs::write(dir.path().join(".lock"), "").unwrap();
    let loaded = draft_file::load_dir(dir.path());
    assert_eq!(loaded.drafts, [good]);
    assert_eq!(loaded.skipped.len(), 5, "{:?}", loaded.skipped);
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        7,
        "nothing removed"
    );
}

#[test]
fn the_newest_fifty_open_oldest_first_and_the_rest_stay() {
    let dir = tempfile::tempdir().unwrap();
    let base = 1_760_600_000_000;
    for i in 1..=52 {
        put(dir.path(), &built(base + i));
    }
    let loaded = draft_file::load_dir(dir.path());
    assert_eq!(loaded.drafts.len(), MAX_DRAFTS);
    assert_eq!(loaded.drafts[0].created_ms, base + 3);
    assert_eq!(loaded.drafts[MAX_DRAFTS - 1].created_ms, base + 52);
    assert_eq!(loaded.unopened, 2);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 52);
}

#[test]
fn a_missing_directory_loads_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let loaded = draft_file::load_dir(&dir.path().join("absent"));
    assert!(loaded.drafts.is_empty() && loaded.skipped.is_empty());
}

#[test]
fn the_lock_is_exclusive_until_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let drafts = dir.path().join("drafts");
    let first = draft_file::lock(&drafts).expect("free");
    assert!(draft_file::lock(&drafts).is_none(), "held by the first");
    drop(first);
    assert!(draft_file::lock(&drafts).is_some());
}

#[test]
fn a_lock_let_go_a_moment_later_is_taken() {
    // A forked child holds a copy of the lock for a moment after its holder drops it.
    let dir = tempfile::tempdir().unwrap();
    let drafts = dir.path().join("drafts");
    let held = draft_file::lock(&drafts).expect("free");
    let holder = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(50));
        drop(held);
    });
    assert!(
        draft_file::lock(&drafts).is_some(),
        "taken once the holder lets go"
    );
    holder.join().unwrap();
}

#[test]
fn build_keeps_method_and_type_tokens_parseable_when_they_are_secrets() {
    let snap = DraftSnapshot::from_json(REQUEST).unwrap();
    let secrets = ["http".to_owned(), "GET1".to_owned(), "GET".to_owned()];
    let file = draft_file::build(
        &key(1_760_600_000_001),
        Path::new("/c"),
        &ItemPath::root(),
        &snap,
        false,
        &secrets,
    )
    .unwrap();
    assert!(DraftSnapshot::from_json(&file.request).is_ok());
    assert!(
        file.request.contains(r#""type":"http""#),
        "{}",
        file.request
    );
    assert!(
        file.request.contains(r#""method":"GET""#),
        "{}",
        file.request
    );
}

// A snapshot whose mapping holds a key JSON cannot carry: `to_json` yields "null" for it.
fn unserializable() -> DraftSnapshot {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("w.yml");
    std::fs::write(
        &path,
        "info:\n  name: W\n  type: http\nhttp:\n  method: GET\n  url: http://127.0.0.1:9/w\n? [a, b]\n: c\n",
    )
    .unwrap();
    let snap = apinata_core::RequestFile::load(&path).unwrap().snapshot();
    assert_eq!(snap.to_json(), "null", "the fixture must not serialize");
    snap
}

#[test]
fn a_snapshot_json_cannot_hold_is_refused_as_unserializable_not_too_large() {
    let out = draft_file::build(
        &key(2),
        Path::new("/c"),
        &ItemPath::root(),
        &unserializable(),
        false,
        &[],
    );
    assert!(matches!(out, Err(BuildError::Unserializable(_))), "{out:?}");
}

#[test]
fn a_null_request_file_is_reported_and_not_counted_as_a_draft() {
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), &built(1_760_600_000_005));
    let mut null = built(1_760_600_000_006);
    null.request = "null".into();
    put(dir.path(), &null);
    let loaded = draft_file::load_dir(dir.path());
    assert_eq!(loaded.drafts.len(), 1);
    assert_eq!(loaded.skipped.len(), 1, "{:?}", loaded.skipped);
    assert!(loaded.skipped[0].contains("unreadable request"));
    assert!(
        dir.path()
            .join(draft_file::file_name(&null.item_key().unwrap()))
            .exists(),
        "reported, never deleted"
    );
}
