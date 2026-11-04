//! Restoring a whole prune at once: every request one update run pruned comes back, newest
//! first, wherever its entries sit in the trash log, or nothing changes. A run without a log
//! says the prune cannot be undone.

use crate::common::{ROOT, write_tree};
use crate::import_update_support::{FakeBin, bundle, paths};
use apinata_core::import::{ImportBundle, UpdateOptions, apply_update, write_collection};
use apinata_core::{Collection, MemoryStore, Trash, TrashError, TrashHandle, TrashLog, item_ops};
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const LEGACY: [&str; 3] = ["removeA", "removeB", "removeC"];

/// An OpenAPI spec with `GET /keep` and, when `legacy`, three more operations in one tag.
fn spec(legacy: bool) -> ImportBundle {
    let mut paths = serde_json::Map::new();
    let op = |id: &str| {
        serde_json::json!({"get": {"tags": ["legacy"], "operationId": id,
        "responses": {"200": {"description": "ok"}}}})
    };
    paths.insert("/keep".into(), op("keep"));
    if legacy {
        for (i, id) in LEGACY.iter().enumerate() {
            paths.insert(format!("/l{i}"), op(id));
        }
    }
    let doc = serde_json::json!({"openapi": "3.0.3", "info": {"title": "Shop", "version": "1"},
        "paths": paths});
    bundle(&serde_json::to_vec(&doc).expect("json"))
}

fn imported(parent: &Path) -> Collection {
    let b = spec(true);
    write_collection(parent, &b.collection, &b.environments, &MemoryStore::new())
        .expect("writes")
        .0
}

fn prune(log: Option<&TrashLog>) -> UpdateOptions {
    let mut o = UpdateOptions::default();
    o.prune = true;
    o.trash_log = log.cloned();
    o
}

fn seq(c: &Collection, rel: &str) -> Option<u64> {
    let text = fs::read_to_string(c.root().join(rel)).ok()?;
    let doc: yaml_serde::Value = yaml_serde::from_str(&text).expect("yaml");
    doc["info"]["seq"].as_u64()
}

fn legacy_files() -> Vec<String> {
    LEGACY.iter().map(|id| format!("legacy/{id}.yml")).collect()
}

/// The entries of the collection's trash record, as JSON.
fn entries(log: &TrashLog, c: &Collection) -> Vec<serde_json::Value> {
    let text = fs::read_to_string(log.file_for(c.root())).expect("log");
    let doc: serde_json::Value = serde_json::from_str(&text).expect("json");
    doc["entries"].as_array().expect("entries").clone()
}

#[test]
fn a_prune_without_a_trash_log_says_it_cannot_be_restored() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path());
    let bin = FakeBin::new(true);
    let s = apply_update(&mut c, &spec(false), &prune(None), Some(&bin as &dyn Trash)).unwrap();
    assert_eq!(s.pruned.len(), 3);
    for file in legacy_files() {
        assert!(
            s.notes.iter().any(|n| n.detail
                == "moved to the trash, but it cannot be restored from here: no data directory"
                && n.file.as_deref() == Some(file.as_str())),
            "{file}: {:?}",
            s.notes
        );
    }
}

#[test]
fn every_request_one_prune_removed_comes_back_with_its_seq() {
    let dir = TempDir::new().unwrap();
    let logs = TempDir::new().unwrap();
    let log = TrashLog::new(logs.path());
    let mut c = imported(dir.path());
    let before: Vec<Option<u64>> = legacy_files().iter().map(|f| seq(&c, f)).collect();
    let keep_before = seq(&c, "legacy/keep.yml");
    let bin = FakeBin::new(true);
    let s = apply_update(
        &mut c,
        &spec(false),
        &prune(Some(&log)),
        Some(&bin as &dyn Trash),
    )
    .unwrap();
    assert_eq!(paths(&s.pruned).len(), 3, "{:?}", s.pruned);
    let recorded = entries(&log, &c);
    let batch = recorded[0]["batch"].as_str().expect("a batch id");
    assert!(recorded.iter().all(|e| e["batch"] == batch), "{recorded:?}");

    let mut c = Collection::open(c.root()).unwrap();
    let all = item_ops::restore_last_prune(&mut c, &bin, &log).expect("restores");
    let mut back: Vec<String> = all.items().iter().map(|p| p.to_string()).collect();
    assert_eq!(back.len(), 3, "{back:?}");
    assert!(all.warnings().is_empty(), "{:?}", all.warnings());
    back.sort();
    assert_eq!(back, legacy_files());
    let after: Vec<Option<u64>> = legacy_files().iter().map(|f| seq(&c, f)).collect();
    assert_eq!(after, before);
    assert_eq!(seq(&c, "legacy/keep.yml"), keep_before);
    assert!(entries(&log, &c).is_empty());
    let err = item_ops::restore_last_prune(&mut c, &bin, &log).unwrap_err();
    assert_eq!(
        err.to_string(),
        "nothing deleted from this collection is left to restore"
    );
}

