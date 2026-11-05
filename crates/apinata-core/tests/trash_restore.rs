//! Restoring the newest item a shell trashed, through the persisted trash log: round trip,
//! clashes, items gone from the bin, foreign and tampered records.

use crate::tree_support;

use apinata_core::{Trash, TrashError, TrashHandle, TrashLog, item_ops};
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use tempfile::TempDir;
use tree_support::{ROOT, disk_seq, exists, open, request, write};

/// A bin in its own temporary directory that records every handle it is asked to restore.
#[derive(Debug)]
struct FakeBin {
    dir: TempDir,
    next: Mutex<u32>,
    restored: Mutex<Vec<OsString>>,
}

impl FakeBin {
    fn new() -> FakeBin {
        FakeBin {
            dir: TempDir::new().expect("tempdir"),
            next: Mutex::new(0),
            restored: Mutex::new(Vec::new()),
        }
    }

    fn restored(&self) -> Vec<OsString> {
        self.restored.lock().expect("lock").clone()
    }
}

impl Trash for FakeBin {
    fn available(&self) -> bool {
        true
    }
    fn trash(&self, path: &Path) -> Result<TrashHandle, TrashError> {
        let mut n = self.next.lock().expect("lock");
        *n += 1;
        let id = OsString::from(format!("item{n}"));
        fs::rename(path, self.dir.path().join(&id))
            .map_err(|e| TrashError::Failed(e.to_string()))?;
        Ok(TrashHandle::new(path.to_path_buf(), id))
    }
    fn restore(&self, handle: &TrashHandle) -> Result<(), TrashError> {
        self.restored
            .lock()
            .expect("lock")
            .push(handle.id().clone());
        let stash = self.dir.path().join(handle.id());
        if !stash.exists() {
            return Err(TrashError::Missing);
        }
        fs::rename(stash, handle.original()).map_err(|e| TrashError::Failed(e.to_string()))
    }
}

fn three() -> (TempDir, apinata_core::Collection) {
    open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("b.yml", &request("b", 2)),
        ("c.yml", &request("c", 3)),
    ])
}

#[test]
fn a_trashed_item_comes_back_with_its_siblings_numbers_from_a_fresh_log() {
    let (dir, mut c) = three();
    let logs = TempDir::new().expect("tempdir");
    let bin = FakeBin::new();
    let log = TrashLog::new(logs.path());
    let warnings = item_ops::trash_item_logged(&mut c, "a.yml", &bin, Some(&log)).expect("rm");
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(disk_seq(&dir, "b.yml"), Some(1));

    // Another process: a new log over the same directory and a reopened collection.
    let mut c = apinata_core::Collection::open(dir.path()).expect("opens");
    let restored =
        item_ops::restore_last_trashed(&mut c, &bin, &TrashLog::new(logs.path())).expect("back");
    assert_eq!(restored.item().to_string(), "a.yml");
    assert!(restored.warnings().is_empty(), "{:?}", restored.warnings());
    assert!(exists(&dir, "a.yml"));
    assert_eq!(disk_seq(&dir, "b.yml"), Some(2));
    assert_eq!(disk_seq(&dir, "c.yml"), Some(3));

    let err = item_ops::restore_last_trashed(&mut c, &bin, &log).unwrap_err();
    assert_eq!(
        err.to_string(),
        "nothing deleted from this collection is left to restore"
    );
}

#[test]
fn two_deletes_come_back_newest_first() {
    let (dir, mut c) = three();
    let logs = TempDir::new().expect("tempdir");
    let (bin, log) = (FakeBin::new(), TrashLog::new(logs.path()));
    item_ops::trash_item_logged(&mut c, "a.yml", &bin, Some(&log)).expect("rm a");
    item_ops::trash_item_logged(&mut c, "c.yml", &bin, Some(&log)).expect("rm c");
    let first = item_ops::restore_last_trashed(&mut c, &bin, &log).expect("c back");
    assert_eq!(first.item().to_string(), "c.yml");
    let second = item_ops::restore_last_trashed(&mut c, &bin, &log).expect("a back");
    assert_eq!(second.item().to_string(), "a.yml");
    assert_eq!(
        [1, 2, 3].map(Some),
        ["a.yml", "b.yml", "c.yml"].map(|f| disk_seq(&dir, f))
    );
}

#[test]
fn a_clash_is_refused_and_keeps_the_entry_for_a_later_restore() {
    let (dir, mut c) = three();
    let logs = TempDir::new().expect("tempdir");
    let (bin, log) = (FakeBin::new(), TrashLog::new(logs.path()));
    item_ops::trash_item_logged(&mut c, "a.yml", &bin, Some(&log)).expect("rm");
    write(&dir, "a.yml", "someone else's file\n");
    let err = item_ops::restore_last_trashed(&mut c, &bin, &log).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("a.yml"), "{text}");
    assert!(text.contains("move or rename"), "{text}");
    assert!(!text.contains(&dir.path().display().to_string()), "{text}");
    assert_eq!(
        fs::read_to_string(dir.path().join("a.yml")).expect("read"),
        "someone else's file\n"
    );
    assert!(bin.restored().is_empty(), "the bin was never asked");

    fs::remove_file(dir.path().join("a.yml")).expect("rm clash");
    let mut c = apinata_core::Collection::open(dir.path()).expect("opens");
    let restored = item_ops::restore_last_trashed(&mut c, &bin, &log).expect("back");
    assert_eq!(restored.item().to_string(), "a.yml");
    assert!(exists(&dir, "a.yml"));
}

#[test]
fn an_item_gone_from_the_bin_is_dropped_and_the_next_restore_goes_further_back() {
    let (dir, mut c) = three();
    let logs = TempDir::new().expect("tempdir");
    let (bin, log) = (FakeBin::new(), TrashLog::new(logs.path()));
    item_ops::trash_item_logged(&mut c, "a.yml", &bin, Some(&log)).expect("rm a");
    item_ops::trash_item_logged(&mut c, "c.yml", &bin, Some(&log)).expect("rm c");
    // The user emptied c from the trash themselves.
    fs::remove_file(bin.dir.path().join("item2")).expect("empty");
    let err = item_ops::restore_last_trashed(&mut c, &bin, &log).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("c.yml"), "{text}");
    assert!(text.contains("no longer in the trash"), "{text}");
    let restored = item_ops::restore_last_trashed(&mut c, &bin, &log).expect("a back");
    assert_eq!(restored.item().to_string(), "a.yml");
    assert!(exists(&dir, "a.yml") && !exists(&dir, "c.yml"));
}

#[test]
fn a_log_written_for_another_collection_is_ignored() {
    let (_a_dir, mut a) = three();
    let (_b_dir, mut b) = three();
    let logs = TempDir::new().expect("tempdir");
    let (bin, log) = (FakeBin::new(), TrashLog::new(logs.path()));
    item_ops::trash_item_logged(&mut a, "a.yml", &bin, Some(&log)).expect("rm");
    fs::copy(log.file_for(a.root()), log.file_for(b.root())).expect("copy");
    let err = item_ops::restore_last_trashed(&mut b, &bin, &log).unwrap_err();
    assert_eq!(
        err.to_string(),
        "nothing deleted from this collection is left to restore"
    );
    assert!(bin.restored().is_empty());
}

