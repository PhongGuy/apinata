//! The budget that caps what one OpenAPI import may produce, in entries and in copied bytes.

use crate::import::{DocumentError, ImportNote};
use serde_json::Value;

/// Limits on one import; the defaults are measured.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Bounds {
    /// The most entries (requests, folders, params, headers, variables, notes) an import makes.
    pub(crate) entries: u64,
    /// The most bytes of text and structure an import copies out of the document.
    pub(crate) bytes: u64,
    /// The deepest property or array nesting a generated sample keeps.
    pub(crate) sample_depth: usize,
    /// The largest compact JSON length one sample may reach.
    pub(crate) sample_bytes: usize,
    /// The most schemas one sample may visit.
    pub(crate) sample_visits: u64,
    /// The most schemas all samples of one document may visit together.
    pub(crate) document_visits: u64,
}

impl Bounds {
    pub(crate) const DEFAULT: Bounds = Bounds {
        entries: 1_000_000,
        bytes: 256 << 20,
        sample_depth: 16,
        sample_bytes: 64 << 10,
        sample_visits: 100_000,
        document_visits: 2_000_000,
    };
}

/// Running counts of entries and copied bytes, each charged against its limit.
pub(crate) struct Budget {
    bounds: Bounds,
    entries: u64,
    bytes: u64,
}

impl Budget {
    pub(crate) fn new(bounds: Bounds) -> Budget {
        Budget {
            bounds,
            entries: 0,
            bytes: 0,
        }
    }

    /// Charge `n` entries. Fails with `TooLarge { limit, unit: "imported entries" }` once more
    /// than the entry limit has been charged.
    pub(crate) fn charge(&mut self, n: u64) -> Result<(), DocumentError> {
        self.entries = self.entries.saturating_add(n);
        if self.entries > self.bounds.entries {
            return Err(DocumentError::TooLarge {
                limit: self.bounds.entries,
                unit: "imported entries",
            });
        }
        Ok(())
    }

    /// Charge `n` copied bytes. Fails with `TooLarge { limit, unit: "imported bytes" }` once
    /// more than the byte limit has been charged.
    pub(crate) fn charge_bytes(&mut self, n: u64) -> Result<(), DocumentError> {
        self.bytes = self.bytes.saturating_add(n);
        if self.bytes > self.bounds.bytes {
            return Err(DocumentError::TooLarge {
                limit: self.bounds.bytes,
                unit: "imported bytes",
            });
        }
        Ok(())
    }

    /// The bytes charged so far, including a charge that went over the limit.
    #[cfg(test)]
    pub(crate) fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Charge the bytes of `text`, before it is copied.
    pub(crate) fn charge_text(&mut self, text: &str) -> Result<(), DocumentError> {
        self.charge_bytes(text.len() as u64)
    }

    /// Charge a note: one entry and the bytes of its source and detail.
    pub(crate) fn charge_note(&mut self, note: &ImportNote) -> Result<(), DocumentError> {
        self.charge(1)?;
        self.charge_bytes((note.source.len() + note.detail.len()) as u64)
    }
}

/// Roughly the bytes a deep copy of `value` holds: its text plus a fixed cost per value and key.
pub(crate) fn approx_size(value: &Value) -> u64 {
    // Per value, about one `Value` plus its share of the container holding it.
    const NODE: u64 = 64;
    let mut total = 0;
    let mut stack = vec![value];
    while let Some(v) = stack.pop() {
        total += NODE;
        match v {
            Value::String(s) => total += s.len() as u64,
            Value::Array(items) => stack.extend(items),
            Value::Object(map) => {
                for (key, item) in map {
                    total += key.len() as u64;
                    stack.push(item);
                }
            }
            _ => {}
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::NoteKind;

    #[test]
    fn bytes_are_charged_apart_from_entries() {
        let mut budget = Budget::new(Bounds {
            entries: 2,
            bytes: 10,
            ..Bounds::DEFAULT
        });
        budget.charge_text("12345").expect("five bytes fit");
        budget.charge_bytes(5).expect("ten bytes fit");
        budget.charge(2).expect("entries are counted apart");
        assert_eq!(
            budget.charge_text("x").unwrap_err(),
            DocumentError::TooLarge {
                limit: 10,
                unit: "imported bytes"
            }
        );
    }

    #[test]
    fn a_note_charges_one_entry_and_its_text() {
        let mut budget = Budget::new(Bounds {
            entries: 1,
            bytes: 8,
            ..Bounds::DEFAULT
        });
        let note = ImportNote::new("src", NoteKind::Dropped, "12345");
        budget
            .charge_note(&note)
            .expect("one entry and eight bytes fit");
        assert_eq!(
            budget.charge_bytes(1).unwrap_err(),
            DocumentError::TooLarge {
                limit: 8,
                unit: "imported bytes"
            }
        );
    }

    #[test]
    fn approx_size_counts_text_and_structure() {
        let small = approx_size(&serde_json::json!({"a": "x"}));
        let big = approx_size(&serde_json::json!({"a": "x".repeat(1000)}));
        assert!(small > 0);
        assert_eq!(big - small, 999);
    }
}
