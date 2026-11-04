//! Files and folders dropped from the desktop onto the window: what each one is, and the
//! overlay shown while they hover.

use std::path::{Path, PathBuf};

use apinata_core::import::{ImportBundle, ImportedRequest, RequestFormat, parse_request};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    InteractiveElement as _, IntoElement, ParentElement as _, Styled as _, TestSupportExt as _, div,
};

use crate::style::SPACE_2;

/// The largest file read to find out what it is; a bigger one is refused, not loaded.
const MAX_SNIFF: u64 = 64 * 1024 * 1024;

/// What one dropped path turned out to be.
pub enum Dropped {
    /// A folder, opened as a collection.
    Folder(PathBuf),
    /// A file holding collections, imported where the user chooses.
    Collections(Vec<ImportBundle>),
    /// A file holding one curl command, imported as a draft.
    Request(Box<ImportedRequest>),
    /// A file that cannot be imported; the text names it and says why.
    Refused(String),
}

/// Reads `path` to decide what it is. Blocking: run it off the render path.
pub fn classify(path: &Path) -> Dropped {
    if path.is_dir() {
        return Dropped::Folder(path.to_path_buf());
    }
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let refuse = |why: &str| Dropped::Refused(format!("Could not import {name}: {why}"));
    match std::fs::metadata(path) {
        Ok(m) if m.len() > MAX_SNIFF => return refuse("the file is too large"),
        Ok(_) => {}
        Err(e) => return refuse(&e.to_string()),
    }
    let input = match std::fs::read(path) {
        Ok(input) => input,
        Err(e) => return refuse(&e.to_string()),
    };
    // A curl command is plain text, so it is tried first and a collection file never matches it.
    if let Ok(text) = std::str::from_utf8(&input)
        && RequestFormat::detect(text).is_some()
    {
        return match parse_request(None, text) {
            Ok(parsed) => Dropped::Request(Box::new(parsed.request)),
            Err(e) => refuse(&crate::ui::plain_quotes(&e.to_string())),
        };
    }
    match crate::shell::parse_bundles_from(path, &input) {
        Ok(bundles) => Dropped::Collections(bundles),
        Err(message) => refuse(&message),
    }
}

/// The veil shown over the window while files hover: it names what a drop does. It takes no
/// pointer input, so the drop still lands on the window beneath.
pub fn overlay(cx: &gpui_kit::App) -> impl IntoElement {
    let theme = cx.theme();
    div()
        .id("drop-overlay")
        .test_support()
        .absolute()
        .inset_0()
        .m(SPACE_2)
        .flex()
        .items_center()
        .justify_center()
        .rounded_lg()
        .border_2()
        .border_dashed()
        .border_color(theme.primary)
        .bg(theme.background.opacity(0.85))
        .text_color(theme.foreground)
        .child("Drop a folder to open it, or a file to import it")
}
