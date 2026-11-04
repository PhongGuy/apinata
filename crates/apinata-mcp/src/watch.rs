//! Pushing external edits to the client: a file watcher feeds `resources/updated` for
//! subscribed resources and `resources/list_changed` when the set of resources changes.
//!
//! The mapping from a changed path to a resource URI works on paths alone; file contents are
//! never read here. Only the collection loader reads files.

use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Component, Path};
use std::sync::Arc;
use std::time::Duration;

use apinata_core::Collection;
use apinata_core::watch::{Change, CollectionWatcher};
use rmcp::RoleServer;
use rmcp::model::ResourceUpdatedNotificationParam;
use rmcp::service::Peer;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout};

use crate::gate::WriteGate;
use crate::resources::{self, ENVIRONMENT};
use crate::server::ApinataServer;

const REQUEST_EXT: &str = ".yml";
const FOLDER_FILE: &str = "folder.yml";
const ROOT_FILE: &str = "opencollection.yml";
const ENVIRONMENTS_DIR: &str = "environments";

/// The resource URI of the collection file at `path`, or `None` when the path is not a
/// resource: ignored by the collection, not a `.yml` file, outside the collection, or a symlink
/// whose target leaves the root. A removed path is placed through its canonical parent.
pub(crate) fn uri_for_path(collection: &Collection, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(collection.root()).ok()?;
    if collection.is_ignored(path) {
        return None;
    }
    let real_root = fs::canonicalize(collection.root()).ok()?;
    let canonical = match fs::canonicalize(path) {
        Ok(canonical) => canonical,
        Err(_) => {
            // A dangling link is not a removed file, and its target is unknown.
            if fs::symlink_metadata(path).is_ok() {
                return None;
            }
            fs::canonicalize(path.parent()?)
                .ok()?
                .join(path.file_name()?)
        }
    };
    if !canonical.starts_with(&real_root) {
        return None;
    }
    let mut segments = Vec::new();
    for component in rel.components() {
        match component {
            Component::Normal(name) => segments.push(name.to_str()?),
            _ => return None,
        }
    }
    let file = *segments.last()?;
    if !file.ends_with(REQUEST_EXT) {
        return None;
    }
    let base = resources::base(collection);
    let escaped = |parts: &[&str]| -> String {
        parts
            .iter()
            .map(|s| resources::encode(s))
            .collect::<Vec<_>>()
            .join("/")
    };
    match segments.as_slice() {
        [ROOT_FILE] => Some(base),
        [ENVIRONMENTS_DIR, name] => Some(format!(
            "{base}{ENVIRONMENT}/{}",
            resources::encode(name.strip_suffix(REQUEST_EXT)?)
        )),
        [dirs @ .., FOLDER_FILE] => Some(if dirs.is_empty() {
            base
        } else {
            format!("{base}{}/", escaped(dirs))
        }),
        all => Some(format!("{base}{}", escaped(all))),
    }
}

/// The URIs present in `after` but not `before` (added) and the reverse (removed).
pub(crate) fn diff_uris(before: &[String], after: &[String]) -> (Vec<String>, Vec<String>) {
    let old: HashSet<&String> = before.iter().collect();
    let new: HashSet<&String> = after.iter().collect();
    let added = after.iter().filter(|u| !old.contains(u)).cloned().collect();
    let removed = before
        .iter()
        .filter(|u| !new.contains(u))
        .cloned()
        .collect();
    (added, removed)
}

/// Keeps the watcher and its notifier task alive; dropping it stops both.
pub(crate) struct WatcherHandle {
    _watcher: Arc<CollectionWatcher>,
    task: JoinHandle<()>,
}

impl Drop for WatcherHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// What one pass over the reloaded collection found.
struct Snapshot {
    uris: Vec<String>,
    relisted: bool,
    dirs: BTreeSet<std::path::PathBuf>,
    changed: BTreeSet<String>,
}

