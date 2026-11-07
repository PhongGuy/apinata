//! Drafts: new requests held in memory until saved under a chosen name and folder.

use super::Collection;
use super::snapshot::DraftSnapshot;
use super::tree::{INFO_ORDER, new_seq};
use crate::document::{Document, set_value};
use crate::error::TreeError;
use crate::item_path::ItemPath;
use crate::request::{RequestFile, RequestKind};
use yaml_serde::{Mapping, Value};

impl Collection {
    /// Start an unsaved request in the folder `parent`, from the same template as
    /// [`Collection::create_request`] and clean until edited. It is named "Untitled request",
    /// or `"Untitled request <n>"` with the lowest `n` from 2 that no other draft holds. Nothing
    /// is written. Returns its path, `<parent>/untitled-<n>.draft`; `NotFound` when there is
    /// no such folder.
    pub fn create_draft(&mut self, parent: &ItemPath) -> Result<ItemPath, TreeError> {
        self.create_draft_of_kind(parent, RequestKind::Http)
    }

    /// Start an unsaved request like [`Collection::create_draft_of_kind`], with the kind the
    /// collection's presets name (GraphQL, else HTTP) in place of an explicit one.
    pub fn create_draft_from_presets(&mut self, parent: &ItemPath) -> Result<ItemPath, TreeError> {
        let kind = self.file.presets().kind();
        self.create_draft_of_kind(parent, kind)
    }

    /// Start an unsaved request of `kind` like [`Collection::create_draft`], from the same
    /// template as [`Collection::create_request_of_kind`], except that its URL starts as the
    /// collection's preset URL. Fails like `create_draft`.
    pub fn create_draft_of_kind(
        &mut self,
        parent: &ItemPath,
        kind: RequestKind,
    ) -> Result<ItemPath, TreeError> {
        let dir = self.folder(parent).ok_or_else(|| TreeError::NotFound {
            item: parent.clone(),
        })?;
        let seq = new_seq(parent, dir);
        let root = self.preset_request_root(&self.free_draft_name(), seq, kind);
        self.push_draft(parent, root)
    }

    /// Start an unsaved request in the folder `parent` holding `snap`'s mapping, clean until
    /// edited. It keeps the snapshot's name unless another draft holds it, then takes a free
    /// name as [`Collection::create_draft`] does. Nothing is written; `NotFound` for an
    /// unknown folder.
    pub fn create_draft_from(
        &mut self,
        parent: &ItemPath,
        snap: &DraftSnapshot,
    ) -> Result<ItemPath, TreeError> {
        let mut root = snap.mapping().clone();
        let held = match root.get("info").and_then(|i| i.get("name")) {
            Some(Value::String(name)) => self.drafts.iter().any(|d| d.name() == name),
            _ => true,
        };
        if held {
            let name = self.free_draft_name();
            set_value(&mut root, &["info", "name"], Value::from(name), INFO_ORDER);
        }
        self.push_draft(parent, root)
    }

    // Place `root` as a new draft in the next `untitled-<n>.draft` slot of `parent`.
    fn push_draft(&mut self, parent: &ItemPath, root: Mapping) -> Result<ItemPath, TreeError> {
        let not_found = || TreeError::NotFound {
            item: parent.clone(),
        };
        self.folder(parent).ok_or_else(not_found)?;
        self.draft_count += 1;
        let item = parent
            .join(&format!("untitled-{}.draft", self.draft_count))
            .ok_or_else(not_found)?;
        let doc = Document::draft(item.to_path(&self.root), root);
        self.drafts.push(RequestFile::from_document(doc));
        Ok(item)
    }

    // Open drafts have no file name to tell them apart, so each gets a name of its own.
    fn free_draft_name(&self) -> String {
        let taken = |name: &str| self.drafts.iter().any(|d| d.name() == name);
        let mut name = "Untitled request".to_owned();
        let mut n = 1;
        while taken(&name) {
            n += 1;
            name = format!("Untitled request {n}");
        }
        name
    }

    /// Whether `item` is a draft of this collection.
    pub fn is_draft(&self, item: &ItemPath) -> bool {
        item.is_draft() && self.request(item).is_some()
    }

    /// Keep the draft at `draft` dirty until it is saved, through a revert too, as a draft
    /// restored from an earlier session is. False, changing nothing, when it is not a draft here.
    pub fn mark_draft_unsaved(&mut self, draft: &ItemPath) -> bool {
        if !draft.is_draft() {
            return false;
        }
        match self.request_mut(draft) {
            Some(request) => {
                request.document_mut().mark_unsaved();
                true
            }
            None => false,
        }
    }

    /// The drafts, oldest first.
    pub fn drafts(&self) -> impl Iterator<Item = &RequestFile> {
        self.drafts.iter()
    }

    /// Write the draft at `draft`, edits included, as a request named `name` in the folder
    /// `parent`, numbered and named exactly as [`Collection::create_request`] would, then drop
    /// the draft. Returns the saved request's path. Fails like `create_request`, or with
    /// `NotFound` for an unknown draft; on any error the draft stays.
    pub fn save_draft(
        &mut self,
        draft: &ItemPath,
        parent: &ItemPath,
        name: &str,
    ) -> Result<ItemPath, TreeError> {
        let edited = match self.request(draft).filter(|_| draft.is_draft()) {
            Some(source) => source.document().mapping().clone(),
            None => {
                return Err(TreeError::NotFound {
                    item: draft.clone(),
                });
            }
        };
        let (stem, seq) = self.new_request_slot(parent, name)?;
        let mut root = edited;
        set_value(&mut root, &["info", "name"], Value::from(name), INFO_ORDER);
        set_value(&mut root, &["info", "seq"], Value::from(seq), INFO_ORDER);
        let saved = self.write_new_request(parent, &stem, name, root)?;
        self.remove_draft(draft);
        Ok(saved)
    }

    /// Write `snap` as a request named `name` in the folder `parent`, through a temporary
    /// draft that is gone afterwards. Returns the saved request's path; fails like
    /// [`Collection::save_draft`], leaving no draft behind on any error.
    pub fn save_snapshot(
        &mut self,
        parent: &ItemPath,
        name: &str,
        snap: &DraftSnapshot,
    ) -> Result<ItemPath, TreeError> {
        let draft = self.create_draft_from(parent, snap)?;
        let saved = self.save_draft(&draft, parent, name);
        if saved.is_err() {
            self.remove_draft(&draft);
        }
        saved
    }

    /// Discard the draft at `draft`; false when there is none.
    pub fn remove_draft(&mut self, draft: &ItemPath) -> bool {
        if !draft.is_draft() {
            return false;
        }
        let full = draft.to_path(&self.root);
        let before = self.drafts.len();
        self.drafts.retain(|d| d.path() != full);
        self.drafts.len() != before
    }
}