#[test]
fn a_plain_delete_on_top_refuses_and_changes_nothing() {
    let dir = TempDir::new().unwrap();
    let logs = TempDir::new().unwrap();
    let log = TrashLog::new(logs.path());
    let mut c = imported(dir.path());
    let bin = FakeBin::new(true);
    apply_update(
        &mut c,
        &spec(false),
        &prune(Some(&log)),
        Some(&bin as &dyn Trash),
    )
    .unwrap();
    item_ops::trash_item_logged(&mut c, "legacy/keep.yml", &bin, Some(&log)).expect("rm");
    let record = fs::read(log.file_for(c.root())).unwrap();
    assert!(
        entries(&log, &c)
            .last()
            .is_some_and(|e| e.get("batch").is_none()),
        "a single delete records no batch"
    );
    let batch = entries(&log, &c)[0]["batch"].as_str().unwrap().to_owned();
    let err = item_ops::restore_last_prune(&mut c, &bin, &log).unwrap_err();
    assert_eq!(
        err.to_string(),
        format!(
            "the newest delete was not an import prune; `apinata item restore` (MCP \
             `restore_item`) puts it back; recorded prunes, newest first: {batch}; restore \
             one with `--pruned --batch <ID>` (MCP `batch`)"
        )
    );
    assert_eq!(fs::read(log.file_for(c.root())).unwrap(), record);
    assert!(!c.root().join("legacy/keep.yml").exists());
}

#[test]
fn a_record_written_before_batches_still_restores() {
    let dir = TempDir::new().unwrap();
    let logs = TempDir::new().unwrap();
    let log = TrashLog::new(logs.path());
    let mut c = imported(dir.path());
    let bin = FakeBin::new(true);
    apply_update(
        &mut c,
        &spec(false),
        &prune(Some(&log)),
        Some(&bin as &dyn Trash),
    )
    .unwrap();
    // Strip the batch ids, as a record from an older version has none.
    let file = log.file_for(c.root());
    let mut doc: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    for e in doc["entries"].as_array_mut().unwrap() {
        e.as_object_mut().unwrap().remove("batch");
    }
    fs::write(&file, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
    let one = item_ops::restore_last_trashed(&mut c, &bin, &log).expect("reads");
    assert!(legacy_files().contains(&one.item().to_string()));
    let err = item_ops::restore_last_prune(&mut c, &bin, &log).unwrap_err();
    assert!(
        err.to_string()
            .starts_with("the newest delete was not an import prune")
    );
}

#[test]
fn a_prune_renumbers_from_the_disk_after_another_delete_in_its_folder() {
    let dir = TempDir::new().unwrap();
    let logs = TempDir::new().unwrap();
    let log = TrashLog::new(logs.path());
    let mut c = imported(dir.path());
    let made = item_ops::create_request(&mut c, "legacy", "mine", &item_ops::NewRequest::new())
        .expect("hand-made sibling");
    // Another process deletes the hand-made sibling; this collection still lists it.
    let mut other = Collection::open(c.root()).unwrap();
    let bin = FakeBin::new(true);
    item_ops::trash_item_logged(&mut other, made.as_str(), &bin, Some(&log)).expect("rm");
    let s = apply_update(
        &mut c,
        &spec(false),
        &prune(Some(&log)),
        Some(&bin as &dyn Trash),
    )
    .unwrap();
    assert_eq!(s.pruned.len(), 3, "{:?}", s.notes);
    assert!(
        s.notes
            .iter()
            .all(|n| !n.detail.contains("not renumbered") && !n.detail.starts_with("not pruned")),
        "{:?}",
        s.notes
    );
    assert_eq!(seq(&c, "legacy/keep.yml"), Some(1));
}

/// A collection whose folder `f` holds `names` numbered 1..n, and whose folder `g` holds `other`.
fn folders(names: &[&str], other: &[&str]) -> (TempDir, Collection) {
    let mut files = vec![("opencollection.yml".to_owned(), ROOT.to_owned())];
    for (dir, list) in [("f", names), ("g", other)] {
        for (i, name) in list.iter().enumerate() {
            let seq = i + 1;
            let text = format!(
                "info:\n  name: {name}\n  type: http\n  seq: {seq}\n\
                 http:\n  method: GET\n  url: http://localhost/\n"
            );
            files.push((format!("{dir}/{name}.yml"), text));
        }
    }
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(p, t)| (p.as_str(), t.as_str()))
        .collect();
    let dir = write_tree(&files);
    let c = Collection::open(dir.path()).unwrap();
    (dir, c)
}

