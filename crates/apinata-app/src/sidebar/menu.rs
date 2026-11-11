//! The sidebar's context menu entries per row kind, and the names new items start with.

use std::path::{Path, PathBuf};

use apinata_core::{ItemPath, RequestKind};

use super::model::NodeKind;
use super::multi::Tally;
use crate::commands::label;

/// One context menu entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuEntry {
    /// Create a request inside the row.
    NewRequest,
    /// Create a GraphQL request inside the row.
    NewGraphqlRequest,
    /// Create a folder inside the row.
    NewFolder,
    /// Copy the item, or the folder with everything in it, next to itself.
    CloneItem,
    /// Copy the row onto the clipboard.
    CopyItem,
    /// Cut the row; it moves when pasted.
    CutItem,
    /// Paste the clipboard into the row's folder.
    PasteItem,
    /// Renumber the folder's items by name.
    SortByName,
    /// Copy the request as a curl command.
    CopyAsCurl,
    /// Show the request as code in a chosen language.
    GenerateCode,
    /// Rewrite an HTTP request as a GraphQL request; a GraphQL row offers `ConvertToHttp` here.
    ConvertToGraphql,
    /// Rewrite a GraphQL request as an HTTP request.
    ConvertToHttp,
    /// Open the collection's runner tab.
    RunCollection,
    /// Open the folder's runner tab.
    RunFolder,
    /// Open the collection's settings tab.
    CollectionSettings,
    /// Open the folder's settings tab.
    Settings,
    /// Rename the row inline.
    Rename,
    /// Delete the row after asking.
    Delete,
    /// Export the collection as a Postman collection file.
    ExportPostman,
    /// Export the collection as a Postman collection file with its scripts kept as written.
    ExportPostmanKeepScripts,
    /// Export the collection as an OpenAPI 3.1 document.
    ExportOpenApi,
    /// Export the collection as an HTML documentation page.
    ExportHtml,
    /// Open the row's directory in the system file manager.
    ShowInFileManager,
    /// Open a terminal in the row's directory.
    OpenInTerminal,
    /// Close every folder of the collection.
    CollapseAll,
    /// Open every folder of the collection.
    ExpandAll,
    /// Close the collection.
    Close,
    /// Open the Changes dialog on the row's file; offered only while it has a git status.
    ShowChanges,
    /// Save the open draft through the save dialog.
    SaveDraft,
    /// Close the open draft's tab.
    CloseDraft,
}

impl MenuEntry {
    /// The entry's menu label; entries backed by a command take its registry name.
    pub fn label(self) -> &'static str {
        match self {
            MenuEntry::NewRequest => label("collection.new_request"),
            MenuEntry::NewGraphqlRequest => label("collection.new_graphql_request"),
            MenuEntry::NewFolder => label("collection.new_folder"),
            MenuEntry::CloneItem => "Duplicate",
            MenuEntry::CopyItem => "Copy",
            MenuEntry::CutItem => "Cut",
            MenuEntry::PasteItem => "Paste",
            MenuEntry::SortByName => label("collection.sort_by_name"),
            MenuEntry::CopyAsCurl => label("request.copy_as_curl"),
            MenuEntry::GenerateCode => label("request.generate_code"),
            MenuEntry::ConvertToGraphql => label("request.convert_to_graphql"),
            MenuEntry::ConvertToHttp => label("request.convert_to_http"),
            MenuEntry::RunCollection => label("collection.run"),
            MenuEntry::RunFolder => label("collection.run_folder"),
            MenuEntry::CollectionSettings => label("collection.settings"),
            MenuEntry::Settings => "Settings",
            MenuEntry::Rename => "Rename",
            MenuEntry::Delete => "Delete",
            MenuEntry::ExportPostman => label("collection.export"),
            MenuEntry::ExportPostmanKeepScripts => label("collection.export_keep_scripts"),
            MenuEntry::ExportOpenApi => label("collection.export_openapi"),
            MenuEntry::ExportHtml => label("collection.export_html"),
            MenuEntry::ShowInFileManager => label("request.show_in_file_manager"),
            MenuEntry::OpenInTerminal => label("request.open_in_terminal"),
            MenuEntry::CollapseAll => label("collection.collapse_all"),
            MenuEntry::ExpandAll => label("collection.expand_all"),
            MenuEntry::Close => label("collection.close"),
            MenuEntry::ShowChanges => "Show changes",
            MenuEntry::SaveDraft => "Save",
            MenuEntry::CloseDraft => "Close",
        }
    }
}

/// Whether the scratch pad's row greys out `entry`: the scratch has no collection files to run,
/// configure, export or close.
pub fn scratch_blocked(entry: MenuEntry) -> bool {
    use MenuEntry as E;
    matches!(
        entry,
        E::RunCollection
            | E::CollectionSettings
            | E::ExportPostman
            | E::ExportPostmanKeepScripts
            | E::ExportOpenApi
            | E::ExportHtml
            | E::Close
    )
}