#[test]
fn a_record_pointing_outside_the_collection_is_dropped_and_never_reaches_the_bin() {
    let (dir, mut c) = three();
    let elsewhere = TempDir::new().expect("tempdir");
    let logs = TempDir::new().expect("tempdir");
    let (bin, log) = (FakeBin::new(), TrashLog::new(logs.path()));
    item_ops::trash_item_logged(&mut c, "a.yml", &bin, Some(&log)).expect("rm");
    let file = log.file_for(c.root());
    let mut doc: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&file).expect("read")).expect("json");
    let outside = elsewhere.path().join("a.yml").display().to_string();
    doc["entries"][0]["original"] = serde_json::Value::String(outside);
    fs::write(&file, doc.to_string()).expect("tamper");

    let err = item_ops::restore_last_trashed(&mut c, &bin, &log).unwrap_err();
    assert!(err.to_string().contains("invalid"), "{err}");
    assert!(bin.restored().is_empty(), "{:?}", bin.restored());
    assert!(!exists(&dir, "a.yml") && !elsewhere.path().join("a.yml").exists());
    let again = item_ops::restore_last_trashed(&mut c, &bin, &log).unwrap_err();
    assert_eq!(
        again.to_string(),
        "nothing deleted from this collection is left to restore"
    );
}

#[test]
fn an_unreadable_log_is_named_and_left_alone() {
    let (_dir, mut c) = three();
    let logs = TempDir::new().expect("tempdir");
    let (bin, log) = (FakeBin::new(), TrashLog::new(logs.path()));
    let file = log.file_for(c.root());
    fs::create_dir_all(file.parent().expect("parent")).expect("mkdir");
    fs::write(&file, "not json").expect("write");
    let err = item_ops::restore_last_trashed(&mut c, &bin, &log).unwrap_err();
    assert!(
        err.to_string().contains(&file.display().to_string()),
        "{err}"
    );
    assert_eq!(fs::read_to_string(&file).expect("read"), "not json");
}

#[test]
fn deleting_without_a_log_warns_that_it_cannot_be_restored() {
    let (dir, mut c) = three();
    let bin = FakeBin::new();
    let warnings = item_ops::trash_item_logged(&mut c, "a.yml", &bin, None).expect("rm");
    assert_eq!(
        warnings,
        ["moved to the trash, but it cannot be restored from here: no data directory"]
    );
    assert!(!exists(&dir, "a.yml"));
}

#[test]
fn a_missing_parent_folder_is_refused_and_keeps_the_entry() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &tree_support::folder("users", 1)),
        ("users/get.yml", &request("get", 1)),
    ]);
    let logs = TempDir::new().expect("tempdir");
    let (bin, log) = (FakeBin::new(), TrashLog::new(logs.path()));
    item_ops::trash_item_logged(&mut c, "users/get.yml", &bin, Some(&log)).expect("rm");
    let before = fs::read_to_string(log.file_for(c.root())).expect("log");
    fs::remove_dir_all(dir.path().join("users")).expect("rm folder");
    let mut c = apinata_core::Collection::open(dir.path()).expect("opens");
    let err = item_ops::restore_last_trashed(&mut c, &bin, &log).unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains("users/get.yml") && text.contains("its folder users no longer exists"),
        "{text}"
    );
    assert_eq!(
        fs::read_to_string(log.file_for(c.root())).expect("log"),
        before
    );
    assert!(bin.restored().is_empty(), "{:?}", bin.restored());
    assert!(!exists(&dir, "users"));
}

/// The item paths the record for `c` holds, oldest first.
fn recorded(log: &TrashLog, c: &apinata_core::Collection) -> Vec<String> {
    let text = fs::read_to_string(log.file_for(c.root())).expect("log");
    let doc: serde_json::Value = serde_json::from_str(&text).expect("json");
    doc["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .map(|e| e["item"].as_str().expect("item").to_owned())
        .collect()
}

#[test]
fn deletes_recorded_at_once_from_separate_processes_all_survive() {
    const N: usize = 12;
    let mut files = vec![("opencollection.yml".to_owned(), ROOT.to_owned())];
    for i in 0..N {
        files.push((
            format!("f{i}/folder.yml"),
            tree_support::folder(&format!("f{i}"), 1),
        ));
        files.push((format!("f{i}/r.yml"), request("r", 1)));
    }
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(p, t)| (p.as_str(), t.as_str()))
        .collect();
    let (dir, c) = open(&refs);
    let logs = TempDir::new().expect("tempdir");
    let bin = FakeBin::new();
    let start = std::sync::Barrier::new(N);
    // Each thread stands for a process: its own collection and its own log value.
    std::thread::scope(|s| {
        for i in 0..N {
            let (dir, logs, bin, start) = (&dir, &logs, &bin, &start);
            s.spawn(move || {
                let mut c = apinata_core::Collection::open(dir.path()).expect("opens");
                let log = TrashLog::new(logs.path());
                start.wait();
                let item = format!("f{i}/r.yml");
                let warnings =
                    item_ops::trash_item_logged(&mut c, &item, bin, Some(&log)).expect("rm");
                assert!(warnings.is_empty(), "{warnings:?}");
            });
        }
    });
    let mut items = recorded(&TrashLog::new(logs.path()), &c);
    items.sort();
    let mut want: Vec<String> = (0..N).map(|i| format!("f{i}/r.yml")).collect();
    want.sort();
    assert_eq!(items, want);
}

/// A bin whose restore, once it starts, lets `started` know and then waits up to half a second
/// for `done`, so another process can act in the middle of a restore.
struct PausingBin {
    inner: FakeBin,
    started: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    done: Mutex<std::sync::mpsc::Receiver<()>>,
}

impl std::fmt::Debug for PausingBin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PausingBin")
    }
}

impl Trash for PausingBin {
    fn available(&self) -> bool {
        true
    }
    fn trash(&self, path: &Path) -> Result<TrashHandle, TrashError> {
        self.inner.trash(path)
    }
    fn restore(&self, handle: &TrashHandle) -> Result<(), TrashError> {
        if let Some(started) = self.started.lock().expect("lock").take() {
            let _ = started.send(());
            let _ = self
                .done
                .lock()
                .expect("lock")
                .recv_timeout(std::time::Duration::from_millis(500));
        }
        self.inner.restore(handle)
    }
}