fn delete(c: &mut Collection, bin: &dyn Trash, log: &TrashLog, rel: &[&str]) {
    for path in rel {
        item_ops::trash_item_logged(c, path, bin, Some(log)).expect("rm");
    }
}

/// Rewrite the record, as the run that made it would have left it.
fn edit_record(log: &TrashLog, c: &Collection, edit: impl FnOnce(&mut Vec<serde_json::Value>)) {
    let file = log.file_for(c.root());
    let mut doc: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    edit(doc["entries"].as_array_mut().unwrap());
    fs::write(&file, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
}

/// Put the entries at `indices` of the record into `batch`.
fn in_batch(log: &TrashLog, c: &Collection, batch: &str, indices: &[usize]) {
    edit_record(log, c, |list| {
        for &i in indices {
            list[i]["batch"] = batch.into();
        }
    });
}

fn seqs(c: &Collection, rel: &[&str]) -> Vec<Option<u64>> {
    rel.iter().map(|f| seq(c, f)).collect()
}

fn items(all: &item_ops::RestoredAll) -> Vec<String> {
    all.items().iter().map(|p| p.to_string()).collect()
}

/// Every file under `root` with its bytes.
fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else {
                out.insert(path.clone(), fs::read(&path).unwrap());
            }
        }
    }
    out
}

type Hook<'a> = Box<dyn Fn(&TrashHandle) -> Result<(), TrashError> + Send + Sync + 'a>;

/// A bin that first asks `hook` about each restore; an error from it is the restore's error.
struct Hooked<'a> {
    bin: &'a FakeBin,
    hook: Hook<'a>,
}

impl fmt::Debug for Hooked<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Hooked")
    }
}

impl Trash for Hooked<'_> {
    fn available(&self) -> bool {
        true
    }
    fn trash(&self, path: &Path) -> Result<TrashHandle, TrashError> {
        self.bin.trash(path)
    }
    fn restore(&self, handle: &TrashHandle) -> Result<(), TrashError> {
        (self.hook)(handle)?;
        self.bin.restore(handle)
    }
}

fn is(handle: &TrashHandle, name: &str) -> bool {
    handle.original().ends_with(name)
}

const NOT_CHANGED: &str = "cannot restore this prune; nothing in the collection was changed:";

#[test]
fn a_split_prune_comes_back_whole_and_the_foreign_delete_stays_recorded() {
    let dir = TempDir::new().unwrap();
    let logs = TempDir::new().unwrap();
    let log = TrashLog::new(logs.path());
    let mut c = imported(dir.path());
    let bin = FakeBin::new(true);
    let s = apply_update(
        &mut c,
        &spec(false),
        &prune(Some(&log)),
        Some(&bin as &dyn Trash),
    )
    .unwrap();
    let order: Vec<String> = s.pruned.iter().map(|p| p.to_string()).collect();
    item_ops::trash_item_logged(&mut c, "legacy/keep.yml", &bin, Some(&log)).expect("rm");
    // A delete that landed between two prunes of the run: move it below the newest two.
    edit_record(&log, &c, |list| {
        let single = list.pop().unwrap();
        list.insert(1, single);
    });

    let mut c = Collection::open(c.root()).unwrap();
    let all = item_ops::restore_last_prune(&mut c, &bin, &log).expect("restores");
    assert_eq!(
        items(&all),
        [order[2].clone(), order[1].clone(), order[0].clone()]
    );
    assert!(all.warnings().is_empty(), "{:?}", all.warnings());
    let left = entries(&log, &c);
    assert_eq!(left.len(), 1, "{left:?}");
    assert_eq!(left[0]["item"], "legacy/keep.yml");
    assert!(!c.root().join("legacy/keep.yml").exists());
}

