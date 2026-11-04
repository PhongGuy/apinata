//! A version 1 store (plan 5r) migrates to version 2 without losing its kept response.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use apinata_app::last_response::ResponseStore;
use apinata_core::{ItemPath, Outcome, RequestResult, Response};

fn ip(s: &str) -> ItemPath {
    ItemPath::new(s).unwrap()
}

fn at(ms: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(ms)
}

fn sent(item: &str, status: u16, body: &str) -> RequestResult {
    let response = Response::new(status)
        .with_text_body(body)
        .with_response_time(Duration::from_millis(7));
    RequestResult::from_parts(ip(item), "A", Outcome::Passed, Some(response), Vec::new())
}

fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|d| {
            d.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn statuses(store: &ResponseStore, root: &Path, item: &str) -> Vec<u16> {
    store
        .history(root, &ip(item))
        .iter()
        .map(|s| s.status)
        .collect()
}

// A version 1 entry file as 5r wrote it.
fn v1_entry(has_body_file: bool) -> String {
    serde_json::json!({
        "version": 1, "saved_at_ms": 5, "name": "A", "outcome": "passed",
        "status": 200, "status_text": "OK", "url": "http://x/ok", "headers": [],
        "time_ms": 42.0, "size": 5, "truncated": false, "body_dropped": false,
        "text": "hello", "has_body_file": has_body_file, "assertions": []
    })
    .to_string()
}

fn v1_store(dir: &Path, has_body_file: bool) {
    std::fs::create_dir_all(dir).unwrap();
    let index = r#"{"version":1,"entries":[{"root":"/c","item":"a.yml","id":"00000000000000aa"}]}"#;
    std::fs::write(dir.join("index.json"), index).unwrap();
    std::fs::write(dir.join("00000000000000aa.json"), v1_entry(has_body_file)).unwrap();
    if has_body_file {
        std::fs::write(dir.join("00000000000000aa.body"), [0xffu8, 0xfe]).unwrap();
    }
}

#[test]
fn a_version_1_store_migrates_without_losing_its_response() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("r");
    v1_store(&dir, false);
    let store = ResponseStore::new(dir.clone());
    let root = Path::new("/c");
    let got = store
        .load(root, &ip("a.yml"))
        .expect("the v1 response survives");
    assert_eq!(got.saved_at, at(5));
    let response = got.result.response.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.text(), "hello");
    let h = store.history(root, &ip("a.yml"));
    assert_eq!(h.len(), 1);
    assert_eq!((h[0].seq, h[0].status, h[0].size), (0, 200, 5));
    assert_eq!(h[0].time, Duration::from_millis(42));
    assert_eq!(read_json(&dir.join("index.json"))["version"], 2);
    assert_eq!(
        listing(&dir),
        [
            "00000000000000aa.0.json".to_owned(),
            "index.json".to_owned()
        ]
    );
    store
        .save(root, &ip("a.yml"), &sent("a.yml", 201, "new"), at(6))
        .unwrap();
    assert_eq!(statuses(&store, root, "a.yml"), [201, 200]);
}

#[test]
fn a_version_1_body_file_moves_with_its_entry() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("r");
    v1_store(&dir, true);
    let store = ResponseStore::new(dir.clone());
    let got = store.load(Path::new("/c"), &ip("a.yml")).expect("kept");
    assert_eq!(got.result.response.unwrap().body(), [0xffu8, 0xfe]);
    assert!(listing(&dir).contains(&"00000000000000aa.0.body".to_owned()));
}

#[test]
fn a_half_migrated_version_1_store_still_loads() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("r");
    v1_store(&dir, false);
    // A crash after the rename but before the new index was written.
    std::fs::rename(
        dir.join("00000000000000aa.json"),
        dir.join("00000000000000aa.0.json"),
    )
    .unwrap();
    let store = ResponseStore::new(dir.clone());
    let got = store.load(Path::new("/c"), &ip("a.yml")).expect("kept");
    assert_eq!(got.result.response.unwrap().text(), "hello");
    assert_eq!(read_json(&dir.join("index.json"))["version"], 2);
}

