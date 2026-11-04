//! The HAR importer's view of the shared import budget.

use crate::import::{DocumentError, ImportNote, NoteKind};

pub(super) use crate::import::budget::Budget;

/// A note whose source is a prebuilt prefix plus a path.
pub(super) trait Prefixed {
    /// A note sourced `{at}{path}`, charged before the source is built.
    fn note(
        &mut self,
        at: &str,
        path: &str,
        kind: NoteKind,
        detail: impl Into<String>,
    ) -> Result<ImportNote, DocumentError>;
}

impl Prefixed for Budget {
    fn note(
        &mut self,
        at: &str,
        path: &str,
        kind: NoteKind,
        detail: impl Into<String>,
    ) -> Result<ImportNote, DocumentError> {
        self.note_from(&[at, path], kind, detail)
    }
}
