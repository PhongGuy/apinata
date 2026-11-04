//! Store format version 2: several sends per request.

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use apinata_app::last_response::{Limits, ResponseStore, SendExtra};
use apinata_app::last_response::{StoreOp, enqueue, install};
use apinata_core::{ItemPath, Outcome, RequestResult, Response};
use gpui_kit::TestAppContext;

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

fn keep(n: usize) -> Limits {
    Limits {
        per_request: n,
        total_bytes: u64::MAX,
    }
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

#[test]
fn each_send_is_kept_newest_first() {
    let data = tempfile::tempdir().unwrap();
    let store = ResponseStore::new(data.path().join("r"));
    let root = Path::new("/c");
    for (i, status) in [201u16, 202, 203].into_iter().enumerate() {
        let r = sent("a.yml", status, "x");
        store
            .save_with(root, &ip("a.yml"), &r, at(1_000 + i as u64), keep(20))
            .unwrap();
    }
    let h = store.history(root, &ip("a.yml"));
    assert_eq!(
        h.iter().map(|s| s.status).collect::<Vec<_>>(),
        [203, 202, 201]
    );
    assert_eq!(h.iter().map(|s| s.seq).collect::<Vec<_>>(), [2, 1, 0]);
    assert_eq!(h[0].saved_at, at(1_002));
    assert_eq!(h[0].time, Duration::from_millis(7));
    assert_eq!(h[0].size, 1);
    let newest = store.load(root, &ip("a.yml")).unwrap();
    assert_eq!(newest.result.response.unwrap().status(), 203);
    let old = store.load_send(root, &ip("a.yml"), 0).unwrap();
    assert_eq!(old.saved_at, at(1_000));
    assert_eq!(old.result.response.unwrap().status(), 201);
    assert!(store.load_send(root, &ip("a.yml"), 9).is_none());
    assert!(store.history(root, &ip("b.yml")).is_empty());
}

#[test]
fn the_per_request_limit_drops_the_oldest_and_its_files() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("r");
    let store = ResponseStore::new(dir.clone());
    let root = Path::new("/c");
    for i in 0..4u64 {
        let r = sent("a.yml", 200 + i as u16, "x");
        store
            .save_with(root, &ip("a.yml"), &r, at(i), keep(2))
            .unwrap();
    }
    let seqs: Vec<u64> = store
        .history(root, &ip("a.yml"))
        .iter()
        .map(|s| s.seq)
        .collect();
    assert_eq!(seqs, [3, 2]);
    let id = read_json(&dir.join("index.json"))["entries"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        listing(&dir),
        [
            format!("{id}.2.json"),
            format!("{id}.3.json"),
            "index.json".to_owned()
        ]
    );
    // Zero keeps the latest send alone, as before history existed.
    let r = sent("a.yml", 299, "x");
    store
        .save_with(root, &ip("a.yml"), &r, at(9), keep(0))
        .unwrap();
    assert_eq!(statuses(&store, root, "a.yml"), [299]);
}

#[test]
fn the_files_follow_the_version_2_layout() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("r");
    let store = ResponseStore::new(dir.clone());
    let r = sent("a.yml", 200, "hello");
    store
        .save(Path::new("/c"), &ip("a.yml"), &r, at(5))
        .unwrap();
    let index = read_json(&dir.join("index.json"));
    assert_eq!(index["version"], 2);
    let entry = &index["entries"][0];
    assert_eq!(entry["root"], "/c");
    assert_eq!(entry["item"], "a.yml");
    let id = entry["id"].as_str().unwrap().to_owned();
    assert_eq!(id.len(), 16);
    let send = &entry["sends"][0];
    assert_eq!(send["seq"], 0);
    assert_eq!(send["saved_at_ms"], 5);
    assert_eq!(send["status"], 200);
    assert_eq!(send["time_ms"], 7.0);
    assert_eq!(send["size"], 5);
    let file = dir.join(format!("{id}.0.json"));
    assert_eq!(send["bytes"], std::fs::metadata(&file).unwrap().len());
    let meta = read_json(&file);
    assert_eq!(meta["version"], 1);
    assert_eq!(meta["text"], "hello");
    assert_eq!(
        listing(&dir),
        [format!("{id}.0.json"), "index.json".to_owned()]
    );
}

#[test]
fn an_unknown_index_version_reads_as_empty() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("r");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("index.json"), r#"{"version":3,"entries":[]}"#).unwrap();
    let store = ResponseStore::new(dir.clone());
    assert!(store.load(Path::new("/c"), &ip("a.yml")).is_none());
    store
        .save(
            Path::new("/c"),
            &ip("a.yml"),
            &sent("a.yml", 200, "x"),
            at(1),
        )
        .unwrap();
    assert_eq!(read_json(&dir.join("index.json"))["version"], 2);
}

