//! Tree tools: create, clone, rename, move and delete. Each maps to one core tree call, which
//! picks Bruno's file names and `seq` values; writes that touch siblings are retried once.

use apinata_core::{
    Collection, ItemError, ItemPath, NewRequest, RequestKind, Trash, TrashLog, item_ops,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::writes::{Failure, conflict_message, write_retrying};

/// `create_request`'s arguments.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateRequestArgs {
    /// The folder to create it in; empty for the collection root.
    pub parent: String,
    /// The display name; the file name is derived from it.
    pub name: String,
    /// The request's kind; the collection's request-type preset when absent, else `http`.
    pub kind: Option<KindDoc>,
    /// The HTTP method; GET for http and POST for graphql when absent.
    pub method: Option<String>,
    /// The URL; the collection's preset URL when absent, else empty. Its query becomes params.
    pub url: Option<String>,
}

/// The kind of request `create_request` makes.
#[derive(Deserialize, JsonSchema, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub(crate) enum KindDoc {
    /// A plain HTTP request.
    Http,
    /// A GraphQL request, sent as a JSON POST.
    Graphql,
}

/// `create_folder`'s arguments.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateFolderArgs {
    /// The folder to create it in; empty for the collection root.
    pub parent: String,
    /// The display name; the directory name is derived from it.
    pub name: String,
}

/// `clone_request`, `clone_folder` and `rename_item` arguments.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct NameItemArgs {
    /// The item's collection-relative path, such as `users/get.yml`.
    pub path: String,
    /// The new display name; the file or directory name follows it.
    pub name: String,
}

/// `move_item`'s arguments: exactly one of `inside`, `before` and `after`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MoveArgs {
    /// The item to move.
    pub path: String,
    /// A folder to move it into; empty for the collection root.
    pub inside: Option<String>,
    /// A sibling to place it above.
    pub before: Option<String>,
    /// A sibling to place it below.
    pub after: Option<String>,
}

/// Where a tree tool left its item.
#[derive(Serialize, JsonSchema)]
pub(crate) struct ItemPathResult {
    /// The item's collection-relative path after the call.
    pub path: String,
}

