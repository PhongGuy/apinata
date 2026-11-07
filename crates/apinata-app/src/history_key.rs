//! The history key: one rule naming where a tab's sends are kept and read.

use std::hash::{BuildHasher as _, Hasher as _, RandomState};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use apinata_core::ItemPath;

use crate::last_response;
use crate::tabs::TabId;
use crate::workspace::Workspace;

/// Where tab `tab`'s sends are kept: a saved request's own path while its file exists,
/// the tab's history key for a draft that has one, else None.
pub fn history_key(ws: &Workspace, tab: TabId) -> Option<(PathBuf, ItemPath)> {
    let entry = ws.tabs().get(tab)?;
    let item = entry.item()?;
    if last_response::keeps(&entry.root, item) {
        return Some((entry.root.clone(), item.clone()));
    }
    let key = ws.tabs().history_key(tab)?;
    Some((entry.root.clone(), key.clone()))
}

/// `untitled-<ms since epoch><6 random digits>.draft` at the collection root.
// Letters, digits, a dash and a dot always form a valid root-level name.
#[allow(clippy::expect_used)]
pub fn new_history_key(now: SystemTime) -> ItemPath {
    let ms = now
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let digits = RandomState::new().build_hasher().finish() % 1_000_000;
    ItemPath::new(&format!("untitled-{ms}{digits:06}.draft")).expect("a valid root-level name")
}
