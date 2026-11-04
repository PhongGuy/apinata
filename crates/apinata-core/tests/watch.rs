//! The public collection watcher: files changed under a root arrive as `Change::Paths`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use apinata_core::watch::{Change, CollectionWatcher};

const WAIT: Duration = Duration::from_secs(10);

fn dirs(list: &[&Path]) -> BTreeSet<PathBuf> {
    list.iter().map(|p| p.to_path_buf()).collect()
}

fn start(root: &Path, watched: &[&Path]) -> (CollectionWatcher, Receiver<Change>) {
    let (tx, rx) = channel();
    let watcher = CollectionWatcher::start(root.to_path_buf(), dirs(watched), move |change| {
        let _ = tx.send(change);
    })
    .expect("watcher starts");
    (watcher, rx)
}

/// Collect paths from batches until every wanted path was seen, or fail after `WAIT`.
fn await_paths(rx: &Receiver<Change>, wanted: &[PathBuf]) {
    let mut missing: BTreeSet<&PathBuf> = wanted.iter().collect();
    let deadline = Instant::now() + WAIT;
    while !missing.is_empty() {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(Change::Paths(paths)) => {
                for p in &paths {
                    missing.remove(p);
                }
            }
            Ok(Change::Rescan) => {}
            Ok(other) => panic!("unexpected change {other:?}"),
            Err(_) => panic!("timed out; still missing {missing:?}"),
        }
    }
}

fn root() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("col");
    fs::create_dir(&root).unwrap();
    (tmp, root)
}

#[test]
fn create_edit_and_delete_arrive_as_paths() {
    let (_tmp, root) = root();
    let (_w, rx) = start(&root, &[&root]);
    let file = root.join("a.yml");

    fs::write(&file, "one").unwrap();
    await_paths(&rx, std::slice::from_ref(&file));
    fs::write(&file, "two").unwrap();
    await_paths(&rx, std::slice::from_ref(&file));
    fs::remove_file(&file).unwrap();
    await_paths(&rx, std::slice::from_ref(&file));
}

#[test]
fn a_directory_added_with_set_dirs_is_watched() {
    let (_tmp, root) = root();
    let (w, rx) = start(&root, &[&root]);
    let sub = root.join("sub");
    fs::create_dir(&sub).unwrap();
    // The existing entry is reported when the directory is first placed.
    fs::write(sub.join("early.yml"), "x").unwrap();
    w.set_dirs(dirs(&[&root, &sub]));
    await_paths(&rx, &[sub.join("early.yml")]);

    let late = sub.join("late.yml");
    fs::write(&late, "y").unwrap();
    await_paths(&rx, &[late]);
}

#[test]
fn a_burst_of_writes_is_seen_in_full() {
    let (_tmp, root) = root();
    let (_w, rx) = start(&root, &[&root]);
    let files: Vec<PathBuf> = (0..20).map(|i| root.join(format!("r{i}.yml"))).collect();
    for f in &files {
        fs::write(f, "x").unwrap();
    }
    await_paths(&rx, &files);
}

#[test]
fn nothing_outside_the_root_is_reported() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("col");
    let outside = tmp.path().join("other");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&outside).unwrap();
    // The outside directory is watched too, so its events reach the translator.
    let (_w, rx) = start(&root, &[&root, &outside]);

    fs::write(outside.join("o.yml"), "x").unwrap();
    let inside = root.join("i.yml");
    fs::write(&inside, "x").unwrap();
    let deadline = Instant::now() + WAIT;
    let mut seen_inside = false;
    while !seen_inside {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left).expect("inside event") {
            Change::Paths(paths) => {
                assert!(paths.iter().all(|p| p.starts_with(&root)), "{paths:?}");
                seen_inside = paths.contains(&inside);
            }
            Change::Rescan => {}
            other => panic!("unexpected change {other:?}"),
        }
    }
}

#[test]
fn dropping_the_watcher_stops_the_callback() {
    let (_tmp, root) = root();
    let (w, rx) = start(&root, &[&root]);
    let file = root.join("a.yml");
    fs::write(&file, "x").unwrap();
    await_paths(&rx, std::slice::from_ref(&file));

    drop(w);
    fs::write(&file, "y").unwrap();
    // The callback's sender is dropped with the thread, so the channel closes instead of
    // delivering; a timeout would mean the thread is still alive and silent.
    let res = rx.recv_timeout(WAIT);
    assert!(
        matches!(res, Err(std::sync::mpsc::RecvTimeoutError::Disconnected)),
        "{res:?}"
    );
}

/// Signals its channel when dropped, which happens when the watcher thread ends and frees
/// the callback that owns it.
struct EndSignal(std::sync::mpsc::Sender<()>);

impl Drop for EndSignal {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

#[test]
fn dropping_the_watcher_ends_its_thread_without_further_events() {
    let (_tmp, root) = root();
    let (ended_tx, ended_rx) = channel();
    let (seen_tx, seen_rx) = channel();
    let guard = EndSignal(ended_tx);
    let watcher = CollectionWatcher::start(root.clone(), dirs(&[&root]), move |change| {
        let _keep = &guard;
        let _ = seen_tx.send(change);
    })
    .expect("watcher starts");

    // Once an event has come through, the thread is idle and blocked waiting for input.
    let file = root.join("a.yml");
    fs::write(&file, "x").unwrap();
    await_paths(&seen_rx, std::slice::from_ref(&file));

    // Late batches for the same write would wake the thread; let them pass first.
    while seen_rx.recv_timeout(Duration::from_secs(1)).is_ok() {}

    drop(watcher);
    // No file is written after the drop, so only an explicit stop can end the thread.
    ended_rx
        .recv_timeout(WAIT)
        .expect("the watcher thread ends after the drop");
}
