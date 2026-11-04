//! The note-byte and entry limits shared by the Insomnia, Hoppscotch and HAR importers.

use super::{DocumentError, ImportNote, NoteKind};

/// Note text, and input text copied more than once, one import may produce. Note sources repeat
/// ancestor names, so a deep tree of long names could otherwise turn a small input into a huge
/// report.
pub(crate) const NOTE_BYTES: u64 = 256 * 1024 * 1024;

/// Entries (requests, folders, params, headers, variables, notes) one import may produce. Each
/// costs far more memory than its text, so text bytes alone do not bound the output; the limit
/// matches the OpenAPI importer's.
pub(crate) const ENTRIES: u64 = 1_000_000;

/// The note bytes and entries still allowed; past either the import is `TooLarge`.
pub(crate) struct Budget {
    left: u64,
    limit: u64,
    entries_left: u64,
    entries_limit: u64,
}

impl Budget {
    pub(crate) fn new(bytes: u64, entries: u64) -> Budget {
        Budget {
            left: bytes,
            limit: bytes,
            entries_left: entries,
            entries_limit: entries,
        }
    }

    /// Charge one emitted entry; `TooLarge { unit: "imported entries" }` past the limit.
    pub(crate) fn entry(&mut self) -> Result<(), DocumentError> {
        self.entries_left = self
            .entries_left
            .checked_sub(1)
            .ok_or(DocumentError::TooLarge {
                limit: self.entries_limit,
                unit: "imported entries",
            })?;
        Ok(())
    }

    /// Charge `bytes`, saturating at `u64::MAX`; `TooLarge { unit: "imported bytes" }` past the
    /// limit.
    pub(crate) fn charge(&mut self, bytes: usize) -> Result<(), DocumentError> {
        let bytes = u64::try_from(bytes).unwrap_or(u64::MAX);
        self.left = self
            .left
            .checked_sub(bytes)
            .ok_or(DocumentError::TooLarge {
                limit: self.limit,
                unit: "imported bytes",
            })?;
        Ok(())
    }

    /// A note sourced `pieces.concat()`: one entry plus the bytes of every piece and the
    /// detail, charged before the source is built.
    pub(crate) fn note_from(
        &mut self,
        pieces: &[&str],
        kind: NoteKind,
        detail: impl Into<String>,
    ) -> Result<ImportNote, DocumentError> {
        self.entry()?;
        let detail = detail.into();
        let source: usize = pieces.iter().map(|p| p.len()).sum();
        self.charge(source.saturating_add(detail.len()))?;
        Ok(ImportNote::new(pieces.concat(), kind, detail))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn too_large(limit: u64, unit: &'static str) -> DocumentError {
        DocumentError::TooLarge { limit, unit }
    }

    #[test]
    fn entries_past_the_limit_are_too_large() {
        let mut budget = Budget::new(NOTE_BYTES, 2);
        budget.entry().expect("first entry fits");
        budget.entry().expect("second entry fits");
        assert_eq!(
            budget.entry().unwrap_err(),
            too_large(2, "imported entries")
        );
    }

    #[test]
    fn bytes_past_the_limit_are_too_large() {
        let mut budget = Budget::new(10, ENTRIES);
        budget.charge(10).expect("ten bytes fit");
        assert_eq!(
            budget.charge(1).unwrap_err(),
            too_large(10, "imported bytes")
        );
    }

    #[test]
    fn a_huge_charge_saturates_instead_of_wrapping() {
        let mut budget = Budget::new(u64::MAX - 1, ENTRIES);
        assert_eq!(
            budget.charge(usize::MAX).unwrap_err(),
            too_large(u64::MAX - 1, "imported bytes")
        );
    }

    #[test]
    fn a_note_charges_one_entry_and_the_exact_bytes_of_its_pieces_and_detail() {
        // "a / " and "b" are 5 bytes and the detail 3, so 8 bytes fit and 7 do not.
        let mut budget = Budget::new(8, 1);
        let note = budget
            .note_from(&["a / ", "b"], NoteKind::Dropped, "xyz")
            .expect("one entry and eight bytes fit");
        assert_eq!(
            (note.source.as_str(), note.kind, note.detail.as_str()),
            ("a / b", NoteKind::Dropped, "xyz")
        );
        assert_eq!(
            budget.entry().unwrap_err(),
            too_large(1, "imported entries")
        );
        let mut tight = Budget::new(7, 1);
        assert_eq!(
            tight
                .note_from(&["a / ", "b"], NoteKind::Dropped, "xyz")
                .unwrap_err(),
            too_large(7, "imported bytes")
        );
    }
}