#[test]
fn a_delete_recorded_while_a_restore_runs_is_kept() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("other/folder.yml", &tree_support::folder("other", 2)),
        ("other/x.yml", &request("x", 1)),
    ]);
    let logs = TempDir::new().expect("tempdir");
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let bin = PausingBin {
        inner: FakeBin::new(),
        started: Mutex::new(None),
        done: Mutex::new(done_rx),
    };
    let log = TrashLog::new(logs.path());
    item_ops::trash_item_logged(&mut c, "a.yml", &bin, Some(&log)).expect("rm a");
    *bin.started.lock().expect("lock") = Some(started_tx);
    std::thread::scope(|s| {
        let (dir, logs, bin) = (&dir, &logs, &bin);
        s.spawn(move || {
            started_rx.recv().expect("restore started");
            let mut other = apinata_core::Collection::open(dir.path()).expect("opens");
            let log = TrashLog::new(logs.path());
            item_ops::trash_item_logged(&mut other, "other/x.yml", bin, Some(&log)).expect("rm");
            let _ = done_tx.send(());
        });
        let restored = item_ops::restore_last_trashed(&mut c, bin, &log).expect("a back");
        assert_eq!(restored.item().to_string(), "a.yml");
    });
    assert_eq!(recorded(&log, &c), ["other/x.yml"]);
    let mut c = apinata_core::Collection::open(dir.path()).expect("opens");
    let again = item_ops::restore_last_trashed(&mut c, &bin, &log).expect("x back");
    assert_eq!(again.item().to_string(), "other/x.yml");
    assert!(exists(&dir, "other/x.yml"));
}

#[test]
fn a_record_keeps_its_paths_as_plain_strings() {
    let (_dir, mut c) = three();
    let logs = TempDir::new().expect("tempdir");
    let (bin, log) = (FakeBin::new(), TrashLog::new(logs.path()));
    item_ops::trash_item_logged(&mut c, "a.yml", &bin, Some(&log)).expect("rm");
    let text = fs::read_to_string(log.file_for(c.root())).expect("log");
    let doc: serde_json::Value = serde_json::from_str(&text).expect("json");
    assert!(doc["root"].is_string(), "{doc}");
    assert!(doc["entries"][0]["original"].is_string(), "{doc}");
    assert_eq!(doc["entries"][0]["id"], "item1", "{doc}");
}

#[test]
fn a_record_written_by_an_earlier_version_still_restores() {
    let (dir, mut c) = three();
    let logs = TempDir::new().expect("tempdir");
    let (bin, log) = (FakeBin::new(), TrashLog::new(logs.path()));
    item_ops::trash_item_logged(&mut c, "c.yml", &bin, Some(&log)).expect("rm");
    let canon = c.root().canonicalize().expect("canon");
    let record = serde_json::json!({
        "root": canon.to_str().expect("utf-8"),
        "entries": [{
            "item": "c.yml",
            "original": c.root().join("c.yml").to_str().expect("utf-8"),
            "id": "item1",
            "renumbered": [],
        }],
    });
    fs::write(log.file_for(c.root()), record.to_string()).expect("write");
    let restored = item_ops::restore_last_trashed(&mut c, &bin, &log).expect("back");
    assert_eq!(restored.item().to_string(), "c.yml");
    assert!(exists(&dir, "c.yml"));
}

#[cfg(unix)]
#[test]
fn a_collection_whose_path_is_not_utf8_restores_through_the_record() {
    use std::os::unix::ffi::OsStrExt;
    let outer = TempDir::new().expect("tempdir");
    let root = outer.path().join(std::ffi::OsStr::from_bytes(b"col\xff"));
    fs::create_dir(&root).expect("mkdir");
    fs::write(root.join("opencollection.yml"), ROOT).expect("write");
    fs::write(root.join("a.yml"), request("a", 1)).expect("write");
    fs::write(root.join("b.yml"), request("b", 2)).expect("write");
    let mut c = apinata_core::Collection::open(&root).expect("opens");
    let logs = TempDir::new().expect("tempdir");
    let (bin, log) = (FakeBin::new(), TrashLog::new(logs.path()));
    let warnings = item_ops::trash_item_logged(&mut c, "a.yml", &bin, Some(&log)).expect("rm");
    assert!(warnings.is_empty(), "{warnings:?}");
    let mut c = apinata_core::Collection::open(&root).expect("opens");
    let restored = item_ops::restore_last_trashed(&mut c, &bin, &log).expect("back");
    assert_eq!(restored.item().to_string(), "a.yml");
    assert!(root.join("a.yml").is_file());
}

#[test]
fn the_first_restore_creates_the_log_directory_and_takes_its_lock() {
    let (_dir, mut c) = three();
    let logs = TempDir::new().expect("tempdir");
    let log = TrashLog::new(logs.path().join("not-yet"));
    let bin = FakeBin::new();
    let err = item_ops::restore_last_trashed(&mut c, &bin, &log).unwrap_err();
    assert_eq!(
        err.to_string(),
        "nothing deleted from this collection is left to restore"
    );
    let mut lock = log.file_for(c.root()).into_os_string();
    lock.push(".lock");
    assert!(Path::new(&lock).is_file(), "{lock:?}");
}

fn four() -> (TempDir, apinata_core::Collection) {
    open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("b.yml", &request("b", 2)),
        ("c.yml", &request("c", 3)),
        ("d.yml", &request("d", 4)),
    ])
}

#[test]
fn two_open_collections_deleting_in_one_folder_both_renumber_cleanly() {
    let (dir, mut first) = four();
    let mut second = apinata_core::Collection::open(dir.path()).expect("opens");
    let logs = TempDir::new().expect("tempdir");
    let bin = FakeBin::new();
    let (one, two) = (TrashLog::new(logs.path()), TrashLog::new(logs.path()));
    let a = item_ops::trash_item_logged(&mut first, "a.yml", &bin, Some(&one)).expect("rm a");
    assert!(a.is_empty(), "{a:?}");
    let b = item_ops::trash_item_logged(&mut second, "b.yml", &bin, Some(&two)).expect("rm b");
    assert!(b.is_empty(), "{b:?}");
    assert_eq!(
        [disk_seq(&dir, "c.yml"), disk_seq(&dir, "d.yml")],
        [Some(1), Some(2)]
    );
}

#[test]
fn deletes_at_once_in_one_folder_both_renumber_cleanly() {
    for round in 0..8 {
        let (dir, _) = four();
        let logs = TempDir::new().expect("tempdir");
        let bin = FakeBin::new();
        let start = std::sync::Barrier::new(2);
        std::thread::scope(|s| {
            for item in ["a.yml", "b.yml"] {
                let (dir, logs, bin, start) = (&dir, &logs, &bin, &start);
                s.spawn(move || {
                    let mut c = apinata_core::Collection::open(dir.path()).expect("opens");
                    let log = TrashLog::new(logs.path());
                    start.wait();
                    let warnings = item_ops::trash_item_logged(&mut c, item, bin, Some(&log))
                        .unwrap_or_else(|e| panic!("round {round}, rm {item}: {e}"));
                    assert!(warnings.is_empty(), "round {round}, {item}: {warnings:?}");
                });
            }
        });
        assert_eq!(
            [disk_seq(&dir, "c.yml"), disk_seq(&dir, "d.yml")],
            [Some(1), Some(2)],
            "round {round}"
        );
    }
}

/// Put the entries at `indices` of the record into one prune, as a run that was interleaved
/// with a single delete would have left it.
fn in_batch(log: &TrashLog, c: &apinata_core::Collection, batch: &str, indices: &[usize]) {
    let file = log.file_for(c.root());
    let mut doc: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&file).expect("read")).expect("json");
    for &i in indices {
        doc["entries"][i]["batch"] = batch.into();
    }
    fs::write(&file, doc.to_string()).expect("write");
}

