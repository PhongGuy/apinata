//! A kept send keeps a keyed digest of each masked header's true value under the key's id;
//! a file without the field loads as before, and a malformed one reads as missing.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use apinata_app::last_response::{DIGEST_KEY_NAME, KeptDigests, Limits, ResponseStore, Stored};
use apinata_core::{DigestKey, ItemPath, MemoryStore, Outcome, RequestResult, Response};
use serde_json::{Value, json};

const COOKIE: &str = "sid=cookievalue66";

fn ip(s: &str) -> ItemPath {
    ItemPath::new(s).unwrap()
}

fn key() -> DigestKey {
    DigestKey::load(&MemoryStore::new(), DIGEST_KEY_NAME).unwrap()
}

fn send(response: Response) -> RequestResult {
    RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(response),
        Vec::new(),
    )
}

fn cookie_send() -> RequestResult {
    send(
        Response::new(200)
            .with_header("Content-Type", "application/json")
            .with_header("Set-Cookie", COOKIE)
            .with_text_body("{}"),
    )
}

fn store_in(dir: &tempfile::TempDir, key: Option<&DigestKey>) -> ResponseStore {
    let store = ResponseStore::new(dir.path().join("responses"));
    if let Some(key) = key {
        store.set_digest_key(key.clone());
    }
    store
}

// A store over a fresh dir, holding `key` when given, with one cookie send saved.
fn saved(key: Option<&DigestKey>) -> (tempfile::TempDir, ResponseStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(&dir, key);
    store
        .save(Path::new("/c"), &ip("a.yml"), &cookie_send(), UNIX_EPOCH)
        .unwrap();
    (dir, store)
}

fn send_file(store: &ResponseStore) -> PathBuf {
    (std::fs::read_dir(store.dir()).unwrap())
        .map(|e| e.unwrap().path())
        .find(|p| p.to_string_lossy().ends_with(".0.json"))
        .expect("a send file")
}

fn read(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn write(path: &Path, value: &Value) {
    std::fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}

fn load(store: &ResponseStore) -> Option<Stored> {
    store.load(Path::new("/c"), &ip("a.yml"))
}

#[test]
fn a_masked_header_is_kept_with_its_digest_under_the_key_id() {
    let key = key();
    let (_dir, store) = saved(Some(&key));
    let path = send_file(&store);
    let digest = key.digest("Set-Cookie", COOKIE);
    assert_eq!(
        read(&path)["digests"],
        json!({ "key": key.id(), "values": [[1, digest.clone()]] })
    );
    assert!(
        !std::fs::read_to_string(&path)
            .unwrap()
            .contains("cookievalue66")
    );
    let stored = load(&store).expect("kept");
    assert_eq!(stored.masked_headers, vec![1]);
    let want = KeptDigests {
        key: key.id().to_owned(),
        values: vec![(1, digest)],
    };
    assert_eq!(stored.header_digests, Some(want));
}

#[test]
fn a_header_scrubbed_of_a_secret_is_digested_as_sent() {
    let key = key();
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(&dir, Some(&key));
    let result = send(
        Response::new(200)
            .with_header("x-echo", "tok-66-secret")
            .with_text_body("{}"),
    );
    let secrets = ["tok-66-secret".to_owned()];
    (store.save_redacted(
        Path::new("/c"),
        &ip("a.yml"),
        &result,
        &secrets,
        UNIX_EPOCH,
        Limits::default(),
    ))
    .unwrap();
    let digests = load(&store).expect("kept").header_digests.map(|d| d.values);
    assert_eq!(
        digests,
        Some(vec![(0, key.digest("x-echo", "tok-66-secret"))])
    );
}

#[test]
fn a_store_without_a_key_writes_no_digests() {
    let (_dir, store) = saved(None);
    assert_eq!(read(&send_file(&store)).get("digests"), None);
    assert_eq!(load(&store).expect("kept").header_digests, None);
}

#[test]
fn a_clone_of_the_store_shares_its_key() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(&dir, None);
    let worker = store.clone();
    assert!(worker.digest_key().is_none());
    let key = key();
    store.set_digest_key(key.clone());
    assert_eq!(
        worker.digest_key().map(|k| k.id().to_owned()),
        Some(key.id().to_owned())
    );
}

#[test]
fn a_send_file_written_before_digests_loads_without_them() {
    let (_dir, store) = saved(Some(&key()));
    let path = send_file(&store);
    let mut file = read(&path);
    file.as_object_mut().unwrap().remove("digests");
    write(&path, &file);
    let stored = load(&store).expect("an old file still loads");
    assert_eq!(stored.masked_headers, vec![1]);
    assert_eq!(stored.header_digests, None);
}

#[test]
fn digests_of_places_not_masked_are_dropped_on_read() {
    let key = key();
    let (_dir, store) = saved(Some(&key));
    let path = send_file(&store);
    let mut file = read(&path);
    let digest = key.digest("Set-Cookie", COOKIE);
    file["digests"]["values"] = json!([[0, "planted"], [1, digest.clone()], [7, "gone"]]);
    write(&path, &file);
    let values = load(&store).expect("kept").header_digests.map(|d| d.values);
    assert_eq!(values, Some(vec![(1, digest)]));
}

#[test]
fn a_remask_keeps_the_digests_and_gives_a_newly_masked_value_none() {
    let key = key();
    let (_dir, store) = saved(Some(&key));
    let path = send_file(&store);
    let mut file = read(&path);
    // A url header kept raw, as a file from before the url mask holds it.
    (file["headers"].as_array_mut().unwrap())
        .push(json!(["Location", "http://joe:pw66@127.0.0.1:1/x"]));
    write(&path, &file);
    let stored = load(&store).expect("kept");
    assert_eq!(stored.masked_headers, vec![1, 2]);
    let values = stored.header_digests.expect("kept digests").values;
    assert_eq!(values, vec![(1, key.digest("Set-Cookie", COOKIE))]);
    assert_eq!(
        read(&path)["digests"]["key"],
        json!(key.id()),
        "the rewrite keeps the field"
    );
}

#[test]
fn a_malformed_digests_field_reads_as_a_missing_send() {
    let (_dir, store) = saved(Some(&key()));
    let path = send_file(&store);
    let mut file = read(&path);
    file["digests"] = json!("not a digest map");
    write(&path, &file);
    assert!(load(&store).is_none());
}