#[test]
fn a_bad_send_file_drops_only_that_send() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("r");
    let store = ResponseStore::new(dir.clone());
    let root = Path::new("/c");
    store
        .save(root, &ip("a.yml"), &sent("a.yml", 201, "x"), at(1))
        .unwrap();
    store
        .save(root, &ip("a.yml"), &sent("a.yml", 202, "x"), at(2))
        .unwrap();
    let id = read_json(&dir.join("index.json"))["entries"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    std::fs::write(dir.join(format!("{id}.0.json")), "{not json").unwrap();
    assert!(store.load_send(root, &ip("a.yml"), 0).is_none());
    assert_eq!(statuses(&store, root, "a.yml"), [202]);
    assert!(!dir.join(format!("{id}.0.json")).exists());
    assert_eq!(store.load(root, &ip("a.yml")).unwrap().saved_at, at(2));
}

#[test]
fn rename_and_remove_carry_every_send() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("r");
    let store = ResponseStore::new(dir.clone());
    let root = Path::new("/c");
    store
        .save(
            root,
            &ip("users/a.yml"),
            &sent("users/a.yml", 201, "x"),
            at(1),
        )
        .unwrap();
    store
        .save(
            root,
            &ip("users/a.yml"),
            &sent("users/a.yml", 202, "x"),
            at(2),
        )
        .unwrap();
    store.rename(root, &ip("users"), &ip("people")).unwrap();
    assert!(store.history(root, &ip("users/a.yml")).is_empty());
    assert_eq!(statuses(&store, root, "people/a.yml"), [202, 201]);
    let old = store.load_send(root, &ip("people/a.yml"), 0).unwrap();
    assert_eq!(old.result.path, ip("people/a.yml"));
    store.remove(root, &ip("people")).unwrap();
    assert!(store.history(root, &ip("people/a.yml")).is_empty());
    assert_eq!(listing(&dir), ["index.json".to_owned()]);
}

#[test]
fn the_store_limit_evicts_the_oldest_sends_across_requests() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("r");
    let store = ResponseStore::new(dir.clone());
    let root = Path::new("/c");
    let body = "x".repeat(10_000);
    // Each send is a little over 10 kB on disk, so three do not fit in 25 kB.
    let lim = Limits {
        per_request: 20,
        total_bytes: 25_000,
    };
    store
        .save_with(root, &ip("a.yml"), &sent("a.yml", 201, &body), at(1), lim)
        .unwrap();
    store
        .save_with(root, &ip("b.yml"), &sent("b.yml", 202, &body), at(2), lim)
        .unwrap();
    store
        .save_with(root, &ip("a.yml"), &sent("a.yml", 203, &body), at(3), lim)
        .unwrap();
    assert_eq!(statuses(&store, root, "a.yml"), [203]);
    assert_eq!(statuses(&store, root, "b.yml"), [202]);
    store
        .save_with(root, &ip("c.yml"), &sent("c.yml", 204, &body), at(4), lim)
        .unwrap();
    assert!(
        store.history(root, &ip("b.yml")).is_empty(),
        "b's only send was the oldest"
    );
    assert!(store.load(root, &ip("b.yml")).is_none());
    let index = read_json(&dir.join("index.json"));
    assert_eq!(
        index["entries"].as_array().unwrap().len(),
        2,
        "b left the index"
    );
    let send_files = listing(&dir)
        .into_iter()
        .filter(|n| n != "index.json")
        .count();
    assert_eq!(send_files, 2);
}

#[test]
fn a_single_send_over_the_store_limit_is_still_kept() {
    let data = tempfile::tempdir().unwrap();
    let store = ResponseStore::new(data.path().join("r"));
    let root = Path::new("/c");
    let lim = Limits {
        per_request: 20,
        total_bytes: 10,
    };
    store
        .save_with(root, &ip("a.yml"), &sent("a.yml", 201, "x"), at(1), lim)
        .unwrap();
    assert_eq!(statuses(&store, root, "a.yml"), [201]);
    store
        .save_with(root, &ip("b.yml"), &sent("b.yml", 202, "x"), at(2), lim)
        .unwrap();
    assert!(store.history(root, &ip("a.yml")).is_empty());
    assert_eq!(statuses(&store, root, "b.yml"), [202]);
}

