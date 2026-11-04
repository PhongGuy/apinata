//! The startup sweep that re-masks kept sends written before their headers took the mask.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use apinata_app::last_response::{FORMAT_VERSION, ResponseStore, install};
use apinata_core::ItemPath;
use gpui_kit::TestAppContext;

const URL_PASSWORD: &str = "hunter2pw";
// The raw values an old kept send holds on disk.
const OLD_RAW: [&str; 3] = [URL_PASSWORD, "cookievalue1", "tok123secret"];
const MARKER: &str = "remask.done";
// The marker version the current sweep writes.
const SWEEP: u32 = 3;

fn ip(s: &str) -> ItemPath {
    ItemPath::new(s).unwrap()
}

// One send in the format kept before headers were masked: raw values, no `masked_headers`, a
// snapshot url with its password.
fn old_entry(saved_at_ms: u64) -> Vec<u8> {
    let snapshot = serde_json::json!({
        "http": {"method": "GET", "url": format!("http://joe:{URL_PASSWORD}@127.0.0.1:1/x")}
    })
    .to_string();
    let entry = serde_json::json!({
        "version": 1, "saved_at_ms": saved_at_ms, "name": "a", "outcome": "passed",
        "status": 302, "status_text": "Found", "url": "http://joe@127.0.0.1:1/x",
        "headers": [
            ["Location", format!("http://joe:{URL_PASSWORD}@127.0.0.1:1/ok")],
            ["Set-Cookie", "sid=cookievalue1"],
            ["Authorization", "Bearer tok123secret"],
            ["Content-Type", "text/plain"],
        ],
        "time_ms": 5.0, "size": 0, "truncated": false,
        "body_dropped": false, "text": "", "has_body_file": false,
        "assertions": [], "tests": [], "request": snapshot, "masked": false
    });
    serde_json::to_vec(&entry).unwrap()
}

fn send_meta(seq: u64) -> serde_json::Value {
    serde_json::json!({
        "seq": seq, "saved_at_ms": 1000 + seq, "status": 302, "time_ms": 5.0, "size": 0,
        "bytes": 10, "method": "GET", "url": "http://joe@127.0.0.1:1/x", "order": seq + 1
    })
}

// A store of old sends: `a.yml` with sends 0 and 1 under id `k`, `b.yml` with send 0 under id
// `m`, and `v.yml` whose send 0 a crash left under its version 1 name `v.json`.
fn old_store(dir: &Path) -> ResponseStore {
    let responses = dir.join("responses");
    std::fs::create_dir_all(&responses).unwrap();
    for (name, at) in [("k.0.json", 1000), ("k.1.json", 1001), ("m.0.json", 1000)] {
        std::fs::write(responses.join(name), old_entry(at)).unwrap();
    }
    std::fs::write(responses.join("v.json"), old_entry(900)).unwrap();
    let root = dir.to_string_lossy();
    let index = serde_json::json!({
        "version": FORMAT_VERSION,
        "entries": [
            {"root": root, "item": "a.yml", "id": "k", "sends": [send_meta(0), send_meta(1)]},
            {"root": root, "item": "b.yml", "id": "m", "sends": [send_meta(0)]},
            {"root": root, "item": "v.yml", "id": "v", "sends": [send_meta(0)]},
        ]
    });
    std::fs::write(
        responses.join("index.json"),
        serde_json::to_vec(&index).unwrap(),
    )
    .unwrap();
    ResponseStore::new(responses)
}

// The files under `dir` that hold `needle`.
fn store_holds(dir: &Path, needle: &str) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            let bytes = std::fs::read(p).unwrap();
            bytes.windows(needle.len()).any(|w| w == needle.as_bytes())
        })
        .map(|p| p.display().to_string())
        .collect()
}

fn assert_no_raw_values(dir: &Path) {
    for raw in OLD_RAW {
        let leaks = store_holds(dir, raw);
        assert!(leaks.is_empty(), "{raw} still in {leaks:?}");
    }
}

fn sweep(store: &ResponseStore, cx: &mut TestAppContext) {
    let store = store.clone();
    cx.update(|cx| install(store, cx));
    cx.run_until_parked();
}

fn set_old_mtime(path: &Path) -> std::time::SystemTime {
    let old = UNIX_EPOCH + Duration::from_secs(1_000_000);
    set_mtime(path, old);
    old
}

fn set_mtime(path: &Path, at: SystemTime) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(at)
        .unwrap();
}

fn index_of(dir: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(dir.join("index.json")).unwrap()).unwrap()
}

