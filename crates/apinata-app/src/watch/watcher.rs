//! One collection's watcher: a background task that keeps its watches in step with the
//! directories core reports and passes each debounced batch on as a `WatchMsg`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use async_channel::{Receiver, Sender};
use gpui_kit::{App, Task};
use notify_debouncer_full::DebounceEventResult;

use notify_debouncer_full::notify::EventKind;

use crate::watch::git;
use crate::watch::rules::{WatchMsg, translate};
use crate::watch::set::{EventHandler, WatchFactory, WatchSet, gap_paths};

enum Input {
    Dirs(BTreeSet<PathBuf>),
    Events(DebounceEventResult),
    GitEvents(DebounceEventResult),
}

/// Watches one collection until dropped, which ends its task and every watch it placed.
pub struct CollectionWatcher {
    inputs: Sender<Input>,
    _task: Task<()>,
}

impl CollectionWatcher {
    /// Starts watching the collection at `root` with watches from `factory`, sending what it
    /// sees to `out`. Nothing is watched until the first `set_dirs`.
    pub fn start(
        root: PathBuf,
        factory: WatchFactory,
        out: Sender<WatchMsg>,
        cx: &App,
    ) -> CollectionWatcher {
        let (inputs, rx) = async_channel::unbounded();
        let events = inputs.clone();
        let handler: EventHandler = Box::new(move |r| {
            let _ = events.try_send(Input::Events(r));
        });
        let task = cx.background_executor().spawn(watch_loop(
            root,
            factory,
            (handler, inputs.clone()),
            rx,
            out,
        ));
        CollectionWatcher {
            inputs,
            _task: task,
        }
    }

    /// Replaces the set of directories to watch.
    pub fn set_dirs(&self, dirs: BTreeSet<PathBuf>) {
        let _ = self.inputs.try_send(Input::Dirs(dirs));
    }
}

// Owns the watches. Each input is either a new directory set from the workspace or a debounced
// batch; after either, the set is reconciled and every newly placed directory's entries reloaded.
async fn watch_loop(
    root: PathBuf,
    factory: WatchFactory,
    (handler, git_inputs): (EventHandler, Sender<Input>),
    inputs: Receiver<Input>,
    out: Sender<WatchMsg>,
) {
    let watch = match factory(handler) {
        Ok(watch) => watch,
        Err(message) => {
            let _ = out.send(WatchMsg::Failed(message)).await;
            return;
        }
    };
    let real_root = std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
    let mut set = WatchSet::new(watch, root.clone());
    // The git watches sit in a watcher of their own, placed before the first input so that
    // anything done after the collection loads is seen. A repository that cannot be watched
    // only costs the automatic refresh, never the collection's own watching.
    let mut git_watch = None;
    sync_git(&root, &factory, &git_inputs, &mut git_watch);
    let mut desired = BTreeSet::new();
    while let Ok(input) = inputs.recv().await {
        // An event naming a watched directory itself means it was removed or replaced, and its
        // old watch is dead, so it is placed again below.
        let mut replace: Vec<PathBuf> = Vec::new();
        match input {
            Input::GitEvents(result) => {
                let touched = result.is_ok_and(|events| {
                    events
                        .iter()
                        .filter(|e| !matches!(e.kind, EventKind::Access(_)))
                        .flat_map(|e| e.paths.iter())
                        .any(|p| git::relevant(p))
                });
                if touched && out.send(WatchMsg::Git).await.is_err() {
                    return;
                }
                continue;
            }
            Input::Dirs(dirs) => desired = dirs,
            Input::Events(result) => {
                // A repository made or removed after opening shows up as a change in the
                // collection, so its watches follow; a new one is read at once.
                if sync_git(&root, &factory, &git_inputs, &mut git_watch)
                    && out.send(WatchMsg::Git).await.is_err()
                {
                    return;
                }
                match translate(result, &root, &real_root) {
                    Some(WatchMsg::Failed(m)) => {
                        let _ = out.send(WatchMsg::Failed(m)).await;
                        return;
                    }
                    Some(msg) => {
                        if let WatchMsg::Paths(paths) = &msg {
                            replace = paths
                                .iter()
                                .filter(|p| set.placed().contains(*p))
                                .cloned()
                                .collect();
                        }
                        if out.send(msg).await.is_err() {
                            return;
                        }
                    }
                    None => {}
                }
            }
        }
        match set.reconcile(&desired, &replace) {
            Ok(placed) => {
                // A re-placed directory may have lost files that its entries cannot name, so it
                // is reread as a whole tree rather than as a gap.
                let msg = if placed.iter().any(|p| replace.contains(p)) {
                    WatchMsg::Rescan
                } else {
                    let gap = gap_paths(&placed);
                    if gap.is_empty() {
                        continue;
                    }
                    WatchMsg::Paths(gap)
                };
                if out.send(msg).await.is_err() {
                    return;
                }
            }
            Err(m) => {
                let _ = out.send(WatchMsg::Failed(m)).await;
                return;
            }
        }
    }
}

// Keeps the git watches on the directories the repository has now, building their watcher on
// first need. True when a directory was newly placed, so its state may not have been read yet.
fn sync_git(
    root: &Path,
    factory: &WatchFactory,
    inputs: &Sender<Input>,
    git_watch: &mut Option<WatchSet>,
) -> bool {
    let dirs = git::dirs(root);
    if git_watch.is_none() {
        if dirs.is_empty() {
            return false;
        }
        let inputs = inputs.clone();
        let handler: EventHandler = Box::new(move |r| {
            let _ = inputs.try_send(Input::GitEvents(r));
        });
        let Ok(watch) = factory(handler) else {
            return false;
        };
        *git_watch = Some(WatchSet::new(watch, PathBuf::new()));
    }
    git_watch
        .as_mut()
        .and_then(|set| set.reconcile(&dirs, &[]).ok())
        .is_some_and(|placed| !placed.is_empty())
}