/// The seqs on disk of `files`, in the order given.
fn seqs<const N: usize>(dir: &TempDir, files: [&str; N]) -> [Option<u64>; N] {
    files.map(|f| disk_seq(dir, f))
}

#[test]
fn a_single_restore_after_a_prune_goes_before_its_tail() {
    // The single delete sits between the prune's two entries, and its tail holds only c.
    let (dir, mut c) = three();
    let logs = TempDir::new().expect("tempdir");
    let (bin, log) = (FakeBin::new(), TrashLog::new(logs.path()));
    for item in ["b.yml", "a.yml", "c.yml"] {
        item_ops::trash_item_logged(&mut c, item, &bin, Some(&log)).expect("rm");
    }
    in_batch(&log, &c, "run", &[0, 2]);
    item_ops::restore_prune(&mut c, &bin, &log, "run").expect("prune back");
    let restored = item_ops::restore_last_trashed(&mut c, &bin, &log).expect("a back");
    assert!(restored.warnings().is_empty(), "{:?}", restored.warnings());
    assert_eq!(seqs(&dir, ["b.yml", "a.yml", "c.yml"]), [1, 2, 3].map(Some));
}

#[test]
fn a_single_restore_after_a_prune_keeps_the_old_order_when_its_tail_allows() {
    let (dir, mut c) = three();
    let logs = TempDir::new().expect("tempdir");
    let (bin, log) = (FakeBin::new(), TrashLog::new(logs.path()));
    for item in ["c.yml", "a.yml", "b.yml"] {
        item_ops::trash_item_logged(&mut c, item, &bin, Some(&log)).expect("rm");
    }
    in_batch(&log, &c, "run", &[0, 2]);
    item_ops::restore_prune(&mut c, &bin, &log, "run").expect("prune back");
    let restored = item_ops::restore_last_trashed(&mut c, &bin, &log).expect("a back");
    assert!(restored.warnings().is_empty(), "{:?}", restored.warnings());
    assert_eq!(seqs(&dir, ["a.yml", "b.yml", "c.yml"]), [1, 2, 3].map(Some));
}

#[test]
fn siblings_renumbered_since_the_delete_keep_their_relative_order() {
    let (dir, mut c) = four();
    let logs = TempDir::new().expect("tempdir");
    let (bin, log) = (FakeBin::new(), TrashLog::new(logs.path()));
    item_ops::trash_item_logged(&mut c, "a.yml", &bin, Some(&log)).expect("rm");
    // Written behind the open collection's back: d now leads, and b and c left a gap.
    write(&dir, "d.yml", &request("d", 2));
    write(&dir, "b.yml", &request("b", 5));
    write(&dir, "c.yml", &request("c", 6));
    let restored = item_ops::restore_last_trashed(&mut c, &bin, &log).expect("a back");
    assert!(restored.warnings().is_empty(), "{:?}", restored.warnings());
    assert_eq!(
        seqs(&dir, ["d.yml", "a.yml", "b.yml", "c.yml"]),
        [1, 2, 3, 4].map(Some)
    );
}

#[test]
fn an_item_whose_tail_is_gone_goes_last() {
    let (dir, mut c) = three();
    let logs = TempDir::new().expect("tempdir");
    let (bin, log) = (FakeBin::new(), TrashLog::new(logs.path()));
    item_ops::trash_item_logged(&mut c, "a.yml", &bin, Some(&log)).expect("rm");
    fs::remove_file(dir.path().join("b.yml")).expect("rm b");
    fs::remove_file(dir.path().join("c.yml")).expect("rm c");
    write(&dir, "d.yml", &request("d", 1));
    write(&dir, "e.yml", &request("e", 2));
    let restored = item_ops::restore_last_trashed(&mut c, &bin, &log).expect("a back");
    assert!(restored.warnings().is_empty(), "{:?}", restored.warnings());
    assert_eq!(seqs(&dir, ["d.yml", "e.yml", "a.yml"]), [1, 2, 3].map(Some));
}

/// A bin whose restore also edits `sibling` on disk, so the renumber finds it not as loaded.
#[derive(Debug)]
struct EditingBin {
    inner: FakeBin,
    sibling: std::path::PathBuf,
}

impl Trash for EditingBin {
    fn available(&self) -> bool {
        true
    }
    fn trash(&self, path: &Path) -> Result<TrashHandle, TrashError> {
        self.inner.trash(path)
    }
    fn restore(&self, handle: &TrashHandle) -> Result<(), TrashError> {
        let text = fs::read_to_string(&self.sibling).expect("read");
        fs::write(&self.sibling, format!("{text}# edited\n")).expect("write");
        self.inner.restore(handle)
    }
}

#[test]
fn a_sibling_that_cannot_be_renumbered_is_a_warning_with_the_item_back() {
    let (dir, mut c) = three();
    let logs = TempDir::new().expect("tempdir");
    let log = TrashLog::new(logs.path());
    let bin = EditingBin {
        inner: FakeBin::new(),
        sibling: dir.path().join("c.yml"),
    };
    item_ops::trash_item_logged(&mut c, "a.yml", &bin, Some(&log)).expect("rm");
    let restored = item_ops::restore_last_trashed(&mut c, &bin, &log).expect("a back");
    assert_eq!(restored.item().to_string(), "a.yml");
    let warning = restored.warnings().join("\n");
    assert!(
        warning.starts_with("restored, but could not renumber the siblings: "),
        "{warning}"
    );
    assert!(exists(&dir, "a.yml"));
    assert!(recorded(&log, &c).is_empty(), "the record is dropped");
}

#[test]
fn a_clash_leaves_the_siblings_numbers_and_the_entry() {
    let (dir, mut c) = three();
    let logs = TempDir::new().expect("tempdir");
    let (bin, log) = (FakeBin::new(), TrashLog::new(logs.path()));
    item_ops::trash_item_logged(&mut c, "a.yml", &bin, Some(&log)).expect("rm");
    write(&dir, "a.yml", &request("mine", 9));
    let err = item_ops::restore_last_trashed(&mut c, &bin, &log).unwrap_err();
    assert!(err.to_string().contains("move or rename"), "{err}");
    assert_eq!(seqs(&dir, ["b.yml", "c.yml"]), [1, 2].map(Some));
    assert_eq!(recorded(&log, &c), ["a.yml"]);
}

/// What the record holds for the newest delete's tail, as (path, old, new).
fn recorded_tail(log: &TrashLog, c: &apinata_core::Collection) -> Vec<(String, u64, u64)> {
    let text = fs::read_to_string(log.file_for(c.root())).expect("log");
    let doc: serde_json::Value = serde_json::from_str(&text).expect("json");
    let entries = doc["entries"].as_array().expect("entries");
    let tail = entries.last().expect("an entry")["renumbered"]
        .as_array()
        .expect("renumbered");
    tail.iter()
        .map(|r| {
            let seq = |key: &str| r[key].as_u64().expect("a seq");
            (
                r["path"].as_str().expect("path").to_owned(),
                seq("old"),
                seq("new"),
            )
        })
        .collect()
}

