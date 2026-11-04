//! Pure rules that turn one debounced batch of file events into a message for the watcher.

use std::path::{Path, PathBuf};

use apinata_core::watch::seams::{RawChange, translate_batch};
use notify_debouncer_full::DebounceEventResult;

/// What one debounced batch asks the watcher to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchMsg {
    /// Reload these paths, all under the collection root, sorted and deduplicated.
    Paths(Vec<PathBuf>),
    /// The repository's `HEAD`, index or a branch head changed; refresh the git status.
    Git,
    /// Events were lost; reload the whole collection.
    Rescan,
    /// Watching cannot continue; carries the error for display.
    Failed(String),
}

/// Translate one debounced batch for the collection at `root`, whose canonical form is
/// `real_root`. Returns `None` when the batch asks for nothing.
pub fn translate(result: DebounceEventResult, root: &Path, real_root: &Path) -> Option<WatchMsg> {
    translate_batch(result, root, real_root).map(|change| match change {
        RawChange::Paths(paths) => WatchMsg::Paths(paths),
        RawChange::Rescan => WatchMsg::Rescan,
        RawChange::Failed(message) => WatchMsg::Failed(message),
    })
}