#[test]
fn seqs_are_dense_after_a_foreign_delete_in_the_same_folder() {
    let (_dir, mut c) = folders(&["a", "b", "c", "d"], &[]);
    let logs = TempDir::new().unwrap();
    let (bin, log) = (FakeBin::new(true), TrashLog::new(logs.path()));
    delete(&mut c, &bin, &log, &["f/b.yml", "f/a.yml", "f/d.yml"]);
    in_batch(&log, &c, "run", &[0, 2]);
    let all = item_ops::restore_last_prune(&mut c, &bin, &log).expect("restores");
    assert_eq!(items(&all), ["f/d.yml", "f/b.yml"]);
    let rel = ["f/b.yml", "f/c.yml", "f/d.yml"];
    assert_eq!(seqs(&c, &rel), [1, 2, 3].map(Some));
}

#[test]
fn a_named_prune_comes_back_past_a_newer_one_which_stays_recorded() {
    let (_dir, mut c) = folders(&["a", "b", "c", "d"], &[]);
    let logs = TempDir::new().unwrap();
    let (bin, log) = (FakeBin::new(true), TrashLog::new(logs.path()));
    delete(&mut c, &bin, &log, &["f/a.yml", "f/b.yml", "f/c.yml"]);
    in_batch(&log, &c, "old", &[0, 1]);
    in_batch(&log, &c, "new", &[2]);
    let all = item_ops::restore_prune(&mut c, &bin, &log, "old").expect("restores");
    assert_eq!(items(&all), ["f/b.yml", "f/a.yml"]);
    assert_eq!(
        seqs(&c, &["f/a.yml", "f/b.yml", "f/d.yml"]),
        [1, 2, 3].map(Some)
    );
    let left = entries(&log, &c);
    assert_eq!(left.len(), 1, "{left:?}");
    assert_eq!(left[0]["batch"], "new");
    assert!(!c.root().join("f/c.yml").exists());
}

#[test]
fn every_clash_is_listed_and_nothing_changes() {
    let (_dir, mut c) = folders(&["a", "b", "c", "d"], &[]);
    let logs = TempDir::new().unwrap();
    let (bin, log) = (FakeBin::new(true), TrashLog::new(logs.path()));
    delete(&mut c, &bin, &log, &["f/a.yml", "f/b.yml", "f/c.yml"]);
    in_batch(&log, &c, "run", &[0, 1, 2]);
    fs::write(c.root().join("f/a.yml"), "mine\n").unwrap();
    fs::write(c.root().join("f/c.yml"), "mine too\n").unwrap();
    let files = tree(c.root());
    let record = fs::read(log.file_for(c.root())).unwrap();
    let taken = bin.taken();

    let err = item_ops::restore_prune(&mut c, &bin, &log, "run").unwrap_err();
    assert_eq!(
        err.to_string(),
        format!(
            "{NOT_CHANGED}\n\
             f/c.yml: something is at that path now; move or rename it, then restore again\n\
             f/a.yml: something is at that path now; move or rename it, then restore again"
        )
    );
    assert_eq!(tree(c.root()), files);
    assert_eq!(fs::read(log.file_for(c.root())).unwrap(), record);
    assert_eq!(bin.taken(), taken);

    fs::remove_file(c.root().join("f/a.yml")).unwrap();
    fs::remove_file(c.root().join("f/c.yml")).unwrap();
    let all = item_ops::restore_prune(&mut c, &bin, &log, "run").expect("restores");
    assert_eq!(items(&all).len(), 3);
    assert!(entries(&log, &c).is_empty());
    let rel = ["f/a.yml", "f/b.yml", "f/c.yml", "f/d.yml"];
    assert_eq!(seqs(&c, &rel), [1, 2, 3, 4].map(Some));
}

#[test]
fn a_symlink_at_the_path_is_a_clash() {
    let (_dir, mut c) = folders(&["a", "b"], &[]);
    let logs = TempDir::new().unwrap();
    let (bin, log) = (FakeBin::new(true), TrashLog::new(logs.path()));
    delete(&mut c, &bin, &log, &["f/a.yml"]);
    in_batch(&log, &c, "run", &[0]);
    #[cfg(unix)]
    std::os::unix::fs::symlink("/nonexistent", c.root().join("f/a.yml")).unwrap();
    #[cfg(not(unix))]
    fs::write(c.root().join("f/a.yml"), "").unwrap();
    let err = item_ops::restore_prune(&mut c, &bin, &log, "run").unwrap_err();
    assert!(err.to_string().starts_with(NOT_CHANGED), "{err}");
    assert_eq!(entries(&log, &c).len(), 1);
}