/// What `delete_item` removed.
#[derive(Serialize, JsonSchema)]
pub(crate) struct Deleted {
    /// The path that was deleted.
    pub deleted: String,
    /// Where the item went: always `"trash"`.
    pub destination: String,
    /// Set when the item is gone but a sibling's `seq` could not be rewritten; absent otherwise.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// What `restore_item` put back.
#[derive(Serialize, JsonSchema)]
pub(crate) struct RestoredItem {
    /// The restored item's collection-relative path.
    pub restored: String,
    /// What went wrong after the item was back, such as a sibling left renumbered.
    pub warnings: Vec<String>,
}

/// What `restore_pruned` put back.
#[derive(Serialize, JsonSchema)]
pub(crate) struct RestoredPrune {
    /// The restored items' collection-relative paths, newest delete first.
    pub restored: Vec<String>,
    /// What went wrong along the way, such as the item that stopped the restore.
    pub warnings: Vec<String>,
}

fn at(path: ItemPath) -> ItemPathResult {
    ItemPathResult {
        path: path.to_string(),
    }
}

/// Create a request from the collection's presets with the given method and URL; core checks
/// the method before any file exists and retries its own save once.
pub(crate) fn create_request(
    c: &mut Collection,
    a: &CreateRequestArgs,
) -> Result<ItemPathResult, String> {
    let mut new = NewRequest::new();
    if let Some(kind) = a.kind {
        new = new.kind(match kind {
            KindDoc::Graphql => RequestKind::Graphql,
            KindDoc::Http => RequestKind::Http,
        });
    }
    if let Some(method) = &a.method {
        new = new.method(method.as_str());
    }
    if let Some(url) = &a.url {
        new = new.url(url.as_str());
    }
    item_ops::create_request(c, &a.parent, &a.name, &new)
        .map(at)
        .map_err(|e| create_failure(&e))
}

/// A failed create in the tool's words: a save that kept conflicting gets the same advice as
/// every other write that lost to the app twice.
fn create_failure(e: &ItemError) -> String {
    match e.unsaved_conflict() {
        Some((created, path)) => format!(
            "created {created} but could not set its method or URL: {}",
            conflict_message(path)
        ),
        None => e.to_string(),
    }
}

/// Create a folder in `a.parent` and return its path.
pub(crate) fn create_folder(
    c: &mut Collection,
    a: &CreateFolderArgs,
) -> Result<ItemPathResult, String> {
    item_ops::create_folder(c, &a.parent, &a.name)
        .map(at)
        .map_err(|e| e.to_string())
}

/// Copy the request or opaque item at `a.path` beside it under `a.name`; returns the copy's path.
pub(crate) fn clone_request(
    c: &mut Collection,
    a: &NameItemArgs,
) -> Result<ItemPathResult, String> {
    item_ops::clone_request(c, &a.path, &a.name)
        .map(at)
        .map_err(|e| e.to_string())
}

/// Copy the folder at `a.path` with everything in it beside it under `a.name` and return the
/// copy's path. It only creates files, so no conflict can need a retry.
pub(crate) fn clone_folder(c: &mut Collection, a: &NameItemArgs) -> Result<ItemPathResult, String> {
    item_ops::clone_folder(c, &a.path, &a.name)
        .map(at)
        .map_err(|e| e.to_string())
}

/// Rename a request or folder and its file or directory; an opaque item is refused by kind.
pub(crate) fn rename_item(c: &mut Collection, a: &NameItemArgs) -> Result<ItemPathResult, String> {
    write_retrying(c, |c| {
        item_ops::rename_item(c, &a.path, &a.name)
            .map(at)
            .map_err(Failure::item)
    })
}

/// Move an item into a folder or beside a sibling, renumbering as Bruno does.
pub(crate) fn move_item(c: &mut Collection, a: &MoveArgs) -> Result<ItemPathResult, String> {
    write_retrying(c, |c| {
        item_ops::move_item(
            c,
            &a.path,
            a.inside.as_deref(),
            a.before.as_deref(),
            a.after.as_deref(),
        )
        .map(at)
        .map_err(Failure::item)
    })
}

/// Move an item (a folder with everything in it) to `bin` and renumber its siblings. Refused
/// up front, changing nothing, when the bin is unusable, so a delete is never for good.
pub(crate) fn delete_item(
    c: &mut Collection,
    raw: &str,
    bin: &dyn Trash,
    log: Option<&TrashLog>,
) -> Result<Deleted, String> {
    if !bin.available() {
        return Err(TRASH_UNAVAILABLE.to_owned());
    }
    let warnings = write_retrying(c, |c| {
        item_ops::trash_item_logged(c, raw, bin, log).map_err(Failure::item)
    })?;
    Ok(Deleted {
        deleted: raw.to_owned(),
        destination: "trash".to_owned(),
        warnings,
    })
}

/// Put back the newest item this collection's deletes moved to `bin`; `None` when there is
/// no data directory to keep the record in.
pub(crate) fn restore_item(
    c: &mut Collection,
    bin: &dyn Trash,
    log: Option<&TrashLog>,
) -> Result<RestoredItem, String> {
    let log = log.ok_or("restoring is not available: no data directory")?;
    let restored = item_ops::restore_last_trashed(c, bin, log).map_err(|e| e.to_string())?;
    Ok(RestoredItem {
        restored: restored.item().to_string(),
        warnings: restored.warnings().to_vec(),
    })
}

/// Put back every item the import prune `batch` moved to `bin`, or the newest prune's without
/// one; refused when there is no data directory to keep the record in.
pub(crate) fn restore_pruned(
    c: &mut Collection,
    bin: &dyn Trash,
    log: Option<&TrashLog>,
    batch: Option<&str>,
) -> Result<RestoredPrune, String> {
    let log = log.ok_or("restoring is not available: no data directory")?;
    let restored = match batch {
        Some(batch) => item_ops::restore_prune(c, bin, log, batch),
        None => item_ops::restore_last_prune(c, bin, log),
    }
    .map_err(|e| e.to_string())?;
    Ok(RestoredPrune {
        restored: restored.items().iter().map(|p| p.to_string()).collect(),
        warnings: restored.warnings().to_vec(),
    })
}

const TRASH_UNAVAILABLE: &str =
    "deleting moves the item to the trash and none is available, so nothing was changed";

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn a_create_that_kept_conflicting_keeps_the_app_advice() {
        let path = PathBuf::from("/c/A.yml");
        let e = ItemError::NotSet {
            created: ItemPath::new("A.yml").expect("path"),
            message: "core's own text".to_owned(),
            conflict: Some(path.clone()),
        };
        assert_eq!(
            create_failure(&e),
            format!(
                "created A.yml but could not set its method or URL: {}",
                conflict_message(&path)
            )
        );
        assert!(create_failure(&e).contains(
            "/c/A.yml was changed by another program, likely the Apinata app with unsaved edits; \
             nothing was written. Ask the user to save or revert there, then retry."
        ));
        let other = ItemError::Target;
        assert_eq!(create_failure(&other), other.to_string());
    }
}
