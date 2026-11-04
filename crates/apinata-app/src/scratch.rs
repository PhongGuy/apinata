//! The hidden scratch collection that holds drafts before any collection is open.

use apinata_core::Collection;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{App, Entity, Window};
use std::path::Path;

use crate::workspace::Workspace;

/// The scratch collection's display name.
pub const NAME: &str = "Scratch pad";

/// Makes sure `dir` holds an openable scratch collection. A directory that does not open is
/// deleted and recreated, but only when it is named `scratch`, so a wrong path loses nothing.
pub fn ensure(dir: &Path) -> Result<(), String> {
    if Collection::open(dir).is_ok() {
        return Ok(());
    }
    if dir.exists() {
        if dir.file_name().is_none_or(|n| n != "scratch") {
            return Err(format!("{} is not a scratch directory", dir.display()));
        }
        std::fs::remove_dir_all(dir).map_err(|e| e.to_string())?;
    }
    Collection::create(dir, NAME)
        .map(drop)
        .map_err(|e| e.to_string())
}

/// The notice for a collection-only command aimed at the scratch pad.
pub const NOT_A_COLLECTION: &str =
    "The scratch pad is not a collection. Save the request to a collection first.";

/// Whether `root` is the scratch pad; if so, raises [`NOT_A_COLLECTION`] and returns true so
/// the caller stops. Every collection-only handler asks this first.
pub fn refuse(
    workspace: &Entity<Workspace>,
    root: &Path,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    if !workspace.read(cx).is_scratch(root) {
        return false;
    }
    window.push_notification(Notification::warning(NOT_A_COLLECTION), cx);
    true
}
