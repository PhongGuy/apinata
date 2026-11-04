//! The shared watch seams: a `WatchSet` over a fake `Watch`, and `translate_batch`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use apinata_core::watch::seams::{
    RawChange, Watch, WatchError, WatchSet, gap_paths, translate_batch,
};
use notify_debouncer_full::DebouncedEvent;
use notify_debouncer_full::notify::event::{CreateKind, Flag};
use notify_debouncer_full::notify::{Error, ErrorKind, Event, EventKind};

#[derive(Clone, Default)]
struct Fake {
    log: Arc<Mutex<Vec<String>>>,
    missing: Vec<PathBuf>,
}

impl Watch for Fake {
    fn watch(&mut self, dir: &Path) -> Result<(), WatchError> {
        self.log
            .lock()
            .unwrap()
            .push(format!("watch {}", dir.display()));
        if self.missing.iter().any(|m| m == dir) {
            return Err(WatchError::Unwatchable);
        }
        Ok(())
    }
    fn unwatch(&mut self, dir: &Path) {
        self.log
            .lock()
            .unwrap()
            .push(format!("unwatch {}", dir.display()));
    }
}

fn set(paths: &[&str]) -> BTreeSet<PathBuf> {
    paths.iter().map(PathBuf::from).collect()
}

fn created(paths: &[&str]) -> DebouncedEvent {
    let mut e = Event::new(EventKind::Create(CreateKind::File));
    for p in paths {
        e = e.add_path(PathBuf::from(p));
    }
    DebouncedEvent::new(e, Instant::now())
}

#[test]
fn a_watch_set_places_and_removes_watches_to_match_the_desired_dirs() {
    let fake = Fake::default();
    let log = fake.log.clone();
    let mut ws = WatchSet::new(Box::new(fake), PathBuf::from("/r"));
    let added = ws.reconcile(&set(&["/r", "/r/a"]), &[]).unwrap();
    assert_eq!(added, vec![PathBuf::from("/r"), PathBuf::from("/r/a")]);
    ws.reconcile(&set(&["/r"]), &[]).unwrap();
    assert_eq!(ws.placed(), &set(&["/r"]));
    assert_eq!(log.lock().unwrap().last().unwrap(), "unwatch /r/a");
}

#[test]
fn a_missing_subdir_is_skipped_for_now() {
    let fake = Fake {
        missing: vec![PathBuf::from("/r/sub")],
        ..Fake::default()
    };
    let mut ws = WatchSet::new(Box::new(fake), PathBuf::from("/r"));
    let added = ws.reconcile(&set(&["/r", "/r/sub"]), &[]).unwrap();
    assert_eq!(added, vec![PathBuf::from("/r")]);
}

#[test]
fn gap_paths_of_an_unreadable_dir_is_empty() {
    assert!(gap_paths(&[PathBuf::from("/definitely/not/here")]).is_empty());
}

#[test]
fn translate_batch_keeps_paths_under_the_root_sorted_and_deduplicated() {
    let root = Path::new("/r");
    let got = translate_batch(
        Ok(vec![
            created(&["/r/b.yml", "/elsewhere/x.yml"]),
            created(&["/r/a.yml", "/r/b.yml"]),
        ]),
        root,
        root,
    );
    assert_eq!(
        got,
        Some(RawChange::Paths(vec![
            PathBuf::from("/r/a.yml"),
            PathBuf::from("/r/b.yml")
        ]))
    );
}

#[test]
fn translate_batch_reports_rescan_and_only_the_watch_limit_as_failure() {
    let root = Path::new("/r");
    let flagged = Event::new(EventKind::Other).set_flag(Flag::Rescan);
    let batch = vec![DebouncedEvent::new(flagged, Instant::now())];
    assert_eq!(
        translate_batch(Ok(batch), root, root),
        Some(RawChange::Rescan)
    );
    assert_eq!(
        translate_batch(Err(vec![Error::new(ErrorKind::MaxFilesWatch)]), root, root),
        Some(RawChange::Failed("OS file watch limit reached.".into()))
    );
    assert_eq!(
        translate_batch(Err(vec![Error::new(ErrorKind::PathNotFound)]), root, root),
        None
    );
}
