//! The watch set now lives in core; this module keeps the app's import paths.

pub use apinata_core::watch::seams::{
    DEBOUNCE, EventHandler, NotifyWatch, Watch, WatchError, WatchFactory, WatchSet, classify,
    gap_paths, notify_factory,
};