#[test]
fn recent_lists_sends_of_the_given_roots_newest_first() {
    let data = tempfile::tempdir().unwrap();
    let store = ResponseStore::new(data.path().join("r"));
    let (c, d) = (PathBuf::from("/c"), PathBuf::from("/d"));
    store
        .save(&c, &ip("a.yml"), &sent("a.yml", 201, "x"), at(1))
        .unwrap();
    store
        .save(&c, &ip("b.yml"), &sent("b.yml", 202, "x"), at(3))
        .unwrap();
    store
        .save(&d, &ip("x.yml"), &sent("x.yml", 203, "x"), at(2))
        .unwrap();
    store
        .save(&c, &ip("a.yml"), &sent("a.yml", 204, "x"), at(4))
        .unwrap();
    let got: Vec<(String, u64, u16)> = store
        .recent(std::slice::from_ref(&c), 10)
        .into_iter()
        .map(|r| {
            assert_eq!(r.root, c);
            (r.item.as_str().to_owned(), r.send.seq, r.send.status)
        })
        .collect();
    assert_eq!(
        got,
        [
            ("a.yml".to_owned(), 1, 204),
            ("b.yml".to_owned(), 0, 202),
            ("a.yml".to_owned(), 0, 201)
        ]
    );
    let two: Vec<u16> = store
        .recent(&[c.clone(), d.clone()], 2)
        .iter()
        .map(|r| r.send.status)
        .collect();
    assert_eq!(two, [204, 202]);
}

#[test]
fn sends_saved_in_one_millisecond_list_newest_first() {
    let data = tempfile::tempdir().unwrap();
    let store = ResponseStore::new(data.path().join("responses"));
    let c = PathBuf::from("/c");
    // A runner run can save several sends within one millisecond; save order breaks the tie.
    for (item, status) in [("a.yml", 201), ("b.yml", 202), ("a.yml", 203)] {
        store
            .save(&c, &ip(item), &sent(item, status, "x"), at(5))
            .unwrap();
    }
    let got: Vec<u16> = store
        .recent(std::slice::from_ref(&c), 10)
        .iter()
        .map(|r| r.send.status)
        .collect();
    assert_eq!(got, [203, 202, 201]);
}

// Waits on real I/O: the queue's worker runs on the test's background executor.
fn reply<T>(cx: &mut TestAppContext, rx: &async_channel::Receiver<T>) -> T {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        if let Ok(v) = rx.try_recv() {
            return v;
        }
        assert!(std::time::Instant::now() < deadline, "no reply");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[gpui_kit::test]
fn queued_reads_see_the_saves_queued_before_them(cx: &mut TestAppContext) {
    let data = tempfile::tempdir().unwrap();
    let store = ResponseStore::new(data.path().join("r"));
    cx.update(|cx| install(store.clone(), cx));
    let root = PathBuf::from("/c");
    let (htx, hrx) = async_channel::bounded(1);
    let (rtx, rrx) = async_channel::bounded(1);
    let (ltx, lrx) = async_channel::bounded(1);
    cx.update(|cx| {
        for (i, status) in [201u16, 202].into_iter().enumerate() {
            let save = StoreOp::Save {
                root: root.clone(),
                item: ip("a.yml"),
                result: Arc::new(sent("a.yml", status, "x")),
                secrets: Vec::new(),
                saved_at: at(i as u64 + 1),
                limits: Limits::default(),
                extra: SendExtra::default(),
                kept: None,
            };
            enqueue(save, cx);
        }
        let history = StoreOp::History {
            root: root.clone(),
            item: ip("a.yml"),
            reply: htx,
        };
        enqueue(history, cx);
        let recent = StoreOp::Recent {
            roots: vec![root.clone()],
            limit: 100,
            reply: rtx,
        };
        enqueue(recent, cx);
        let load = StoreOp::LoadSend {
            root: root.clone(),
            item: ip("a.yml"),
            seq: 0,
            reply: ltx,
        };
        enqueue(load, cx);
    });
    let h = reply(cx, &hrx);
    assert_eq!(h.iter().map(|s| s.status).collect::<Vec<_>>(), [202, 201]);
    assert_eq!(reply(cx, &rrx).len(), 2);
    let old = reply(cx, &lrx).expect("send 0");
    assert_eq!(old.result.response.unwrap().status(), 201);
}

#[gpui_kit::test]
fn without_a_queue_a_queued_read_gets_no_reply(cx: &mut TestAppContext) {
    let (tx, rx) = async_channel::bounded::<Vec<apinata_app::last_response::KeptSend>>(1);
    cx.update(|cx| {
        let op = StoreOp::History {
            root: PathBuf::from("/c"),
            item: ip("a.yml"),
            reply: tx,
        };
        enqueue(op, cx);
    });
    cx.run_until_parked();
    assert!(rx.try_recv().is_err());
    assert!(rx.is_closed(), "the sender was dropped");
}
