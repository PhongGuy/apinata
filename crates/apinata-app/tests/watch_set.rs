use apinata_app::watch::rules::{WatchMsg, translate};
use apinata_app::watch::set::{Watch, WatchError, WatchSet, gap_paths, notify_factory};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Default)]
struct Fake {
    log: Arc<Mutex<Vec<String>>>,
    missing: Arc<Mutex<Vec<PathBuf>>>,
    fail: Option<PathBuf>,
}

impl Watch for Fake {
    fn watch(&mut self, dir: &Path) -> Result<(), WatchError> {
        self.log
            .lock()
            .unwrap()
            .push(format!("watch {}", dir.display()));
        if self.missing.lock().unwrap().iter().any(|m| m == dir) {
            return Err(WatchError::Unwatchable);
        }
        if self.fail.as_deref() == Some(dir) {
            return Err(WatchError::Failed("limit".into()));
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

fn pb(paths: &[&str]) -> Vec<PathBuf> {
    paths.iter().map(PathBuf::from).collect()
}

#[test]
fn reconcile_places_new_dirs_drops_old_ones_and_returns_only_the_new() {
    let fake = Fake::default();
    let mut s = WatchSet::new(Box::new(fake.clone()), PathBuf::from("/r"));
    assert_eq!(
        s.reconcile(&set(&["/r", "/r/a"]), &[]),
        Ok(pb(&["/r", "/r/a"]))
    );
    assert_eq!(s.reconcile(&set(&["/r", "/r/b"]), &[]), Ok(pb(&["/r/b"])));
    assert_eq!(s.placed(), &set(&["/r", "/r/b"]));
    assert_eq!(
        *fake.log.lock().unwrap(),
        ["watch /r", "watch /r/a", "unwatch /r/a", "watch /r/b"]
    );
}

#[test]
fn an_unwatchable_dir_is_skipped_and_retried_on_every_reconcile() {
    let fake = Fake::default();
    fake.missing
        .lock()
        .unwrap()
        .push(PathBuf::from("/r/environments"));
    let mut s = WatchSet::new(Box::new(fake.clone()), PathBuf::from("/r"));
    let desired = set(&["/r", "/r/environments"]);
    assert_eq!(s.reconcile(&desired, &[]), Ok(pb(&["/r"])));
    assert_eq!(s.reconcile(&desired, &[]), Ok(vec![]));
    fake.missing.lock().unwrap().clear();
    assert_eq!(s.reconcile(&desired, &[]), Ok(pb(&["/r/environments"])));
    let attempts = fake
        .log
        .lock()
        .unwrap()
        .iter()
        .filter(|l| *l == "watch /r/environments")
        .count();
    assert_eq!(attempts, 3);
}

#[test]
fn a_failed_watch_is_an_error() {
    let fake = Fake {
        fail: Some(PathBuf::from("/r/b")),
        ..Fake::default()
    };
    let mut s = WatchSet::new(Box::new(fake), PathBuf::from("/r"));
    assert_eq!(
        s.reconcile(&set(&["/r", "/r/b"]), &[]),
        Err("limit".to_string())
    );
}

#[test]
fn an_unwatchable_root_is_a_failure() {
    let fake = Fake::default();
    fake.missing.lock().unwrap().push(PathBuf::from("/r"));
    let mut s = WatchSet::new(Box::new(fake), PathBuf::from("/r"));
    let got = s.reconcile(&set(&["/r", "/r/a"]), &[]);
    assert!(matches!(&got, Err(m) if m.contains("/r")), "{got:?}");
}

#[test]
fn replace_rewatches_placed_desired_dirs_only() {
    let fake = Fake::default();
    let mut s = WatchSet::new(Box::new(fake.clone()), PathBuf::from("/r"));
    let desired = set(&["/r", "/r/a"]);
    s.reconcile(&desired, &[]).unwrap();
    assert_eq!(
        s.reconcile(&desired, &pb(&["/r/a", "/r/gone"])),
        Ok(pb(&["/r/a"]))
    );
    assert_eq!(
        fake.log
            .lock()
            .unwrap()
            .iter()
            .filter(|l| *l == "watch /r/a")
            .count(),
        2
    );
}

#[test]
fn gap_paths_list_each_dirs_entries_sorted() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("b.yml"), "b").unwrap();
    std::fs::write(root.join("a.yml"), "a").unwrap();
    std::fs::create_dir(root.join("sub")).unwrap();
    let got = gap_paths(&[root.to_path_buf(), root.join("missing")]);
    assert_eq!(
        got,
        vec![root.join("a.yml"), root.join("b.yml"), root.join("sub")]
    );
}

#[test]
fn the_notify_watch_delivers_events_and_skips_missing_dirs() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap(); // macOS: /var is a symlink to /private/var
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watch = notify_factory()(Box::new(move |r| {
        let _ = tx.send(r);
    }))
    .unwrap();
    assert_eq!(
        watch.watch(&root.join("missing")),
        Err(WatchError::Unwatchable)
    );
    watch.watch(&root).unwrap();
    std::fs::write(root.join(".env"), "A=1\n").unwrap();
    std::fs::write(root.join("a.yml"), "x").unwrap();
    let mut seen = BTreeSet::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while seen.len() < 2 && Instant::now() < deadline {
        if let Ok(r) = rx.recv_timeout(Duration::from_millis(100))
            && let Some(WatchMsg::Paths(p)) = translate(r, &root, &root)
        {
            seen.extend(p);
        }
    }
    assert_eq!(
        seen,
        BTreeSet::from([root.join(".env"), root.join("a.yml")])
    );
}