// The index with every send's `bytes` dropped, to compare what else a sweep changed.
fn without_bytes(mut index: serde_json::Value) -> serde_json::Value {
    for entry in index["entries"].as_array_mut().unwrap() {
        for send in entry["sends"].as_array_mut().unwrap() {
            send.as_object_mut().unwrap().remove("bytes");
        }
    }
    index
}

// Every listed send's `bytes` is its json and body sizes on disk; send 0 may sit under its
// version 1 names.
fn assert_index_bytes_match_disk(dir: &Path) {
    let size = |name: String| std::fs::metadata(dir.join(name)).map_or(0, |m| m.len());
    for entry in index_of(dir)["entries"].as_array().unwrap() {
        let id = entry["id"].as_str().unwrap();
        for send in entry["sends"].as_array().unwrap() {
            let seq = send["seq"].as_u64().unwrap();
            let v1 = seq == 0 && dir.join(format!("{id}.json")).exists();
            let disk = if v1 {
                size(format!("{id}.json")) + size(format!("{id}.body"))
            } else {
                size(format!("{id}.{seq}.json")) + size(format!("{id}.{seq}.body"))
            };
            assert_eq!(send["bytes"].as_u64().unwrap(), disk, "{id}.{seq}");
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn write_marker(dir: &Path, version: u32, checked_ms: u64) {
    let marker = serde_json::json!({"version": version, "checked_ms": checked_ms});
    std::fs::write(dir.join(MARKER), serde_json::to_vec(&marker).unwrap()).unwrap();
}

fn marker_checked_ms(dir: &Path) -> u64 {
    let marker: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join(MARKER)).unwrap()).unwrap();
    assert_eq!(marker["version"], SWEEP);
    marker["checked_ms"].as_u64().unwrap()
}

#[gpui_kit::test]
fn installing_the_store_re_masks_every_old_send_without_loading_one(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = old_store(dir.path());
    // The sweep changes only each send's `bytes` and drops its url's username.
    let mut index = index_of(store.dir());
    for entry in index["entries"].as_array_mut().unwrap() {
        for send in entry["sends"].as_array_mut().unwrap() {
            send["url"] = serde_json::json!("http://127.0.0.1:1/x");
        }
    }
    sweep(&store, cx);
    assert_no_raw_values(store.dir());
    assert_eq!(without_bytes(index_of(store.dir())), without_bytes(index));
    assert_index_bytes_match_disk(store.dir());
    assert!(store.dir().join(MARKER).exists());
    assert_eq!(store.history(dir.path(), &ip("a.yml")).len(), 2);
    assert_eq!(store.history(dir.path(), &ip("b.yml")).len(), 1);
    for (item, seq) in [("a.yml", 0), ("a.yml", 1), ("b.yml", 0)] {
        let stored = store.load_send(dir.path(), &ip(item), seq).expect("kept");
        assert_eq!(stored.masked_headers, vec![0, 1, 2], "{item} {seq}");
    }
}

#[gpui_kit::test]
fn a_send_older_than_the_marker_watermark_is_not_read(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = old_store(dir.path());
    write_marker(store.dir(), SWEEP, now_ms() - 60_000);
    let raw = store.dir().join("k.0.json");
    let old = set_old_mtime(&raw);
    let before = std::fs::read(&raw).unwrap();
    sweep(&store, cx);
    assert_eq!(std::fs::read(&raw).unwrap(), before);
    assert_eq!(std::fs::metadata(&raw).unwrap().modified().unwrap(), old);
}

#[gpui_kit::test]
fn a_raw_send_written_after_the_marker_is_masked_and_the_watermark_moves(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = old_store(dir.path());
    let checked = now_ms() - 60_000;
    write_marker(store.dir(), SWEEP, checked);
    sweep(&store, cx);
    assert_no_raw_values(store.dir());
    assert!(marker_checked_ms(store.dir()) > checked);
}

#[gpui_kit::test]
fn a_version_1_marker_gets_a_full_sweep(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = old_store(dir.path());
    std::fs::write(store.dir().join(MARKER), r#"{"version": 1}"#).unwrap();
    set_old_mtime(&store.dir().join("k.0.json"));
    sweep(&store, cx);
    assert_no_raw_values(store.dir());
    assert!(marker_checked_ms(store.dir()) > 0);
}

#[gpui_kit::test]
fn loading_an_old_send_keeps_its_index_bytes_true_to_disk(_cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = old_store(dir.path());
    store.load(dir.path(), &ip("a.yml")).expect("kept");
    let index = index_of(store.dir());
    let size = std::fs::metadata(store.dir().join("k.1.json"))
        .unwrap()
        .len();
    assert_eq!(
        index["entries"][0]["sends"][1]["bytes"].as_u64(),
        Some(size)
    );
}

// Old orphans beside the store: an unlisted raw send with its body, and a version 1 name.
fn add_old_orphans(dir: &Path) -> Vec<PathBuf> {
    let orphans = ["o.0.json", "o.0.body", "p.json"].map(|n| dir.join(n));
    for orphan in &orphans {
        std::fs::write(orphan, old_entry(800)).unwrap();
        set_old_mtime(orphan);
    }
    orphans.to_vec()
}

#[gpui_kit::test]
fn the_sweep_removes_old_orphans_and_keeps_everything_else(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = old_store(dir.path());
    let orphans = add_old_orphans(store.dir());
    let young = store.dir().join("n.0.json");
    std::fs::write(&young, old_entry(700)).unwrap();
    let listed = store.dir().join("k.0.json");
    set_old_mtime(&listed);
    let keep = [".tmpAbC123", "notes.txt", "k.x.json", "k.0.json.bak"].map(|n| store.dir().join(n));
    for file in &keep {
        std::fs::write(file, b"x").unwrap();
        set_old_mtime(file);
    }
    std::fs::write(store.dir().join(MARKER), r#"{"version": 1}"#).unwrap();
    sweep(&store, cx);
    for orphan in &orphans {
        assert!(!orphan.exists(), "{}", orphan.display());
    }
    assert_eq!(std::fs::read(&young).unwrap(), old_entry(700));
    assert!(listed.exists());
    for file in &keep {
        assert_eq!(std::fs::read(file).unwrap(), b"x", "{}", file.display());
    }
    assert!(store.dir().join(MARKER).exists());
    assert!(store.dir().join("index.json").exists());
}

#[gpui_kit::test]
fn an_unreadable_or_version_1_index_removes_no_orphan(cx: &mut TestAppContext) {
    let v1 = serde_json::json!({
        "version": 1, "entries": [{"root": "/r", "item": "a.yml", "id": "k"}]
    });
    for index in [b"{ not json".to_vec(), serde_json::to_vec(&v1).unwrap()] {
        let dir = tempfile::tempdir().unwrap();
        let store = old_store(dir.path());
        std::fs::write(store.dir().join("index.json"), &index).unwrap();
        let orphans = add_old_orphans(store.dir());
        sweep(&store, cx);
        for orphan in &orphans {
            assert!(orphan.exists(), "{}", orphan.display());
        }
    }
}

#[gpui_kit::test]
fn an_interrupted_sweep_resumes_without_rewriting_a_masked_send(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = old_store(dir.path());
    // A load rewrites `k`'s newest send masked, as the part of a sweep that got that far did.
    store.load(dir.path(), &ip("a.yml")).expect("kept");
    let masked = store.dir().join("k.1.json");
    let old = set_old_mtime(&masked);
    let before = std::fs::read(&masked).unwrap();
    assert!(!store.dir().join(MARKER).exists());
    sweep(&store, cx);
    assert_no_raw_values(store.dir());
    assert!(store.dir().join(MARKER).exists());
    assert_eq!(std::fs::read(&masked).unwrap(), before);
    assert_eq!(std::fs::metadata(&masked).unwrap().modified().unwrap(), old);
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_failed_rewrite_writes_no_marker_and_the_next_start_finishes(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let store = old_store(dir.path());
    let names = ["index.json", "k.0.json", "k.1.json", "m.0.json", "v.json"];
    let before: Vec<Vec<u8>> = (names.iter())
        .map(|n| std::fs::read(store.dir().join(n)).unwrap())
        .collect();
    let mode = |m| std::fs::Permissions::from_mode(m);
    std::fs::set_permissions(store.dir(), mode(0o555)).unwrap();
    sweep(&store, cx);
    std::fs::set_permissions(store.dir(), mode(0o755)).unwrap();
    assert!(!store.dir().join(MARKER).exists());
    for (name, bytes) in names.iter().zip(&before) {
        assert_eq!(
            &std::fs::read(store.dir().join(name)).unwrap(),
            bytes,
            "{name}"
        );
    }
    sweep(&store, cx);
    assert_no_raw_values(store.dir());
    assert!(store.dir().join(MARKER).exists());
}

#[gpui_kit::test]
fn a_corrupt_or_foreign_send_is_left_untouched_and_the_sweep_finishes(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = old_store(dir.path());
    let corrupt = store.dir().join("x.0.json");
    std::fs::write(&corrupt, b"{ not json").unwrap();
    let foreign = store.dir().join("y.0.json");
    let mut entry: serde_json::Value = serde_json::from_slice(&old_entry(1000)).unwrap();
    entry["version"] = 99.into();
    let foreign_bytes = serde_json::to_vec(&entry).unwrap();
    std::fs::write(&foreign, &foreign_bytes).unwrap();
    let not_utf8 = store.dir().join("z.0.json");
    std::fs::write(&not_utf8, b"{\xff\xfe}").unwrap();
    sweep(&store, cx);
    assert_eq!(std::fs::read(&not_utf8).unwrap(), b"{\xff\xfe}");
    assert_eq!(std::fs::read(&corrupt).unwrap(), b"{ not json");
    assert_eq!(std::fs::read(&foreign).unwrap(), foreign_bytes);
    assert!(store.dir().join(MARKER).exists());
    for raw in OLD_RAW {
        let leaks: Vec<String> = (store_holds(store.dir(), raw).into_iter())
            .filter(|p| !p.ends_with("y.0.json"))
            .collect();
        assert!(leaks.is_empty(), "{raw} still in {leaks:?}");
    }
}

#[gpui_kit::test]
fn a_young_unlisted_send_holds_the_watermark_below_its_mtime(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = old_store(dir.path());
    let young = store.dir().join("n.0.json");
    std::fs::write(&young, old_entry(700)).unwrap();
    let at = SystemTime::now() - Duration::from_secs(60);
    set_mtime(&young, at);
    sweep(&store, cx);
    let at_ms = at.duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
    assert!(marker_checked_ms(store.dir()) <= at_ms - 5_000);
}

// A send file already masked: `old_store`'s `k.0.json` after one sweep.
fn masked_entry_bytes(cx: &mut TestAppContext) -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    let store = old_store(dir.path());
    sweep(&store, cx);
    std::fs::read(store.dir().join("k.0.json")).unwrap()
}

#[gpui_kit::test]
fn a_young_unlisted_masked_send_does_not_hold_the_watermark(cx: &mut TestAppContext) {
    let masked = masked_entry_bytes(cx);
    let dir = tempfile::tempdir().unwrap();
    let store = old_store(dir.path());
    let at = SystemTime::now() - Duration::from_secs(60);
    for (name, bytes) in [("n.0.json", &masked[..]), ("n.0.body", b"raw body")] {
        let path = store.dir().join(name);
        std::fs::write(&path, bytes).unwrap();
        set_mtime(&path, at);
    }
    let before = now_ms();
    sweep(&store, cx);
    assert!(
        marker_checked_ms(store.dir()) >= before - 5_000,
        "{} < {}",
        marker_checked_ms(store.dir()),
        before - 5_000
    );
    assert_eq!(std::fs::read(store.dir().join("n.0.json")).unwrap(), masked);
}

#[gpui_kit::test]
fn a_young_unlisted_raw_send_holds_the_watermark_and_is_not_rewritten(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = old_store(dir.path());
    let young = store.dir().join("n.0.json");
    let raw = old_entry(700);
    std::fs::write(&young, &raw).unwrap();
    let at = SystemTime::now() - Duration::from_secs(60);
    set_mtime(&young, at);
    sweep(&store, cx);
    let at_ms = at.duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
    assert!(marker_checked_ms(store.dir()) <= at_ms - 5_000);
    assert_eq!(std::fs::read(&young).unwrap(), raw);
    let mtime = std::fs::metadata(&young).unwrap().modified().unwrap();
    assert_eq!(mtime, at);
}

const USERNAME: &str = "acct-7781";

// A store of one `a.yml` send kept by an older version, whose entry and index urls hold a username, as
// does its snapshot. Its file is older than a version 2 marker, which the sweep must not trust.
fn username_store(dir: &Path) -> ResponseStore {
    let responses = dir.join("responses");
    std::fs::create_dir_all(&responses).unwrap();
    let url = format!("http://{USERNAME}@127.0.0.1:9/x");
    let snapshot = serde_json::json!({"http": {"method": "GET", "url": url}}).to_string();
    let entry = serde_json::json!({
        "version": 1, "saved_at_ms": 1000, "name": "a", "outcome": "passed",
        "status": 200, "status_text": "OK", "url": url,
        "headers": [["Content-Type", "text/plain"]],
        "time_ms": 5.0, "size": 0, "truncated": false,
        "body_dropped": false, "text": "", "has_body_file": false,
        "assertions": [], "tests": [], "request": snapshot, "masked": false
    });
    let json = responses.join("k.0.json");
    std::fs::write(&json, serde_json::to_vec(&entry).unwrap()).unwrap();
    set_old_mtime(&json);
    let mut send = send_meta(0);
    send["url"] = serde_json::json!(url);
    send["bytes"] = serde_json::json!(std::fs::metadata(&json).unwrap().len());
    let index = serde_json::json!({
        "version": FORMAT_VERSION,
        "entries": [{"root": dir.to_string_lossy(), "item": "a.yml", "id": "k", "sends": [send]}]
    });
    std::fs::write(
        responses.join("index.json"),
        serde_json::to_vec(&index).unwrap(),
    )
    .unwrap();
    write_marker(&responses, 2, now_ms());
    ResponseStore::new(responses)
}

fn marker_version(dir: &Path) -> u64 {
    let marker: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join(MARKER)).unwrap()).unwrap();
    marker["version"].as_u64().unwrap()
}

// The send json without its snapshot, and the index, hold no username.
fn assert_no_username_outside_the_snapshot(dir: &Path) {
    let mut entry: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("k.0.json")).unwrap()).unwrap();
    entry.as_object_mut().unwrap().remove("request");
    assert!(!entry.to_string().contains(USERNAME), "{entry}");
    let index = std::fs::read_to_string(dir.join("index.json")).unwrap();
    assert!(!index.contains(USERNAME), "{index}");
}