#[test]
fn an_entry_whose_file_is_gone_is_dropped_and_the_rest_migrate() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("r");
    v1_store(&dir, false);
    let index = r#"{"version":1,"entries":[
        {"root":"/c","item":"a.yml","id":"00000000000000aa"},
        {"root":"/c","item":"b.yml","id":"00000000000000bb"}]}"#;
    std::fs::write(dir.join("index.json"), index).unwrap();
    let store = ResponseStore::new(dir.clone());
    assert!(store.load(Path::new("/c"), &ip("b.yml")).is_none());
    assert_eq!(statuses(&store, Path::new("/c"), "a.yml"), [200]);
    assert_eq!(
        read_json(&dir.join("index.json"))["entries"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[cfg(unix)]
#[test]
fn a_failed_migration_keeps_the_version_1_files_for_the_next_read() {
    use std::os::unix::fs::PermissionsExt as _;
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("r");
    v1_store(&dir, false);
    // A read-only directory makes every rename fail.
    let _g = crate::support::ModeGuard::set(&dir, 0o555);
    let store = ResponseStore::new(dir.clone());
    let root = Path::new("/c");
    assert!(store.load(root, &ip("a.yml")).is_none());
    assert!(
        store
            .save(root, &ip("a.yml"), &sent("a.yml", 201, "x"), at(9))
            .is_err()
    );
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        listing(&dir),
        ["00000000000000aa.json".to_owned(), "index.json".to_owned()]
    );
    assert_eq!(read_json(&dir.join("index.json"))["version"], 1);
    let got = store
        .load(root, &ip("a.yml"))
        .expect("migrated on the next read");
    assert_eq!(got.result.response.unwrap().text(), "hello");
}

#[cfg(unix)]
#[test]
fn an_unreadable_index_is_never_overwritten() {
    use std::os::unix::fs::PermissionsExt as _;
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("r");
    v1_store(&dir, false);
    let index = dir.join("index.json");
    let _g = crate::support::ModeGuard::set(&index, 0o000);
    let store = ResponseStore::new(dir.clone());
    let root = Path::new("/c");
    assert!(store.load(root, &ip("a.yml")).is_none());
    assert!(
        store
            .save(root, &ip("a.yml"), &sent("a.yml", 201, "x"), at(9))
            .is_err()
    );
    std::fs::set_permissions(&index, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        listing(&dir),
        ["00000000000000aa.json".to_owned(), "index.json".to_owned()]
    );
    assert_eq!(read_json(&index)["version"], 1);
}

#[test]
fn reading_history_leaves_a_version_1_store_as_it_is() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("r");
    v1_store(&dir, true);
    let snapshot = |dir: &Path| -> Vec<(String, Vec<u8>)> {
        listing(dir)
            .into_iter()
            .map(|n| (n.clone(), std::fs::read(dir.join(&n)).unwrap()))
            .collect()
    };
    let before = snapshot(&dir);
    let store = ResponseStore::new(dir.clone());
    let h = store.history(Path::new("/c"), &ip("a.yml"));
    assert_eq!(snapshot(&dir), before);
    assert_eq!(h.len(), 1);
    assert_eq!((h[0].seq, h[0].status, h[0].size), (0, 200, 5));
}

// Remembering off reads kept sends through `peek`, which never migrates or drops anything.
#[test]
fn peeking_a_version_1_store_reads_it_in_place() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("r");
    v1_store(&dir, true);
    let index = std::fs::read_to_string(dir.join("index.json")).unwrap();
    let store = ResponseStore::new(dir.clone());
    let got = store.peek(Path::new("/c"), &ip("a.yml")).expect("kept");
    assert_eq!(got.saved_at, at(5));
    assert_eq!(got.result.response.unwrap().body(), [0xffu8, 0xfe]);
    assert!(store.peek(Path::new("/c"), &ip("b.yml")).is_none());
    assert_eq!(
        listing(&dir),
        [
            "00000000000000aa.body".to_owned(),
            "00000000000000aa.json".to_owned(),
            "index.json".to_owned()
        ]
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("index.json")).unwrap(),
        index
    );
}

#[test]
fn peeking_an_unreadable_send_keeps_it() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("r");
    let store = ResponseStore::new(dir.clone());
    let root = Path::new("/c");
    store
        .save(root, &ip("a.yml"), &sent("a.yml", 200, "kept"), at(6))
        .unwrap();
    let json = listing(&dir)
        .into_iter()
        .find(|n| n.ends_with(".0.json"))
        .expect("a send file");
    std::fs::write(dir.join(&json), "not json").unwrap();
    let before = listing(&dir);
    let index = std::fs::read_to_string(dir.join("index.json")).unwrap();
    assert!(store.peek(root, &ip("a.yml")).is_none());
    assert_eq!(listing(&dir), before);
    assert_eq!(
        std::fs::read_to_string(dir.join("index.json")).unwrap(),
        index
    );
}

#[test]
fn peeking_a_send_of_a_version_1_store_reads_it_in_place() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("r");
    v1_store(&dir, false);
    let before = listing(&dir);
    let index = std::fs::read_to_string(dir.join("index.json")).unwrap();
    let store = ResponseStore::new(dir.clone());
    let got = store
        .peek_send(Path::new("/c"), &ip("a.yml"), 0)
        .expect("send 0 of the v1 key");
    assert_eq!(got.result.response.unwrap().text(), "hello");
    assert!(store.peek_send(Path::new("/c"), &ip("a.yml"), 1).is_none());
    assert_eq!(listing(&dir), before, "peeking moved files");
    assert_eq!(
        std::fs::read_to_string(dir.join("index.json")).unwrap(),
        index,
        "peeking rewrote the index"
    );
}