fn snapshot(
    server: &ApinataServer,
    collection: &Collection,
    paths: &[std::path::PathBuf],
) -> Snapshot {
    Snapshot {
        uris: resources::list(collection)
            .into_iter()
            .map(|r| r.uri)
            .collect(),
        relisted: server.record_listing(resources::signature(collection)),
        dirs: collection.watch_dirs().into_iter().collect(),
        changed: paths
            .iter()
            .filter_map(|p| uri_for_path(collection, p))
            .collect(),
    }
}

/// How long the watcher waits for further batches before it takes a snapshot.
const SETTLE: Duration = Duration::from_millis(300);
/// The most a steady stream of batches may delay a snapshot.
const MAX_HOLD: Duration = Duration::from_secs(3);

/// `first` plus every batch that follows until `settle` passes quietly or `max_hold` has
/// elapsed since `first`, so one logical change is handled in one pass.
async fn gather(
    rx: &mut mpsc::UnboundedReceiver<Change>,
    first: Change,
    settle: Duration,
    max_hold: Duration,
) -> Vec<Change> {
    let deadline = Instant::now() + max_hold;
    let mut batch = vec![first];
    loop {
        while let Ok(more) = rx.try_recv() {
            batch.push(more);
        }
        if batch.iter().any(|c| matches!(c, Change::Failed(_))) {
            return batch;
        }
        let wait = settle.min(deadline.saturating_duration_since(Instant::now()));
        if wait.is_zero() {
            return batch;
        }
        match timeout(wait, rx.recv()).await {
            Ok(Some(more)) => batch.push(more),
            _ => return batch,
        }
    }
}

/// Wait for in-flight tool writes, but no longer than `bound`, so a long-running mutating tool
/// cannot stall the watcher.
async fn wait_for_writes(gate: &WriteGate, bound: Duration) {
    let _ = timeout(bound, gate.wait_idle()).await;
}

/// Start watching the server's collection and notify `peer`. `None` when the platform watcher
/// cannot start; tools still work, the client just gets no pushes.
pub(crate) async fn start(server: ApinataServer, peer: Peer<RoleServer>) -> Option<WatcherHandle> {
    let this = server.clone();
    let initial = server
        .with_collection(move |c| Ok((c.root().to_path_buf(), snapshot(&this, c, &[]))))
        .await
        .ok()?;
    let (root, first) = initial;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let watcher = match CollectionWatcher::start(root, first.dirs, move |change| {
        let _ = tx.send(change);
    }) {
        Ok(w) => Arc::new(w),
        Err(message) => {
            eprintln!("apinata-mcp: file watching is off: {message}");
            return None;
        }
    };
    let handle = Arc::clone(&watcher);
    let task = tokio::spawn(async move {
        let mut known = first.uris;
        while let Some(change) = rx.recv().await {
            // A logical change arrives as several batches; one pass after a quiet window
            // handles them all, and only once no tool write is half done.
            let batch = gather(&mut rx, change, SETTLE, MAX_HOLD).await;
            wait_for_writes(&server.write_gate(), MAX_HOLD).await;
            let mut rescan = false;
            let mut paths = Vec::new();
            for change in batch {
                match change {
                    Change::Paths(p) => paths.extend(p),
                    Change::Rescan => rescan = true,
                    Change::Failed(message) => {
                        eprintln!("apinata-mcp: file watching stopped: {message}");
                        return;
                    }
                    _ => rescan = true,
                }
            }
            let this = server.clone();
            let Ok(now) = server
                .with_collection(move |c| Ok(snapshot(&this, c, &paths)))
                .await
            else {
                continue;
            };
            watcher.set_dirs(now.dirs);
            let (_, removed) = diff_uris(&known, &now.uris);
            known = now.uris;
            let subscribed = server.subscribed();
            let mut updated: BTreeSet<String> = if rescan {
                subscribed.iter().cloned().collect()
            } else {
                now.changed
                    .into_iter()
                    .chain(removed.iter().cloned())
                    .filter(|u| subscribed.contains(u))
                    .collect()
            };
            // Whether the list changed is the recorded listing's verdict alone, even after a
            // rescan, so a change a tool already announced is not announced again.
            if peer_gone(&peer, now.relisted).await {
                return;
            }
            for uri in std::mem::take(&mut updated) {
                if peer
                    .notify_resource_updated(ResourceUpdatedNotificationParam::new(uri))
                    .await
                    .is_err()
                {
                    return;
                }
            }
        }
    });
    Some(WatcherHandle {
        _watcher: handle,
        task,
    })
}

