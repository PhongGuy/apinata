//! Watching a collection's files for external edits.

mod collection_watcher;
pub use collection_watcher::{Change, CollectionWatcher};

// Low-level seams shared with the desktop app; they expose the watcher crate's types.
#[doc(hidden)]
pub mod seams;