#[test]
fn a_record_holds_the_siblings_after_the_item_changed_or_not() {
    let (_dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 2)),
        ("b.yml", &request("b", 3)),
        ("c.yml", &request("c", 3)),
        ("d.yml", &request("d", 4)),
    ]);
    let logs = TempDir::new().expect("tempdir");
    let (bin, log) = (FakeBin::new(), TrashLog::new(logs.path()));
    item_ops::trash_item_logged(&mut c, "b.yml", &bin, Some(&log)).expect("rm");
    assert_eq!(
        recorded_tail(&log, &c),
        [("c.yml".to_owned(), 3, 2), ("d.yml".to_owned(), 4, 3)]
    );
}

#[test]
fn a_tied_sibling_that_keeps_its_number_is_in_the_record() {
    let (_dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("b.yml", &request("b", 1)),
    ]);
    let logs = TempDir::new().expect("tempdir");
    let (bin, log) = (FakeBin::new(), TrashLog::new(logs.path()));
    item_ops::trash_item_logged(&mut c, "a.yml", &bin, Some(&log)).expect("rm");
    assert_eq!(recorded_tail(&log, &c), [("b.yml".to_owned(), 1, 1)]);
}

#[test]
fn an_item_outside_the_numbered_list_has_no_tail() {
    let (_dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("z.yml", &tree_support::item("app", "z", 2)),
        ("b.yml", &request("b", 3)),
    ]);
    let logs = TempDir::new().expect("tempdir");
    let (bin, log) = (FakeBin::new(), TrashLog::new(logs.path()));
    item_ops::trash_item_logged(&mut c, "z.yml", &bin, Some(&log)).expect("rm");
    assert_eq!(recorded_tail(&log, &c), []);
}

#[test]
fn the_last_item_of_a_gapped_folder_comes_back_last() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 2)),
        ("b.yml", &request("b", 3)),
        ("c.yml", &request("c", 4)),
    ]);
    let logs = TempDir::new().expect("tempdir");
    let (bin, log) = (FakeBin::new(), TrashLog::new(logs.path()));
    item_ops::trash_item_logged(&mut c, "c.yml", &bin, Some(&log)).expect("rm");
    item_ops::restore_last_trashed(&mut c, &bin, &log).expect("back");
    assert_eq!(seqs(&dir, ["a.yml", "b.yml", "c.yml"]), [1, 2, 3].map(Some));
}

#[test]
fn an_earlier_record_naming_siblings_before_the_item_still_puts_it_after_them() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 2)),
        ("b.yml", &request("b", 3)),
        ("c.yml", &request("c", 4)),
    ]);
    let logs = TempDir::new().expect("tempdir");
    let (bin, log) = (FakeBin::new(), TrashLog::new(logs.path()));
    item_ops::trash_item_logged(&mut c, "c.yml", &bin, Some(&log)).expect("rm");
    let file = log.file_for(c.root());
    let mut doc: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&file).expect("read")).expect("json");
    doc["entries"][0]["renumbered"] = serde_json::json!([
        {"path": "a.yml", "old": 2, "new": 1},
        {"path": "b.yml", "old": 3, "new": 2},
    ]);
    fs::write(&file, doc.to_string()).expect("write");
    item_ops::restore_last_trashed(&mut c, &bin, &log).expect("back");
    assert_eq!(seqs(&dir, ["a.yml", "b.yml", "c.yml"]), [1, 2, 3].map(Some));
}

#[test]
fn an_item_tied_with_the_next_comes_back_before_it() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("b.yml", &request("b", 1)),
    ]);
    let logs = TempDir::new().expect("tempdir");
    let (bin, log) = (FakeBin::new(), TrashLog::new(logs.path()));
    item_ops::trash_item_logged(&mut c, "a.yml", &bin, Some(&log)).expect("rm");
    item_ops::restore_last_trashed(&mut c, &bin, &log).expect("back");
    assert_eq!(seqs(&dir, ["a.yml", "b.yml"]), [1, 2].map(Some));
}

#[test]
fn an_item_tied_with_the_one_before_comes_back_after_it() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("b.yml", &request("b", 1)),
    ]);
    let logs = TempDir::new().expect("tempdir");
    let (bin, log) = (FakeBin::new(), TrashLog::new(logs.path()));
    item_ops::trash_item_logged(&mut c, "b.yml", &bin, Some(&log)).expect("rm");
    item_ops::restore_last_trashed(&mut c, &bin, &log).expect("back");
    assert_eq!(seqs(&dir, ["a.yml", "b.yml"]), [1, 2].map(Some));
}

#[test]
fn folders_without_seqs_keep_a_restored_folder_in_place() {
    let bare = |name: &str| tree_support::folder(name, 1).replace("  seq: 1\n", "");
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("x/folder.yml", &bare("x")),
        ("y/folder.yml", &bare("y")),
        ("z/folder.yml", &bare("z")),
    ]);
    let logs = TempDir::new().expect("tempdir");
    let (bin, log) = (FakeBin::new(), TrashLog::new(logs.path()));
    item_ops::trash_item_logged(&mut c, "y", &bin, Some(&log)).expect("rm");
    item_ops::restore_last_trashed(&mut c, &bin, &log).expect("back");
    assert_eq!(seqs(&dir, ["x", "y", "z"]), [1, 2, 3].map(Some));
}

/// A folder of `items` as (file, seq), the file `z.yml` an app item and the rest requests.
fn with_app(items: &[(&str, u64)]) -> (TempDir, apinata_core::Collection) {
    let text: Vec<(String, String)> = items
        .iter()
        .map(|&(file, seq)| {
            let name = file.trim_end_matches(".yml");
            let body = match file {
                "z.yml" => tree_support::item("app", name, seq),
                _ => request(name, seq),
            };
            (file.to_owned(), body)
        })
        .collect();
    let mut files = vec![("opencollection.yml", ROOT)];
    files.extend(text.iter().map(|(f, t)| (f.as_str(), t.as_str())));
    open(&files)
}

/// Trash `item` and undo it, proving the app item's file came through untouched.
fn trash_and_undo(dir: &TempDir, c: &mut apinata_core::Collection, item: &str) {
    let app = fs::read(dir.path().join("z.yml")).expect("read");
    let logs = TempDir::new().expect("tempdir");
    let (bin, log) = (FakeBin::new(), TrashLog::new(logs.path()));
    item_ops::trash_item_logged(c, item, &bin, Some(&log)).expect("rm");
    let restored = item_ops::restore_last_trashed(c, &bin, &log).expect("back");
    assert!(restored.warnings().is_empty(), "{:?}", restored.warnings());
    assert_eq!(fs::read(dir.path().join("z.yml")).expect("read"), app);
}

#[test]
fn a_restore_skips_the_number_an_app_item_holds() {
    let (dir, mut c) = with_app(&[("a.yml", 1), ("z.yml", 2), ("b.yml", 3)]);
    trash_and_undo(&dir, &mut c, "a.yml");
    assert_eq!(seqs(&dir, ["a.yml", "z.yml", "b.yml"]), [1, 2, 3].map(Some));
}

#[test]
fn a_restore_skips_a_held_number_past_the_first_siblings() {
    let (dir, mut c) = with_app(&[("a.yml", 1), ("b.yml", 2), ("z.yml", 3), ("c.yml", 4)]);
    trash_and_undo(&dir, &mut c, "a.yml");
    assert_eq!(
        seqs(&dir, ["a.yml", "b.yml", "z.yml", "c.yml"]),
        [1, 2, 3, 4].map(Some)
    );
}

