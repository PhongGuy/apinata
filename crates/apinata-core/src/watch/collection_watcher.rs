//! A collection's file watcher with no dependency types in its signatures.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};

use notify_debouncer_full::DebounceEventResult;

use super::seams::{EventHandler, RawChange, WatchSet, gap_paths, notify_factory, translate_batch};

/// What happened to the watched files.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Change {
    /// Paths that changed, all under the root, sorted and deduplicated.
    Paths(Vec<PathBuf>),
    /// Events were lost; reread the whole collection.
    Rescan,
    /// Watching cannot continue and has stopped; carries the error for display.
    Failed(String),
}

enum Input {
    Dirs(BTreeSet<PathBuf>),
    Events(DebounceEventResult),
    Stop,
}

/// Watches one collection's directories until dropped. Changes are passed to the callback
/// on the watcher's own thread, after the debounce. After a `Change::Failed` the thread has
/// ended and later `set_dirs` calls are ignored.
pub struct CollectionWatcher {
    inputs: Sender<Input>,
    stopped: Arc<AtomicBool>,
}

impl CollectionWatcher {
    /// Start watching `dirs` (non-recursively) of the collection at `root`, calling
    /// `on_change` for each batch. Err when the platform watcher cannot be created.
    pub fn start(
        root: PathBuf,
        dirs: BTreeSet<PathBuf>,
        on_change: impl FnMut(Change) + Send + 'static,
    ) -> Result<CollectionWatcher, String> {
        let (inputs, rx) = channel();
        let events = inputs.clone();
        let handler: EventHandler = Box::new(move |r| {
            let _ = events.send(Input::Events(r));
        });
        let watch = notify_factory()(handler)?;
        let stopped = Arc::new(AtomicBool::new(false));
        let flag = stopped.clone();
        let _ = inputs.send(Input::Dirs(dirs));
        std::thread::Builder::new()
            .name("collection-watcher".into())
            .spawn(move || {
                let mut on_change = on_change;
                let mut emit = |c: Change| {
                    if !flag.load(Ordering::SeqCst) {
                        on_change(c);
                    }
                };
                let real_root = std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
                let mut set = WatchSet::new(watch, root.clone());
                let mut desired = BTreeSet::new();
                // The loop ends on `Stop` or a failure; ending drops the watches.
                while let Ok(input) = rx.recv() {
                    if flag.load(Ordering::SeqCst) {
                        return;
                    }
                    // A watched directory named by an event was removed or replaced, so its
                    // old watch is dead and it is placed again below.
                    let mut replace: Vec<PathBuf> = Vec::new();
                    match input {
                        Input::Stop => return,
                        Input::Dirs(dirs) => desired = dirs,
                        Input::Events(result) => match translate_batch(result, &root, &real_root) {
                            Some(RawChange::Failed(m)) => return emit(Change::Failed(m)),
                            Some(RawChange::Paths(paths)) => {
                                replace = paths
                                    .iter()
                                    .filter(|p| set.placed().contains(*p))
                                    .cloned()
                                    .collect();
                                emit(Change::Paths(paths));
                            }
                            Some(RawChange::Rescan) => emit(Change::Rescan),
                            None => {}
                        },
                    }
                    match set.reconcile(&desired, &replace) {
                        Ok(placed) => {
                            // Entries that predate a new watch raise no event, so they are
                            // reported here; a re-placed directory may have lost files that
                            // entries cannot name, so it asks for a full rescan instead.
                            if placed.iter().any(|p| replace.contains(p)) {
                                emit(Change::Rescan);
                            } else {
                                let mut gap = gap_paths(&placed);
                                gap.retain(|p| p.starts_with(&root));
                                if !gap.is_empty() {
                                    emit(Change::Paths(gap));
                                }
                            }
                        }
                        Err(m) => return emit(Change::Failed(m)),
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(CollectionWatcher { inputs, stopped })
    }

    /// Replace the set of watched directories.
    pub fn set_dirs(&self, dirs: BTreeSet<PathBuf>) {
        let _ = self.inputs.send(Input::Dirs(dirs));
    }
}

impl Drop for CollectionWatcher {
    fn drop(&mut self) {
        // The flag silences the callback at once and `Stop` wakes the thread so it ends now.
        // The notify handler holds a sender too, so the channel never disconnects on its own.
        // There is no join, so dropping inside the callback cannot deadlock.
        self.stopped.store(true, Ordering::SeqCst);
        let _ = self.inputs.send(Input::Stop);
    }
}