#[gpui_kit::test]
fn the_sweep_drops_old_kept_send_usernames_but_keeps_the_snapshot(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = username_store(dir.path());
    sweep(&store, cx);
    assert_no_username_outside_the_snapshot(store.dir());
    assert_eq!(marker_version(store.dir()), u64::from(SWEEP));
    let snapshot = store.request_json(dir.path(), &ip("a.yml"), 0).unwrap();
    assert!(
        snapshot.contains(&format!("{USERNAME}@127.0.0.1")),
        "{snapshot}"
    );
    assert_eq!(store.history(dir.path(), &ip("a.yml")).len(), 1);
    assert!(store.load_send(dir.path(), &ip("a.yml"), 0).is_some());
    assert_index_bytes_match_disk(store.dir());
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_read_only_store_keeps_its_old_marker_and_the_next_start_drops_usernames(
    cx: &mut TestAppContext,
) {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let store = username_store(dir.path());
    let mode = |m| std::fs::Permissions::from_mode(m);
    std::fs::set_permissions(store.dir(), mode(0o555)).unwrap();
    sweep(&store, cx);
    std::fs::set_permissions(store.dir(), mode(0o755)).unwrap();
    assert_ne!(marker_version(store.dir()), u64::from(SWEEP));
    assert_eq!(store.history(dir.path(), &ip("a.yml")).len(), 1);
    sweep(&store, cx);
    assert_no_username_outside_the_snapshot(store.dir());
    assert_eq!(marker_version(store.dir()), u64::from(SWEEP));
}

// Migrating a version 1 index writes no url username, even from a send no sweep has masked
// yet: the sweep skips a version 1 index, so it would not mask one written meanwhile.
#[gpui_kit::test]
fn a_migrated_version_1_index_holds_no_url_username(_cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = username_store(dir.path());
    std::fs::rename(store.dir().join("k.0.json"), store.dir().join("k.json")).unwrap();
    let v1 = serde_json::json!({
        "version": 1, "entries": [{"root": dir.path().to_string_lossy(), "item": "a.yml", "id": "k"}]
    });
    std::fs::write(
        store.dir().join("index.json"),
        serde_json::to_vec(&v1).unwrap(),
    )
    .unwrap();
    // Saving another item migrates the index without reading `a.yml`'s send back.
    let other = apinata_core::RequestResult::from_parts(
        ip("b.yml"),
        "B",
        apinata_core::Outcome::Passed,
        Some(apinata_core::Response::new(200).with_url("http://127.0.0.1:9/b")),
        vec![],
    );
    let limits = apinata_app::last_response::Limits::default();
    (store.save_redacted(dir.path(), &ip("b.yml"), &other, &[], UNIX_EPOCH, limits)).unwrap();
    let index = std::fs::read_to_string(store.dir().join("index.json")).unwrap();
    assert!(
        index.contains("\"version\": 2") || index.contains("\"version\":2"),
        "{index}"
    );
    assert!(!index.contains(USERNAME), "{index}");
}