#[test]
fn an_app_item_numbered_far_above_the_folder_changes_nothing() {
    let (dir, mut c) = with_app(&[("a.yml", 1), ("b.yml", 2), ("z.yml", 10)]);
    trash_and_undo(&dir, &mut c, "a.yml");
    assert_eq!(
        seqs(&dir, ["a.yml", "b.yml", "z.yml"]),
        [Some(1), Some(2), Some(10)]
    );
}

#[test]
fn a_batch_restore_skips_the_number_an_app_item_holds() {
    let (dir, mut c) = with_app(&[("a.yml", 1), ("z.yml", 2), ("b.yml", 3)]);
    let logs = TempDir::new().expect("tempdir");
    let (bin, log) = (FakeBin::new(), TrashLog::new(logs.path()));
    item_ops::trash_item_logged(&mut c, "a.yml", &bin, Some(&log)).expect("rm");
    in_batch(&log, &c, "run", &[0]);
    let all = item_ops::restore_prune(&mut c, &bin, &log, "run").expect("back");
    assert!(all.warnings().is_empty(), "{:?}", all.warnings());
    assert_eq!(seqs(&dir, ["a.yml", "z.yml", "b.yml"]), [1, 2, 3].map(Some));
}

#[test]
fn a_seqless_app_item_holds_no_number() {
    let app = tree_support::item("app", "z", 1).replace("  seq: 1\n", "");
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("b.yml", &request("b", 2)),
        ("z.yml", &app),
    ]);
    trash_and_undo(&dir, &mut c, "a.yml");
    assert_eq!(seqs(&dir, ["a.yml", "b.yml"]), [1, 2].map(Some));
}

#[test]
fn a_sibling_tied_with_an_app_item_before_the_restored_one_keeps_its_number() {
    let (dir, mut c) = with_app(&[("a.yml", 1), ("z.yml", 1), ("b.yml", 2)]);
    trash_and_undo(&dir, &mut c, "b.yml");
    assert_eq!(seqs(&dir, ["a.yml", "b.yml", "z.yml"]), [1, 2, 1].map(Some));
}

#[test]
fn a_restored_item_tied_with_an_app_item_keeps_its_number() {
    let (dir, mut c) = with_app(&[("a.yml", 1), ("z.yml", 1), ("b.yml", 2)]);
    trash_and_undo(&dir, &mut c, "a.yml");
    assert_eq!(seqs(&dir, ["a.yml", "b.yml", "z.yml"]), [1, 2, 1].map(Some));
}

#[test]
fn a_tie_the_delete_broke_comes_back_through_the_record() {
    let (dir, mut c) = with_app(&[("x.yml", 1), ("a.yml", 2), ("z.yml", 2)]);
    trash_and_undo(&dir, &mut c, "x.yml");
    assert_eq!(seqs(&dir, ["x.yml", "a.yml", "z.yml"]), [1, 2, 2].map(Some));
}

#[test]
fn a_batch_restore_keeps_a_tie_with_an_app_item() {
    let (dir, mut c) = with_app(&[("x.yml", 1), ("a.yml", 2), ("z.yml", 2)]);
    let logs = TempDir::new().expect("tempdir");
    let (bin, log) = (FakeBin::new(), TrashLog::new(logs.path()));
    item_ops::trash_item_logged(&mut c, "x.yml", &bin, Some(&log)).expect("rm");
    in_batch(&log, &c, "run", &[0]);
    let all = item_ops::restore_prune(&mut c, &bin, &log, "run").expect("back");
    assert!(all.warnings().is_empty(), "{:?}", all.warnings());
    assert_eq!(seqs(&dir, ["x.yml", "a.yml", "z.yml"]), [1, 2, 2].map(Some));
}

#[test]
fn a_folder_stored_with_seq_zero_after_the_item_keeps_the_item_in_place() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("f/folder.yml", &tree_support::folder("f", 0)),
        ("g/folder.yml", &tree_support::folder("g", 0)),
    ]);
    let logs = TempDir::new().expect("tempdir");
    let (bin, log) = (FakeBin::new(), TrashLog::new(logs.path()));
    item_ops::trash_item_logged(&mut c, "a.yml", &bin, Some(&log)).expect("rm");
    assert_eq!(
        recorded_tail(&log, &c),
        [("f".to_owned(), 2, 1), ("g".to_owned(), 3, 2)]
    );
    item_ops::restore_last_trashed(&mut c, &bin, &log).expect("back");
    assert_eq!(seqs(&dir, ["a.yml", "f", "g"]), [1, 2, 3].map(Some));
}

#[test]
fn a_sibling_the_delete_moved_onto_a_held_number_goes_back_past_it() {
    let (dir, mut c) = with_app(&[("x.yml", 1), ("z.yml", 2), ("y.yml", 3), ("w.yml", 4)]);
    trash_and_undo(&dir, &mut c, "w.yml");
    assert_eq!(
        seqs(&dir, ["x.yml", "z.yml", "y.yml", "w.yml"]),
        [1, 2, 3, 4].map(Some)
    );
}

#[test]
fn a_sibling_ahead_and_tied_with_the_item_stays_ahead_of_it() {
    let (dir, mut c) = with_app(&[("x.yml", 1), ("z.yml", 2), ("a.yml", 3), ("b.yml", 3)]);
    trash_and_undo(&dir, &mut c, "b.yml");
    assert_eq!(
        seqs(&dir, ["x.yml", "z.yml", "a.yml", "b.yml"]),
        [1, 2, 3, 4].map(Some)
    );
}

#[test]
fn undoing_an_app_item_gives_back_the_numbers_its_delete_took() {
    let (dir, mut c) = with_app(&[("x.yml", 1), ("z.yml", 2), ("y.yml", 3), ("w.yml", 4)]);
    trash_and_undo(&dir, &mut c, "z.yml");
    assert_eq!(
        seqs(&dir, ["x.yml", "z.yml", "y.yml", "w.yml"]),
        [1, 2, 3, 4].map(Some)
    );
}

#[test]
fn a_sibling_ahead_of_the_item_gets_its_recorded_tie_back() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("x.yml", &request("x", 1)),
        ("z.yml", &tree_support::item("app", "z", 2)),
        ("y.yml", &request("y", 3)),
        ("q.yml", &tree_support::item("app", "q", 3)),
        ("w.yml", &request("w", 4)),
    ]);
    trash_and_undo(&dir, &mut c, "w.yml");
    assert_eq!(
        seqs(&dir, ["x.yml", "z.yml", "y.yml", "q.yml", "w.yml"]),
        [1, 2, 3, 3, 4].map(Some)
    );
}

