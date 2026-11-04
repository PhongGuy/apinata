//! Settling a draft against its file once they disagree: drop the draft and take the disk, or
//! write the draft over whatever the disk holds.

use super::{Collection, ENVIRONMENTS_DIR, FOLDER_FILE, ROOT_FILE};
use crate::document::{Document, read_regular};
use crate::error::{FileError, SaveError};
use crate::item_path::ItemPath;
use crate::yaml::emit::to_bruno_yaml;
use std::io::ErrorKind;
use std::path::Path;

/// The text a document's file holds and the text its draft would write.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct FileTexts {
    /// The file's current content; `None` when it cannot be read (gone).
    pub disk: Option<String>,
    /// The draft, serialized by the writer.
    pub draft: String,
}

impl Collection {
    /// Drop the draft at `path` and re-read its file: take the disk's content, leave the
    /// collection when the file is gone or broken, or (opencollection.yml) stay marked broken.
    /// A folder kept only for drafts goes with its last one. A new-request draft is reset to
    /// its template in memory.
    pub fn revert(&mut self, path: &Path) -> Result<(), FileError> {
        if let Some(draft) = self.drafts.iter_mut().find(|d| d.path() == path) {
            draft.document_mut().discard_draft();
            return Ok(());
        }
        let Some(doc) = self.document_at_mut(path) else {
            return Err(FileError::new(path, "no open document at this path"));
        };
        // A clean document is exactly what the reload handlers already settle against the disk.
        doc.discard_draft();
        self.reload_path(path);
        if let Some(dir) = self.item_path(path).and_then(|rel| rel.parent()) {
            self.prune_removed(dir);
        }
        Ok(())
    }

    /// Write the draft at `path` whatever the disk holds, recreating missing directories.
    /// A new-request draft has no file, so it fails with `Io` (`NotFound`) and writes nothing.
    pub fn save_overwrite(&mut self, path: &Path) -> Result<(), SaveError> {
        let Some(doc) = self.document_at_mut(path) else {
            return Err(SaveError::Io {
                path: path.to_path_buf(),
                source: ErrorKind::NotFound.into(),
            });
        };
        doc.save_overwrite()?;
        // The file loads again, so an earlier parse failure no longer applies.
        self.replace_errors(path, Vec::new());
        Ok(())
    }

    /// What the disk and the draft hold for the document at `path`, both as the writer
    /// would produce them, for showing a comparison. `None` for a path with no open document.
    /// Changes nothing.
    pub fn file_texts(&self, path: &Path) -> Option<FileTexts> {
        let doc = self.document_at(path)?;
        Some(FileTexts {
            disk: read_regular(doc.path())
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok()),
            draft: to_bruno_yaml(doc.mapping()),
        })
    }

    // Read-only twin of `document_at_mut`, resolving the same paths.
    fn document_at(&self, path: &Path) -> Option<&Document> {
        let rel = self.item_path(path)?;
        let segments: Vec<&str> = rel.segments().collect();
        match segments.as_slice() {
            [ROOT_FILE] => return Some(self.file.document()),
            [".env"] => return None,
            [ENVIRONMENTS_DIR, _] => {
                let full = rel.to_path(&self.root);
                return self
                    .environments
                    .iter()
                    .find(|e| e.doc.path() == full)
                    .map(|e| &e.doc);
            }
            _ => {}
        }
        if rel.file_name() == FOLDER_FILE {
            let folder = self.folder(&rel.parent()?)?;
            if folder.is_root {
                return None;
            }
            return folder.file.as_ref().map(|f| f.document());
        }
        self.request(&rel).map(|r| r.document())
    }

    /// The open document whose file is `path`: a request, a non-root `folder.yml`,
    /// `opencollection.yml` or an environment. `None` for anything else, `.env` included.
    fn document_at_mut(&mut self, path: &Path) -> Option<&mut Document> {
        let rel = self.item_path(path)?;
        let segments: Vec<&str> = rel.segments().collect();
        match segments.as_slice() {
            [ROOT_FILE] => return Some(self.file.document_mut()),
            [".env"] => return None,
            [ENVIRONMENTS_DIR, _] => {
                let full = rel.to_path(&self.root);
                return self
                    .environments
                    .iter_mut()
                    .find(|e| e.doc.path() == full)
                    .map(|e| &mut e.doc);
            }
            _ => {}
        }
        if rel.file_name() == FOLDER_FILE {
            let folder = self.folder_mut(&rel.parent()?)?;
            if folder.is_root {
                return None;
            }
            return folder.file.as_mut().map(|f| f.document_mut());
        }
        self.request_mut(&rel).map(|r| r.document_mut())
    }

    /// Walk from `dir` toward the root, dropping each folder kept for drafts that has none
    /// left; stop at the first folder that is present or still holds something.
    fn prune_removed(&mut self, mut dir: ItemPath) {
        while let Some(parent) = dir.parent() {
            let empty = self.folder(&dir).is_some_and(|f| {
                f.removed && f.requests.is_empty() && f.file.is_none() && f.folders.is_empty()
            });
            if !empty {
                return;
            }
            let name = dir.file_name().to_owned();
            if let Some(parent) = self.folder_mut(&parent) {
                parent.folders.retain(|f| f.dir_name != name);
            }
            dir = parent;
        }
    }
}
