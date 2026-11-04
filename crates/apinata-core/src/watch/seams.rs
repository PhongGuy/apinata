//! The set of non-recursive watches for one collection, kept in step with core's directory list,
//! and the rule that turns a debounced batch into a change.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify_debouncer_full::notify::{
    self, ErrorKind, EventKind, RecommendedWatcher, RecursiveMode,
};
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};

/// How long the debouncer waits for a path to go quiet before reporting it.
pub const DEBOUNCE: Duration = Duration::from_millis(250);

/// Why a directory could not be watched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchError {
    /// The directory is missing or unreadable; try again later.
    Unwatchable,
    /// Watching cannot continue; carries the error for display.
    Failed(String),
}

/// Places and removes non-recursive watches on single directories.
pub trait Watch: Send + 'static {
    /// Watch `dir` without its subdirectories.
    fn watch(&mut self, dir: &Path) -> Result<(), WatchError>;
    /// Stop watching `dir`; errors are ignored.
    fn unwatch(&mut self, dir: &Path);
}

/// Receives each debounced batch of events.
pub type EventHandler = Box<dyn FnMut(DebounceEventResult) + Send>;

/// Builds a `Watch` that reports to the given handler, or the error that prevented it.
pub type WatchFactory = Arc<dyn Fn(EventHandler) -> Result<Box<dyn Watch>, String> + Send + Sync>;

/// The watches placed for one collection.
pub struct WatchSet {
    watch: Box<dyn Watch>,
    root: PathBuf,
    placed: BTreeSet<PathBuf>,
}

impl WatchSet {
    /// A set with no watches yet, for the collection at `root`.
    pub fn new(watch: Box<dyn Watch>, root: PathBuf) -> WatchSet {
        WatchSet {
            watch,
            root,
            placed: BTreeSet::new(),
        }
    }

    /// The directories currently watched.
    pub fn placed(&self) -> &BTreeSet<PathBuf> {
        &self.placed
    }

    /// Unwatch placed dirs not desired; re-watch `replace` dirs that are placed and desired;
    /// watch desired dirs not placed. Ok: the dirs placed by this call, sorted. Err: the first
    /// `Failed`, or the root being `Unwatchable`.
    pub fn reconcile(
        &mut self,
        desired: &BTreeSet<PathBuf>,
        replace: &[PathBuf],
    ) -> Result<Vec<PathBuf>, String> {
        for dir in self.placed.difference(desired).cloned().collect::<Vec<_>>() {
            self.watch.unwatch(&dir);
            self.placed.remove(&dir);
        }
        // A replaced directory leaves a dead watch behind, so it is dropped from the placed
        // set and placed afresh below with the other missing ones.
        let replaced: BTreeSet<&PathBuf> = replace
            .iter()
            .filter(|d| self.placed.contains(*d))
            .collect();
        for dir in &replaced {
            self.placed.remove(*dir);
        }
        let mut added = Vec::new();
        for dir in desired
            .difference(&self.placed)
            .cloned()
            .collect::<Vec<_>>()
        {
            match self.watch.watch(&dir) {
                Ok(()) => {
                    self.placed.insert(dir.clone());
                    added.push(dir);
                }
                Err(WatchError::Unwatchable) if dir == self.root => {
                    return Err(format!(
                        "cannot watch the collection folder {}",
                        dir.display()
                    ));
                }
                // Missing or unreadable for now; the next reconcile tries again.
                Err(WatchError::Unwatchable) => {}
                Err(WatchError::Failed(message)) => return Err(message),
            }
        }
        Ok(added)
    }
}

/// The entries of each directory, sorted and deduplicated; an unreadable directory adds none.
pub fn gap_paths(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let entries: BTreeSet<PathBuf> = dirs
        .iter()
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flat_map(|read| read.filter_map(|entry| entry.ok().map(|e| e.path())))
        .collect();
    entries.into_iter().collect()
}

/// Sort a watch error: a missing or unreadable directory is `Unwatchable`, anything else
/// (the watch limit among them) is `Failed`.
pub fn classify(error: &notify::Error) -> WatchError {
    match &error.kind {
        ErrorKind::PathNotFound => WatchError::Unwatchable,
        ErrorKind::Io(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
            ) =>
        {
            WatchError::Unwatchable
        }
        _ => WatchError::Failed(error.to_string()),
    }
}

/// A `Watch` over the platform's watcher, debounced by `DEBOUNCE`.
pub struct NotifyWatch {
    debouncer: Debouncer<RecommendedWatcher, RecommendedCache>,
}

impl Watch for NotifyWatch {
    fn watch(&mut self, dir: &Path) -> Result<(), WatchError> {
        self.debouncer
            .watch(dir, RecursiveMode::NonRecursive)
            .map_err(|e| classify(&e))
    }

    fn unwatch(&mut self, dir: &Path) {
        // A deleted or renamed directory reports `WatchNotFound`; it is already gone.
        let _ = self.debouncer.unwatch(dir);
    }
}

/// The factory the app uses: real watches on the platform's watcher.
pub fn notify_factory() -> WatchFactory {
    Arc::new(|handler: EventHandler| {
        let debouncer = new_debouncer(DEBOUNCE, None, handler).map_err(|e| e.to_string())?;
        Ok(Box::new(NotifyWatch { debouncer }) as Box<dyn Watch>)
    })
}

/// What one debounced batch asks for, before a shell adds its own variants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawChange {
    /// Changed paths, all under the collection root, sorted and deduplicated.
    Paths(Vec<PathBuf>),
    /// Events were lost; reload the whole collection.
    Rescan,
    /// Watching cannot continue; carries the error for display.
    Failed(String),
}

/// Translate one debounced batch for the collection at `root`, whose canonical form is
/// `real_root`. Returns `None` when the batch asks for nothing.
pub fn translate_batch(
    result: DebounceEventResult,
    root: &Path,
    real_root: &Path,
) -> Option<RawChange> {
    let events = match result {
        Ok(events) => events,
        // Only the watch limit is fatal; other stream errors leave watching intact.
        Err(errors) => {
            return errors
                .iter()
                .find(|e| matches!(e.kind, ErrorKind::MaxFilesWatch))
                .map(|e| RawChange::Failed(e.to_string()));
        }
    };
    if events.iter().any(|e| e.need_rescan()) {
        return Some(RawChange::Rescan);
    }
    let paths: BTreeSet<PathBuf> = events
        .iter()
        .filter(|e| !matches!(e.kind, EventKind::Access(_)))
        .flat_map(|e| e.paths.iter())
        .filter_map(|p| under_root(p, root, real_root))
        .collect();
    (!paths.is_empty()).then(|| RawChange::Paths(paths.into_iter().collect()))
}

/// Map `path` under `root`: kept as is when already there, rebased when it arrives in the
/// root's canonical form (FSEvents reports resolved paths), dropped otherwise.
fn under_root(path: &Path, root: &Path, real_root: &Path) -> Option<PathBuf> {
    if path.starts_with(root) {
        return Some(path.to_path_buf());
    }
    path.strip_prefix(real_root).ok().map(|rel| root.join(rel))
}