#[test]
fn a_clash_on_the_newest_refuses_the_whole_prune() {
    let dir = TempDir::new().unwrap();
    let logs = TempDir::new().unwrap();
    let log = TrashLog::new(logs.path());
    let mut c = imported(dir.path());
    let bin = FakeBin::new(true);
    let s = apply_update(
        &mut c,
        &spec(false),
        &prune(Some(&log)),
        Some(&bin as &dyn Trash),
    )
    .unwrap();
    let newest = s.pruned[2].to_string();
    fs::write(c.root().join(&newest), "mine\n").unwrap();
    let err = item_ops::restore_last_prune(&mut c, &bin, &log).unwrap_err();
    let text = err.to_string();
    assert!(text.starts_with(NOT_CHANGED), "{text}");
    assert!(text.contains(&newest), "{text}");
    assert_eq!(entries(&log, &c).len(), 3);
    assert_eq!(bin.taken(), 3);
}

#[test]
fn a_missing_folder_refuses_the_whole_prune() {
    let (_dir, mut c) = folders(&["a", "b"], &["x", "y"]);
    let logs = TempDir::new().unwrap();
    let (bin, log) = (FakeBin::new(true), TrashLog::new(logs.path()));
    delete(&mut c, &bin, &log, &["f/a.yml", "g/x.yml"]);
    in_batch(&log, &c, "run", &[0, 1]);
    fs::remove_dir_all(c.root().join("g")).unwrap();
    let err = item_ops::restore_prune(&mut c, &bin, &log, "run").unwrap_err();
    assert_eq!(
        err.to_string(),
        format!(
            "{NOT_CHANGED}\n\
             g/x.yml: its folder g no longer exists; create the folder, then restore again"
        )
    );
    assert!(!c.root().join("f/a.yml").exists());
    assert_eq!(entries(&log, &c).len(), 2);
}

#[test]
fn an_invalid_entry_refuses_once_and_is_dropped() {
    let (_dir, mut c) = folders(&["a", "b", "c"], &[]);
    let logs = TempDir::new().unwrap();
    let (bin, log) = (FakeBin::new(true), TrashLog::new(logs.path()));
    delete(&mut c, &bin, &log, &["f/a.yml", "f/b.yml"]);
    in_batch(&log, &c, "run", &[0, 1]);
    edit_record(&log, &c, |list| {
        list[0]["original"] = "/elsewhere/a.yml".into()
    });
    let err = item_ops::restore_prune(&mut c, &bin, &log, "run").unwrap_err();
    assert_eq!(
        err.to_string(),
        format!("{NOT_CHANGED}\nf/a.yml: its trash record was invalid, so it was dropped")
    );
    assert!(!c.root().join("f/b.yml").exists());
    assert_eq!(entries(&log, &c).len(), 1);

    let all = item_ops::restore_prune(&mut c, &bin, &log, "run").expect("restores");
    assert_eq!(items(&all), ["f/b.yml"]);
    assert!(entries(&log, &c).is_empty());
}

#[test]
fn two_entries_with_one_path_refuse() {
    let (_dir, mut c) = folders(&["a", "b"], &[]);
    let logs = TempDir::new().unwrap();
    let (bin, log) = (FakeBin::new(true), TrashLog::new(logs.path()));
    delete(&mut c, &bin, &log, &["f/a.yml", "f/b.yml"]);
    edit_record(&log, &c, |list| {
        list[0]["item"] = "f/b.yml".into();
        list[0]["original"] = list[1]["original"].clone();
    });
    in_batch(&log, &c, "run", &[0, 1]);
    let record = fs::read(log.file_for(c.root())).unwrap();
    let err = item_ops::restore_prune(&mut c, &bin, &log, "run").unwrap_err();
    assert_eq!(
        err.to_string(),
        format!("{NOT_CHANGED}\nf/b.yml: two entries of this prune name this path")
    );
    assert_eq!(fs::read(log.file_for(c.root())).unwrap(), record);
    assert!(!c.root().join("f/b.yml").exists());
}