/// Send `list_changed` when `announce`; true when the client is gone and the task should end.
async fn peer_gone(peer: &Peer<RoleServer>, announce: bool) -> bool {
    if peer.is_transport_closed() {
        return true;
    }
    announce && peer.notify_resource_list_changed().await.is_err()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const ROOT_YML: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: shop\n";

    fn collection(files: &[&str]) -> (TempDir, Collection) {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(ROOT_FILE), ROOT_YML).unwrap();
        for rel in files {
            let path = dir.path().join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "info:\n  name: x\n").unwrap();
        }
        let c = Collection::open(dir.path()).unwrap();
        (dir, c)
    }

    fn uri(c: &Collection, d: &TempDir, rel: &str) -> Option<String> {
        uri_for_path(c, &d.path().join(rel))
    }

    #[test]
    fn maps_each_kind_of_file() {
        let (d, c) = collection(&["a/b.yml", "a/folder.yml", "environments/dev.yml", "top.yml"]);
        assert_eq!(uri(&c, &d, "top.yml").unwrap(), "apinata://shop/top.yml");
        assert_eq!(uri(&c, &d, "a/b.yml").unwrap(), "apinata://shop/a/b.yml");
        assert_eq!(uri(&c, &d, "a/folder.yml").unwrap(), "apinata://shop/a/");
        assert_eq!(uri(&c, &d, "folder.yml").unwrap(), "apinata://shop/");
        assert_eq!(
            uri(&c, &d, "opencollection.yml").unwrap(),
            "apinata://shop/"
        );
        assert_eq!(
            uri(&c, &d, "environments/dev.yml").unwrap(),
            "apinata://shop/@environment/dev"
        );
    }

    #[test]
    fn drops_ignored_and_foreign_files() {
        let (d, c) = collection(&[".git/x.yml", ".env", "notes.txt"]);
        assert_eq!(uri(&c, &d, ".git/x.yml"), None);
        assert_eq!(uri(&c, &d, ".git/HEAD"), None);
        assert_eq!(uri(&c, &d, ".env"), None);
        assert_eq!(uri(&c, &d, "notes.txt"), None);
        assert_eq!(uri_for_path(&c, Path::new("/etc/passwd")), None);
    }

    #[test]
    fn a_removed_file_still_maps_through_its_parent() {
        let (d, c) = collection(&["a/b.yml"]);
        fs::remove_file(d.path().join("a/b.yml")).unwrap();
        assert_eq!(uri(&c, &d, "a/b.yml").unwrap(), "apinata://shop/a/b.yml");
        assert_eq!(uri(&c, &d, "gone/c.yml"), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_out_of_the_root_is_not_a_resource() {
        let outside = TempDir::new().unwrap();
        fs::write(outside.path().join("t.yml"), "x: 1\n").unwrap();
        let (d, c) = collection(&["a/b.yml"]);
        std::os::unix::fs::symlink(outside.path().join("t.yml"), d.path().join("link.yml"))
            .unwrap();
        std::os::unix::fs::symlink(outside.path(), d.path().join("dir")).unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("nope.yml"),
            d.path().join("dangling.yml"),
        )
        .unwrap();
        assert_eq!(uri(&c, &d, "link.yml"), None);
        assert_eq!(uri(&c, &d, "dir/t.yml"), None);
        assert_eq!(uri(&c, &d, "dangling.yml"), None);
        assert!(uri(&c, &d, "a/b.yml").is_some());
    }

    #[test]
    fn diff_reports_added_and_removed() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let (added, removed) = diff_uris(&s(&["a", "b"]), &s(&["b", "c"]));
        assert_eq!((added, removed), (s(&["c"]), s(&["a"])));
        let (added, removed) = diff_uris(&s(&["a"]), &s(&["a"]));
        assert!(added.is_empty() && removed.is_empty());
    }

    use std::path::PathBuf;
    use tokio::time::sleep;

    fn change(name: &str) -> Change {
        Change::Paths(vec![PathBuf::from(name)])
    }

    fn names(batch: &[Change]) -> Vec<String> {
        batch
            .iter()
            .map(|c| match c {
                Change::Paths(p) => p[0].display().to_string(),
                _ => "other".into(),
            })
            .collect()
    }

    const MS: fn(u64) -> Duration = Duration::from_millis;

    #[tokio::test(start_paused = true)]
    async fn gather_returns_a_lone_change_after_the_settle_window() {
        let (_tx, mut rx) = mpsc::unbounded_channel();
        let start = Instant::now();
        let batch = gather(&mut rx, change("a"), MS(300), MS(3000)).await;
        assert_eq!(names(&batch), ["a"]);
        assert_eq!(start.elapsed(), MS(300));
    }

    #[tokio::test(start_paused = true)]
    async fn gather_merges_changes_within_the_window_and_restarts_it() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            sleep(MS(200)).await;
            tx.send(change("b")).unwrap();
            sleep(MS(200)).await;
            tx.send(change("c")).unwrap();
            // Keep the sender alive past the window so the channel stays open.
            sleep(MS(5000)).await;
        });
        let start = Instant::now();
        let batch = gather(&mut rx, change("a"), MS(300), MS(3000)).await;
        assert_eq!(names(&batch), ["a", "b", "c"]);
        assert_eq!(start.elapsed(), MS(700));
    }

    #[tokio::test(start_paused = true)]
    async fn gather_drains_changes_already_queued() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        tx.send(change("b")).unwrap();
        tx.send(change("c")).unwrap();
        let batch = gather(&mut rx, change("a"), MS(300), MS(3000)).await;
        assert_eq!(names(&batch), ["a", "b", "c"]);
    }

    #[tokio::test(start_paused = true)]
    async fn gather_stops_at_max_hold_under_a_steady_stream() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            for i in 0..100 {
                sleep(MS(100)).await;
                if tx.send(change(&format!("n{i}"))).is_err() {
                    return;
                }
            }
        });
        let start = Instant::now();
        let batch = gather(&mut rx, change("a"), MS(300), MS(1000)).await;
        assert_eq!(start.elapsed(), MS(1000));
        assert!(batch.len() > 5, "merged only {}", batch.len());
        assert!(batch.len() <= 11, "ran past the hold: {}", batch.len());
    }

    #[tokio::test(start_paused = true)]
    async fn a_held_write_releases_the_wait_at_the_bound() {
        let gate = WriteGate::default();
        let _held = gate.enter();
        let start = Instant::now();
        wait_for_writes(&gate, MS(3000)).await;
        assert_eq!(start.elapsed(), MS(3000));
    }

    #[tokio::test(start_paused = true)]
    async fn gather_returns_at_once_on_a_failed_watcher() {
        let (_tx, mut rx) = mpsc::unbounded_channel();
        let start = Instant::now();
        let batch = gather(&mut rx, Change::Failed("gone".into()), MS(300), MS(3000)).await;
        assert!(matches!(batch[0], Change::Failed(_)));
        assert_eq!(start.elapsed(), Duration::ZERO);
    }
}