/// The entries a row of this kind offers, in menu order; invalid rows and the Unsaved group offer none.
pub fn menu_entries(kind: NodeKind) -> &'static [MenuEntry] {
    use MenuEntry as E;
    match kind {
        NodeKind::Collection => &[
            E::NewRequest,
            E::NewGraphqlRequest,
            E::NewFolder,
            E::PasteItem,
            E::SortByName,
            E::RunCollection,
            E::CollectionSettings,
            E::ExportPostman,
            E::ExportPostmanKeepScripts,
            E::ExportOpenApi,
            E::ExportHtml,
            E::ShowInFileManager,
            E::OpenInTerminal,
            E::CollapseAll,
            E::ExpandAll,
            E::Close,
        ],
        NodeKind::Folder => &[
            E::NewRequest,
            E::NewGraphqlRequest,
            E::NewFolder,
            E::CloneItem,
            E::CopyItem,
            E::CutItem,
            E::PasteItem,
            E::SortByName,
            E::RunFolder,
            E::Settings,
            E::ShowInFileManager,
            E::OpenInTerminal,
            E::Rename,
            E::Delete,
        ],
        NodeKind::Request => &[
            E::CloneItem,
            E::CopyItem,
            E::CutItem,
            E::PasteItem,
            E::CopyAsCurl,
            E::GenerateCode,
            E::ConvertToGraphql,
            E::ShowInFileManager,
            E::OpenInTerminal,
            E::Rename,
            E::Delete,
        ],
        NodeKind::Opaque => &[
            E::CloneItem,
            E::CopyItem,
            E::CutItem,
            E::ShowInFileManager,
            E::OpenInTerminal,
            E::Delete,
        ],
        NodeKind::Example => &[E::Rename, E::Delete],
        NodeKind::Draft => &[E::SaveDraft, E::CloseDraft],
        NodeKind::Invalid | NodeKind::UnsavedGroup => &[],
    }
}

/// The entry a request row of `kind` shows in `entry`'s place: the conversion slot names the
/// other kind, so a GraphQL row offers Convert to HTTP where an HTTP row offers Convert to GraphQL.
pub fn for_request_kind(entry: MenuEntry, kind: RequestKind) -> MenuEntry {
    match (entry, kind) {
        (MenuEntry::ConvertToGraphql, RequestKind::Graphql) => MenuEntry::ConvertToHttp,
        _ => entry,
    }
}

/// The directory the file manager and terminal entries act on: a collection or folder row's
/// own directory, a request or opaque file's containing directory. `None` for other kinds.
pub fn entry_dir(kind: NodeKind, root: &Path, path: &ItemPath) -> Option<PathBuf> {
    let dir = match kind {
        NodeKind::Collection | NodeKind::Folder => path.clone(),
        NodeKind::Request | NodeKind::Opaque => path.parent()?,
        _ => return None,
    };
    Some(if dir.is_root() {
        root.to_path_buf()
    } else {
        root.join(dir.as_str())
    })
}

/// One entry of the menu a multi-selection opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BulkEntry {
    /// Run the n selected requests.
    Run(usize),
    /// Copy the n selected items.
    Copy(usize),
    /// Cut the n selected items.
    Cut(usize),
    /// Delete the selected items, told apart as folders and the rest.
    Delete(Tally),
}

impl BulkEntry {
    /// The entry's menu label, counting what it acts on.
    pub fn label(self) -> String {
        match self {
            BulkEntry::Run(n) => format!("Run {n} request{}", if n == 1 { "" } else { "s" }),
            BulkEntry::Copy(n) => format!("Copy {n} item{}", if n == 1 { "" } else { "s" }),
            BulkEntry::Cut(n) => format!("Cut {n} item{}", if n == 1 { "" } else { "s" }),
            BulkEntry::Delete(t) => format!("Delete {}", t.describe()),
        }
    }
}

/// The bulk menu in order: Run first when `run` is `Some`, then Copy, Cut and Delete.
pub fn bulk_entries(items: Tally, run: Option<usize>) -> Vec<BulkEntry> {
    run.map(BulkEntry::Run)
        .into_iter()
        .chain([
            BulkEntry::Copy(items.total()),
            BulkEntry::Cut(items.total()),
            BulkEntry::Delete(items),
        ])
        .collect()
}

/// The name a new folder starts with, before the inline rename.
pub const NEW_FOLDER_NAME: &str = "New folder";

/// The name of a request's clone, as Bruno names it.
pub fn clone_name(name: &str) -> String {
    format!("{name} copy")
}