#[cfg(unix)]
#[test]
fn an_unwritable_record_refuses_with_nothing_changed() {
    let dir = TempDir::new().unwrap();
    let logs = TempDir::new().unwrap();
    let log = TrashLog::new(logs.path());
    let mut c = imported(dir.path());
    let bin = FakeBin::new(true);
    apply_update(
        &mut c,
        &spec(false),
        &prune(Some(&log)),
        Some(&bin as &dyn Trash),
    )
    .unwrap();
    let files = tree(c.root());
    let guard = crate::common::ModeGuard::set(logs.path(), 0o555);
    if fs::write(logs.path().join("probe"), "").is_ok() {
        return; // permissions are not enforced (root), so the record still saves
    }
    let err = item_ops::restore_last_prune(&mut c, &bin, &log).unwrap_err();
    guard.restore();
    assert!(
        err.to_string().starts_with("cannot use the trash record "),
        "{err}"
    );
    assert_eq!(tree(c.root()), files);
    assert_eq!(bin.taken(), 3);
    assert_eq!(entries(&log, &c).len(), 3);
}

#[test]
fn an_unknown_prune_lists_the_recorded_ones_newest_first() {
    let (_dir, mut c) = folders(&["a", "b", "c", "d"], &[]);
    let logs = TempDir::new().unwrap();
    let (bin, log) = (FakeBin::new(true), TrashLog::new(logs.path()));
    let err = item_ops::restore_prune(&mut c, &bin, &log, "nope").unwrap_err();
    assert_eq!(
        err.to_string(),
        "no prune nope in the trash record, which holds no prunes"
    );
    delete(&mut c, &bin, &log, &["f/a.yml", "f/b.yml", "f/c.yml"]);
    in_batch(&log, &c, "older", &[0]);
    in_batch(&log, &c, "newer", &[1, 2]);
    let record = fs::read(log.file_for(c.root())).unwrap();
    let err = item_ops::restore_prune(&mut c, &bin, &log, "nope").unwrap_err();
    assert_eq!(
        err.to_string(),
        "no prune nope in the trash record; recorded prunes, newest first: newer, older"
    );
    assert_eq!(fs::read(log.file_for(c.root())).unwrap(), record);
}

#[test]
fn a_plain_delete_on_top_lists_every_recorded_prune() {
    let (_dir, mut c) = folders(&["a", "b", "c", "d"], &[]);
    let logs = TempDir::new().unwrap();
    let (bin, log) = (FakeBin::new(true), TrashLog::new(logs.path()));
    delete(&mut c, &bin, &log, &["f/a.yml", "f/b.yml", "f/c.yml"]);
    in_batch(&log, &c, "older", &[0]);
    in_batch(&log, &c, "newer", &[1]);
    let err = item_ops::restore_last_prune(&mut c, &bin, &log).unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains("recorded prunes, newest first: newer, older;"),
        "{text}"
    );
}

#[test]
fn an_item_gone_from_the_bin_stops_the_batch_and_what_is_back_is_dense() {
    let (_dir, mut c) = folders(&["a", "b", "c", "d"], &[]);
    let logs = TempDir::new().unwrap();
    let (bin, log) = (FakeBin::new(true), TrashLog::new(logs.path()));
    delete(&mut c, &bin, &log, &["f/a.yml", "f/b.yml", "f/c.yml"]);
    in_batch(&log, &c, "run", &[0, 1, 2]);
    let gone = Hooked {
        bin: &bin,
        hook: Box::new(|h| {
            if is(h, "b.yml") {
                return Err(TrashError::Missing);
            }
            Ok(())
        }),
    };
    let all = item_ops::restore_prune(&mut c, &gone, &log, "run").expect("partial");
    assert_eq!(items(&all), ["f/c.yml"]);
    let warning = all.warnings().join("\n");
    assert!(
        warning.contains("f/b.yml is no longer in the trash"),
        "{warning}"
    );
    assert!(warning.contains("1 more from this prune"), "{warning}");
    assert_eq!(seqs(&c, &["f/c.yml", "f/d.yml"]), [1, 2].map(Some));
    assert_eq!(entries(&log, &c).len(), 1);

    let rest = item_ops::restore_prune(&mut c, &bin, &log, "run").expect("the rest");
    assert_eq!(items(&rest), ["f/a.yml"]);
    assert!(entries(&log, &c).is_empty());
    let rel = ["f/a.yml", "f/c.yml", "f/d.yml"];
    assert_eq!(seqs(&c, &rel), [1, 2, 3].map(Some));
}

