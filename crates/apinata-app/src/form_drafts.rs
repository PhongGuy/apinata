//! What was typed into a transient form and closed without submitting, kept in memory for the
//! session so reopening the form brings it back. Nothing here reaches disk.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use apinata_core::ItemPath;
use gpui_kit::{App, Global};

use crate::clone_form::shown_url;

// Commit messages by collection root.
#[derive(Default)]
struct CommitDrafts(HashMap<PathBuf, String>);
impl Global for CommitDrafts {}

/// A clone form closed before cloning.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CloneDraft {
    /// The URL; empty when the typed one held credentials.
    pub url: String,
    /// The folder name.
    pub name: String,
}
impl Global for CloneDraft {}

/// A Save example form closed before saving.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExampleDraft {
    /// The example name.
    pub name: String,
    /// The description.
    pub description: String,
}

// Example forms by collection root and request.
#[derive(Default)]
struct ExampleDrafts(HashMap<(PathBuf, ItemPath), ExampleDraft>);
impl Global for ExampleDrafts {}

/// The commit message kept for the collection at `root`.
pub fn commit(root: &Path, cx: &App) -> Option<String> {
    cx.try_global::<CommitDrafts>()
        .and_then(|d| d.0.get(root).cloned())
}

/// Keeps `text` as `root`'s commit message; blank text drops it.
pub fn set_commit(root: &Path, text: &str, cx: &mut App) {
    let drafts = &mut cx.default_global::<CommitDrafts>().0;
    if text.trim().is_empty() {
        drafts.remove(root);
    } else {
        drafts.insert(root.to_owned(), text.to_owned());
    }
}

/// Drops `root`'s commit message.
pub fn clear_commit(root: &Path, cx: &mut App) {
    set_commit(root, "", cx);
}

/// The kept clone form, if any.
pub fn clone_draft(cx: &App) -> Option<CloneDraft> {
    cx.try_global::<CloneDraft>().cloned()
}

/// Keeps the clone form's `url` and `name`; a URL with credentials is left out, and an empty
/// form keeps nothing.
pub fn set_clone(url: &str, name: &str, cx: &mut App) {
    let url = if shown_url(url) == url { url } else { "" };
    if url.trim().is_empty() && name.trim().is_empty() {
        clear_clone(cx);
    } else {
        cx.set_global(CloneDraft {
            url: url.to_owned(),
            name: name.to_owned(),
        });
    }
}

/// Drops the kept clone form.
pub fn clear_clone(cx: &mut App) {
    if cx.has_global::<CloneDraft>() {
        cx.remove_global::<CloneDraft>();
    }
}

/// The Save example form kept for `item` in the collection at `root`.
pub fn example(root: &Path, item: &ItemPath, cx: &App) -> Option<ExampleDraft> {
    cx.try_global::<ExampleDrafts>()
        .and_then(|d| d.0.get(&(root.to_owned(), item.clone())).cloned())
}

/// Keeps `draft` for `item` in the collection at `root`; `None` drops it.
pub fn set_example(root: &Path, item: &ItemPath, draft: Option<ExampleDraft>, cx: &mut App) {
    let drafts = &mut cx.default_global::<ExampleDrafts>().0;
    let key = (root.to_owned(), item.clone());
    match draft {
        Some(d) => drafts.insert(key, d),
        None => drafts.remove(&key),
    };
}