#[test]
fn a_batch_restore_moves_a_sibling_ahead_of_the_item_back_past_a_held_number() {
    let (dir, mut c) = with_app(&[("x.yml", 1), ("z.yml", 2), ("y.yml", 3), ("w.yml", 4)]);
    let logs = TempDir::new().expect("tempdir");
    let (bin, log) = (FakeBin::new(), TrashLog::new(logs.path()));
    item_ops::trash_item_logged(&mut c, "w.yml", &bin, Some(&log)).expect("rm");
    in_batch(&log, &c, "run", &[0]);
    let all = item_ops::restore_prune(&mut c, &bin, &log, "run").expect("back");
    assert!(all.warnings().is_empty(), "{:?}", all.warnings());
    assert_eq!(
        seqs(&dir, ["x.yml", "z.yml", "y.yml", "w.yml"]),
        [1, 2, 3, 4].map(Some)
    );
}

#[test]
fn a_seqless_app_item_never_pushes_a_gapped_folder_off_one() {
    let app = tree_support::item("app", "z", 1).replace("  seq: 1\n", "");
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 2)),
        ("b.yml", &request("b", 3)),
        ("z.yml", &app),
    ]);
    trash_and_undo(&dir, &mut c, "a.yml");
    assert_eq!(seqs(&dir, ["a.yml", "b.yml"]), [1, 2].map(Some));
}

#[test]
fn a_batch_restore_gives_a_chained_sibling_the_number_it_had_first() {
    // Deleting c moves b onto the app item's 2; deleting a then moves it off again.
    let (dir, mut c) = with_app(&[("a.yml", 1), ("b.yml", 3), ("c.yml", 4), ("z.yml", 2)]);
    let logs = TempDir::new().expect("tempdir");
    let (bin, log) = (FakeBin::new(), TrashLog::new(logs.path()));
    for item in ["c.yml", "a.yml"] {
        item_ops::trash_item_logged(&mut c, item, &bin, Some(&log)).expect("rm");
    }
    in_batch(&log, &c, "run", &[0, 1]);
    let all = item_ops::restore_prune(&mut c, &bin, &log, "run").expect("back");
    assert!(all.warnings().is_empty(), "{:?}", all.warnings());
    assert_eq!(seqs(&dir, ["a.yml", "b.yml", "c.yml"]), [1, 3, 4].map(Some));
}

/// A request stored as `line`, or with no seq at all when `line` is empty.
fn request_with(name: &str, line: &str) -> String {
    request(name, 1).replace("  seq: 1\n", line)
}

#[test]
fn a_seqless_request_is_no_tie_with_an_app_item_on_one() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request_with("a", "")),
        ("b.yml", &request("b", 6)),
        ("z.yml", &tree_support::item("app", "z", 1)),
    ]);
    trash_and_undo(&dir, &mut c, "b.yml");
    assert_eq!(seqs(&dir, ["a.yml", "b.yml"]), [2, 3].map(Some));
}

#[test]
fn a_request_stored_with_seq_zero_is_no_tie_with_an_app_item_on_one() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request_with("a", "  seq: 0\n")),
        ("b.yml", &request("b", 5)),
        ("z.yml", &tree_support::item("app", "z", 1)),
    ]);
    trash_and_undo(&dir, &mut c, "a.yml");
    assert_eq!(seqs(&dir, ["a.yml", "b.yml"]), [2, 3].map(Some));
}

/// One sibling of a table case: `a1` a request, `a/1` a folder, `z!1` an app item, each with
/// `-` for no stored seq.
fn table_item(token: &str) -> (String, String) {
    let (name, rest) = token.split_at(1);
    let (kind, number) = rest.split_at(
        rest.find(['-', '0', '1', '2', '3', '4', '5', '6'])
            .expect("seq"),
    );
    let line = match number {
        "-" => String::new(),
        n => format!("  seq: {n}\n"),
    };
    let (file, text) = match kind {
        "/" => (format!("{name}/folder.yml"), tree_support::folder(name, 1)),
        "!" => (format!("{name}.yml"), tree_support::item("app", name, 1)),
        _ => (format!("{name}.yml"), request(name, 1)),
    };
    (file, text.replace("  seq: 1\n", &line))
}

/// Delete `deleted` in order and undo them all, one at a time or as a batch, then give the
/// folder back in the notation of its case.
fn after_undo(start: &str, deleted: &str, batch: bool) -> String {
    let tokens: Vec<&str> = start.split(' ').collect();
    let bodies: Vec<(String, String)> = tokens.iter().map(|t| table_item(t)).collect();
    let mut files = vec![("opencollection.yml", ROOT)];
    files.extend(bodies.iter().map(|(f, t)| (f.as_str(), t.as_str())));
    let (dir, mut c) = open(&files);
    let logs = TempDir::new().expect("tempdir");
    let (bin, log) = (FakeBin::new(), TrashLog::new(logs.path()));
    let item = |name: &str| {
        let token = tokens.iter().find(|t| t.starts_with(name)).expect("listed");
        if token.contains('/') {
            name.to_owned()
        } else {
            format!("{name}.yml")
        }
    };
    let deleted: Vec<&str> = deleted.split(' ').collect();
    for name in &deleted {
        item_ops::trash_item_logged(&mut c, &item(name), &bin, Some(&log)).expect("rm");
    }
    if batch {
        let indices: Vec<usize> = (0..deleted.len()).collect();
        in_batch(&log, &c, "run", &indices);
        item_ops::restore_prune(&mut c, &bin, &log, "run").expect("back");
    } else {
        for _ in &deleted {
            item_ops::restore_last_trashed(&mut c, &bin, &log).expect("back");
        }
    }
    let shown = tokens.iter().map(|t| {
        let (name, rest) = t.split_at(1);
        let mark = rest.trim_end_matches(|c: char| c == '-' || c.is_ascii_digit());
        let seq = disk_seq(&dir, &item(name)).map_or("-".to_owned(), |s| s.to_string());
        format!("{name}{mark}{seq}")
    });
    shown.collect::<Vec<_>>().join(" ")
}

#[test]
fn undoing_every_delete_gives_the_old_order_past_held_numbers() {
    // (folder, deletes in order, folder after undoing them); a tie with an app item is kept only
    // when the sibling had it before the first delete.
    let cases = [
        ("a1 b2 c3 z!4", "b", "a1 b2 c3 z!4"),
        ("a1 z!2 b3 c4", "a c", "a1 z!2 b3 c4"),
        ("a1 z!2 b3 c4", "c a", "a1 z!2 b3 c4"),
        ("a1 z!2 b3 c4", "c b a", "a1 z!2 b3 c4"),
        ("a1 b2 z!2 c3", "a c", "a1 b2 z!2 c3"),
        ("a1 b2 z!2 c3", "c a", "a1 b2 z!2 c3"),
        ("a1 b1 z!1 c2", "b c", "a1 b2 z!1 c3"),
        ("a- b6 z!1", "b", "a2 b3 z!1"),
        ("a0 b5 z!1", "a", "a2 b3 z!1"),
        ("a- b- z!1", "a b", "a2 b3 z!1"),
        ("a1 b- c- z!2", "a", "a1 b3 c4 z!2"),
        ("a1 b/- z!2", "a", "a1 b/3 z!2"),
        ("a1 b/0 z!2 y!4", "a", "a1 b/3 z!2 y!4"),
        ("a/- b1 z!2", "b", "a/3 b1 z!2"),
        ("a1 b/- c/- z!2", "a", "a1 b/3 c/4 z!2"),
        ("a1 b/- c3 z!2", "c a", "a1 b/3 c4 z!2"),
    ];
    for (start, deleted, want) in cases {
        for batch in [false, true] {
            let got = after_undo(start, deleted, batch);
            assert_eq!(got, want, "{start}, deleting {deleted}, batch {batch}");
        }
    }
}