#[test]
fn the_newest_gone_from_the_bin_is_the_error_itself_and_its_entry_is_dropped() {
    let (_dir, mut c) = folders(&["a", "b", "c"], &[]);
    let logs = TempDir::new().unwrap();
    let (bin, log) = (FakeBin::new(true), TrashLog::new(logs.path()));
    delete(&mut c, &bin, &log, &["f/a.yml", "f/b.yml"]);
    in_batch(&log, &c, "run", &[0, 1]);
    let gone = Hooked {
        bin: &bin,
        hook: Box::new(|_| Err(TrashError::Missing)),
    };
    let err = item_ops::restore_prune(&mut c, &gone, &log, "run").unwrap_err();
    assert!(
        err.to_string()
            .starts_with("f/b.yml is no longer in the trash"),
        "{err}"
    );
    assert_eq!(entries(&log, &c).len(), 1);
    assert!(!c.root().join("f/a.yml").exists());
}

#[test]
fn a_bin_failure_stops_the_batch_and_keeps_its_entry() {
    let (_dir, mut c) = folders(&["a", "b", "c", "d"], &[]);
    let logs = TempDir::new().unwrap();
    let (bin, log) = (FakeBin::new(true), TrashLog::new(logs.path()));
    delete(&mut c, &bin, &log, &["f/a.yml", "f/b.yml", "f/c.yml"]);
    in_batch(&log, &c, "run", &[0, 1, 2]);
    let failing = Hooked {
        bin: &bin,
        hook: Box::new(|h| {
            if is(h, "b.yml") {
                return Err(TrashError::Failed("disk trouble".into()));
            }
            Ok(())
        }),
    };
    let all = item_ops::restore_prune(&mut c, &failing, &log, "run").expect("partial");
    assert_eq!(items(&all), ["f/c.yml"]);
    let warning = all.warnings().join("\n");
    assert!(warning.contains("disk trouble"), "{warning}");
    assert!(warning.contains("2 more from this prune"), "{warning}");
    assert_eq!(entries(&log, &c).len(), 2);
}

#[test]
fn a_file_that_appears_after_the_check_stops_the_batch_and_keeps_its_entry() {
    let (_dir, mut c) = folders(&["a", "b", "c", "d"], &[]);
    let root = c.root().to_path_buf();
    let logs = TempDir::new().unwrap();
    let (bin, log) = (FakeBin::new(true), TrashLog::new(logs.path()));
    delete(&mut c, &bin, &log, &["f/a.yml", "f/b.yml", "f/c.yml"]);
    in_batch(&log, &c, "run", &[0, 1, 2]);
    let racing = Hooked {
        bin: &bin,
        hook: Box::new(|h| {
            if is(h, "c.yml") {
                fs::write(root.join("f/b.yml"), "mine\n").unwrap();
            }
            Ok(())
        }),
    };
    let all = item_ops::restore_prune(&mut c, &racing, &log, "run").expect("partial");
    assert_eq!(items(&all), ["f/c.yml"]);
    let warning = all.warnings().join("\n");
    assert!(warning.contains("something is at that path"), "{warning}");
    assert!(warning.contains("f/b.yml"), "{warning}");
    assert!(warning.contains("2 more from this prune"), "{warning}");
    assert_eq!(entries(&log, &c).len(), 2);
    assert_eq!(fs::read_to_string(root.join("f/b.yml")).unwrap(), "mine\n");
}

#[test]
fn a_bin_failure_with_nothing_back_is_the_error_and_keeps_every_entry() {
    let (_dir, mut c) = folders(&["a", "b", "c", "d"], &[]);
    let root = c.root().to_path_buf();
    let logs = TempDir::new().unwrap();
    let (bin, log) = (FakeBin::new(true), TrashLog::new(logs.path()));
    delete(&mut c, &bin, &log, &["f/a.yml", "f/b.yml"]);
    in_batch(&log, &c, "run", &[0, 1]);
    let failing = Hooked {
        bin: &bin,
        hook: Box::new(|_| Err(TrashError::Failed("disk trouble".into()))),
    };
    let err = item_ops::restore_prune(&mut c, &failing, &log, "run").unwrap_err();
    assert!(err.to_string().contains("disk trouble"), "{err}");
    assert!(err.to_string().contains("f/b.yml"), "{err}");
    assert_eq!(entries(&log, &c).len(), 2);
    assert!(!root.join("f/a.yml").exists());
    assert!(!root.join("f/b.yml").exists());
}

