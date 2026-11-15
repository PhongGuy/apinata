//! Opening a conflict copy, the local side a team sync kept aside, as a request's unsaved
//! edits, so the existing conflict banners settle it.

use super::{
    Collection, ENVIRONMENTS_DIR, Folder, WORKSPACE_FILE, dir_name_of, is_item_file, sort_lists,
};
use crate::collection::{Change, ChangeKind, ItemKind};
use crate::document::Document;
use crate::item_path::ItemPath;
use crate::request::RequestFile;
use std::fs;

/// Why a conflict copy could not open as a request's edits.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AdoptError {
    /// The path names no request: a folder file, an environment or another item type.
    #[error("only a request's copy opens as its edits")]
    NotARequest,
    /// The copy does not parse as a request.
    #[error("the copy does not load: {message}")]
    Unreadable {
        /// What the parser reported.
        message: String,
    },
}

impl Collection {
    /// Take `bytes`, a conflict copy of the request at `item`, as that request's unsaved edits:
    /// re-read the file first, keep it dirty even when equal to the disk, and bring the
    /// request back as removed-on-disk when the file is gone. Returns the `Conflict` change.
    pub fn adopt_conflict_copy(
        &mut self,
        item: &ItemPath,
        bytes: &[u8],
    ) -> Result<Change, AdoptError> {
        let segments: Vec<&str> = item.segments().collect();
        let Some(parent) = item.parent() else {
            return Err(AdoptError::NotARequest);
        };
        if !is_item_file(item.file_name())
            || matches!(
                segments.as_slice(),
                [WORKSPACE_FILE] | [ENVIRONMENTS_DIR, _]
            )
            || self.opaque_item(item).is_some()
        {
            return Err(AdoptError::NotARequest);
        }
        // Parse before touching the tree, so a copy that does not load changes nothing.
        let full = item.to_path(&self.root);
        let unreadable = |e: crate::error::FileError| AdoptError::Unreadable {
            message: e.message().to_owned(),
        };
        let doc = Document::parse(&full, bytes.to_vec()).map_err(unreadable)?;
        let typed = doc.get(&["info", "type"]).map(|t| t.as_str());
        if !matches!(typed, None | Some(Some("http" | "graphql"))) {
            return Err(AdoptError::NotARequest);
        }
        let mut copy = RequestFile::checked(doc)
            .map_err(unreadable)?
            .into_document();

        // Read the disk first, so its bytes are the snapshot and a later watcher event is quiet.
        self.reload_path(&full);
        if self.opaque_item(item).is_some() {
            return Err(AdoptError::NotARequest);
        }
        match self.request_mut(item) {
            Some(request) => {
                let doc = request.document_mut();
                doc.set_mapping(copy.mapping().clone());
                doc.mark_unsaved();
            }
            // The reload dropped a clean request whose file is gone or broken: bring it back
            // with no disk hash, so a plain save recreates a gone file but never replaces one.
            None => {
                let present = fs::symlink_metadata(&full).is_ok();
                copy.mark_removed();
                if present {
                    copy.mark_broken();
                }
                copy.mark_unsaved();
                let folder = self.restore_folders(&parent);
                folder.requests.push(RequestFile::from_document(copy));
                sort_lists(folder);
            }
        }
        Ok(Change {
            path: full,
            kind: ChangeKind::Conflict,
            item: ItemKind::Request,
            items: Vec::new(),
        })
    }

    /// The folder at `dir`, putting back each missing folder on the way as one kept for
    /// drafts, so a teammate's deleted folder returns with the copy inside it.
    fn restore_folders(&mut self, dir: &ItemPath) -> &mut Folder {
        let mut folder = &mut self.tree;
        for name in dir.segments() {
            let i = match folder.folders.iter().position(|f| f.dir_name == name) {
                Some(i) => i,
                None => {
                    let path = folder.path.join(name);
                    folder.folders.push(Folder {
                        dir_name: dir_name_of(&path),
                        removed: !path.is_dir(),
                        path,
                        is_root: false,
                        file: None,
                        file_broken: false,
                        folders: Vec::new(),
                        requests: Vec::new(),
                        opaque: Vec::new(),
                    });
                    // Just pushed, so the lookup after sorting always finds it.
                    sort_lists(folder);
                    folder
                        .folders
                        .iter()
                        .position(|f| f.dir_name == name)
                        .unwrap_or_default()
                }
            };
            folder = &mut folder.folders[i];
        }
        folder
    }
}