#[test]
fn undoing_a_delete_puts_a_seqless_sibling_back_behind_the_number_it_followed() {
    // The delete pulls a seqless sibling ahead of a numbered one; the undo sends it back.
    let cases = [
        ("f/- g/- r2 s4 t5", "f", "f/1 g/3 r2 s4 t5"),
        ("e/- f/- g/- h3", "e", "e/1 f/2 g/4 h3"),
        ("h1 b/- n3 d/-", "b", "h1 b/2 n3 d/4"),
        ("f/- g/- r2 s4 t5", "f g", "f/1 g/3 r2 s4 t5"),
        ("f/- g/- h/- r3", "f g", "f/1 g/2 h/4 r3"),
    ];
    for (start, deleted, want) in cases {
        for batch in [false, true] {
            let got = after_undo(start, deleted, batch);
            assert_eq!(got, want, "{start}, deleting {deleted}, batch {batch}");
        }
    }
}

#[test]
fn a_seqless_sibling_renumbered_since_the_delete_stays_where_it_was_put() {
    let bare = |name: &str| tree_support::folder(name, 1).replace("  seq: 1\n", "");
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &bare("f")),
        ("g/folder.yml", &bare("g")),
        ("r.yml", &request("r", 2)),
        ("s.yml", &request("s", 4)),
    ]);
    let logs = TempDir::new().expect("tempdir");
    let (bin, log) = (FakeBin::new(), TrashLog::new(logs.path()));
    item_ops::trash_item_logged(&mut c, "f", &bin, Some(&log)).expect("rm");
    // Written behind the open collection's back: g leading is now a choice, not the delete's doing.
    write(&dir, "g/folder.yml", &tree_support::folder("g", 2));
    write(&dir, "r.yml", &request("r", 5));
    write(&dir, "s.yml", &request("s", 6));
    item_ops::restore_last_trashed(&mut c, &bin, &log).expect("back");
    assert_eq!(
        seqs(&dir, ["g", "f", "r.yml", "s.yml"]),
        [1, 2, 3, 4].map(Some)
    );
}

#[test]
fn undoing_a_delete_keeps_a_seqless_folder_where_it_stood_before_a_numbered_tail() {
    // Seqless requests count as 1, so the folder sorts after them and before e.
    for batch in [false, true] {
        let got = after_undo("a/- b- c- d- e3", "d", batch);
        assert_eq!(got, "a/4 b- c2 d3 e5", "batch {batch}");
    }
}

/// The names of a case in the order the folder lists them: names first, then each positive seq
/// spliced in at its index, equal seqs sharing a slot. A request stored without one counts as 1.
fn listed_order(case: &str) -> String {
    let mut items: Vec<(char, Option<u64>)> = case
        .split(' ')
        .map(|t| {
            let name = t.chars().next().expect("name");
            let digits = t.trim_start_matches(|c: char| !c.is_ascii_digit());
            let stored: Option<u64> = digits.parse().ok();
            let seq = match &t[1..2] {
                "/" => stored,
                "!" => Some(stored.unwrap_or(1)),
                _ => Some(stored.filter(|&s| s > 0).unwrap_or(1)),
            };
            (name, seq.filter(|&s| s > 0))
        })
        .collect();
    items.sort_by_key(|&(name, _)| name);
    let (mut numbered, rest): (Vec<_>, Vec<_>) = items.into_iter().partition(|i| i.1.is_some());
    numbered.sort_by_key(|&(_, seq)| seq);
    let mut slots: Vec<Vec<(char, Option<u64>)>> = rest.into_iter().map(|i| vec![i]).collect();
    for item in numbered {
        let at =
            usize::try_from(item.1.expect("seq") - 1).map_or(slots.len(), |i| i.min(slots.len()));
        match slots.get_mut(at) {
            Some(slot) if slot[0].1 == item.1 => slot.push(item),
            _ => slots.insert(at, vec![item]),
        }
    }
    slots.into_iter().flatten().map(|(name, _)| name).collect()
}

/// A small deterministic generator, so each seed always draws the same folder and deletes.
struct Draw(u64);

impl Draw {
    fn below(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % n
    }
}

/// A random folder in the table notation and a few of its items to delete, in order. A
/// canonical folder holds only requests and folders numbered 1..n, each number once, though an
/// item may store no number at all.
fn drawn_case(seed: u64, canonical: bool) -> (String, String) {
    let mut draw = Draw(seed);
    let count = 3 + draw.below(4);
    let mut free: Vec<u64> = (1..=count).collect();
    let tokens: Vec<String> = (0..count)
        .map(|i| {
            let name = char::from(b'a' + u8::try_from(i).expect("small"));
            let kind = match draw.below(10) {
                0 if !canonical => "!",
                0..=3 => "/",
                _ => "",
            };
            let seq = if canonical {
                let at = usize::try_from(draw.below(free.len() as u64)).expect("small");
                let seq = free.remove(at);
                if draw.below(3) == 0 {
                    "-".to_owned()
                } else {
                    seq.to_string()
                }
            } else {
                match draw.below(count + 3) {
                    0 => "-".to_owned(),
                    1 => "0".to_owned(),
                    n => (n - 1).min(6).to_string(),
                }
            };
            format!("{name}{kind}{seq}")
        })
        .collect();
    let mut names: Vec<char> = tokens
        .iter()
        .map(|t| t.chars().next().expect("name"))
        .collect();
    let mut deleted = Vec::new();
    for _ in 0..1 + draw.below(3) {
        let i = usize::try_from(draw.below(names.len() as u64)).expect("small");
        deleted.push(names.remove(i).to_string());
    }
    (tokens.join(" "), deleted.join(" "))
}

/// Every run over `seeds` whose undo leaves the folder in another order than it started in.
fn lost_orders(seeds: std::ops::Range<u64>, canonical: bool) -> Vec<String> {
    let mut differ = Vec::new();
    for seed in seeds {
        let (start, deleted) = drawn_case(seed, canonical);
        for batch in [false, true] {
            let got = after_undo(&start, &deleted, batch);
            if listed_order(&got) != listed_order(&start) {
                differ.push(format!("{start}, deleting {deleted}, batch {batch}: {got}"));
            }
        }
    }
    differ
}

#[test]
fn undoing_random_deletes_loses_the_old_order_no_more_often() {
    // Some folders cannot get their order back, such as one whose app item holds a number past a
    // gap; the ceiling is the count of those today, so a change that loses order elsewhere fails.
    let differ = lost_orders(0..400, false);
    assert!(
        differ.len() <= 148,
        "{} differ:\n{}",
        differ.len(),
        differ.join("\n")
    );
}

#[test]
fn undoing_random_deletes_in_a_folder_numbered_one_to_n_keeps_the_old_order() {
    let differ = lost_orders(0..400, true);
    assert!(
        differ.is_empty(),
        "{} differ:\n{}",
        differ.len(),
        differ.join("\n")
    );
}