#[test]
fn a_sibling_that_cannot_be_renumbered_is_a_warning() {
    let (_dir, mut c) = folders(&["a", "b", "c"], &[]);
    let root = c.root().to_path_buf();
    let logs = TempDir::new().unwrap();
    let (bin, log) = (FakeBin::new(true), TrashLog::new(logs.path()));
    delete(&mut c, &bin, &log, &["f/a.yml"]);
    in_batch(&log, &c, "run", &[0]);
    let editing = Hooked {
        bin: &bin,
        hook: Box::new(|_| {
            let text = fs::read_to_string(root.join("f/c.yml")).unwrap();
            fs::write(root.join("f/c.yml"), format!("{text}# edited\n")).unwrap();
            Ok(())
        }),
    };
    let all = item_ops::restore_prune(&mut c, &editing, &log, "run").expect("back");
    assert_eq!(items(&all), ["f/a.yml"]);
    let warning = all.warnings().join("\n");
    assert!(
        warning.starts_with("restored, but could not renumber the siblings: "),
        "{warning}"
    );
    assert!(entries(&log, &c).is_empty());
}

#[cfg(unix)]
#[test]
fn a_record_that_fails_after_the_probe_is_a_warning_and_its_entries_stay() {
    let (_dir, mut c) = folders(&["a", "b", "c"], &[]);
    let logs = TempDir::new().unwrap();
    let (bin, log) = (FakeBin::new(true), TrashLog::new(logs.path()));
    delete(&mut c, &bin, &log, &["f/a.yml"]);
    in_batch(&log, &c, "run", &[0]);
    let guard = std::sync::Mutex::new(None);
    let locking = Hooked {
        bin: &bin,
        hook: Box::new(|_| {
            *guard.lock().unwrap() = Some(crate::common::ModeGuard::set(logs.path(), 0o555));
            Ok(())
        }),
    };
    let all = item_ops::restore_prune(&mut c, &locking, &log, "run").expect("back");
    let locked = guard.lock().unwrap().take();
    let enforced = fs::write(logs.path().join("probe"), "").is_err();
    drop(locked);
    if !enforced {
        return; // permissions are not enforced (root), so the record still saved
    }
    assert_eq!(items(&all), ["f/a.yml"]);
    let warning = all.warnings().join("\n");
    assert!(
        warning.contains("restored, but the trash record could not be updated"),
        "{warning}"
    );
    assert!(warning.contains("refuses"), "{warning}");
    assert_eq!(entries(&log, &c).len(), 1);
    assert!(c.root().join("f/a.yml").is_file());
}

#[test]
fn a_foreign_delete_restored_after_the_batch_goes_before_its_tail() {
    let (_dir, mut c) = folders(&["a", "b", "c"], &[]);
    let logs = TempDir::new().unwrap();
    let (bin, log) = (FakeBin::new(true), TrashLog::new(logs.path()));
    delete(&mut c, &bin, &log, &["f/b.yml", "f/a.yml", "f/c.yml"]);
    in_batch(&log, &c, "run", &[0, 2]);
    let all = item_ops::restore_prune(&mut c, &bin, &log, "run").expect("restores");
    assert_eq!(items(&all), ["f/c.yml", "f/b.yml"]);
    assert_eq!(seqs(&c, &["f/b.yml", "f/c.yml"]), [1, 2].map(Some));
    let one = item_ops::restore_last_trashed(&mut c, &bin, &log).expect("restores");
    assert_eq!(one.item().to_string(), "f/a.yml");
    let rel = ["f/b.yml", "f/a.yml", "f/c.yml"];
    assert_eq!(seqs(&c, &rel), [1, 2, 3].map(Some));
}

/// An accepted limit: neither tail names the other prune, so their order is lost.
#[test]
fn two_prunes_restored_out_of_order_lose_their_relative_place() {
    let (_dir, mut c) = folders(&["a", "b", "c"], &[]);
    let logs = TempDir::new().unwrap();
    let (bin, log) = (FakeBin::new(true), TrashLog::new(logs.path()));
    delete(&mut c, &bin, &log, &["f/b.yml", "f/a.yml"]);
    in_batch(&log, &c, "first", &[0]);
    in_batch(&log, &c, "second", &[1]);
    item_ops::restore_prune(&mut c, &bin, &log, "first").expect("restores");
    item_ops::restore_prune(&mut c, &bin, &log, "second").expect("restores");
    let rel = ["f/b.yml", "f/a.yml", "f/c.yml"];
    assert_eq!(seqs(&c, &rel), [1, 2, 3].map(Some));
}
