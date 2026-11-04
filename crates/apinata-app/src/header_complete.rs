//! Completing header names and values in the Headers table: which texts a cell offers for
//! what is typed, and the edit that accepts one.

use std::ops::Range;

use apinata_core::{header_names, header_values};

/// What a header cell completes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hint {
    /// A name cell.
    Name {
        /// The names the other rows already hold.
        present: Vec<String>,
    },
    /// A value cell.
    Value {
        /// The header the row names.
        name: String,
    },
}

/// The texts offered for `typed`, ignoring case: prefix matches before substring matches,
/// table order within each; names the table already holds last. Nothing is offered for an
/// empty cell or for a text that is already exactly what is typed.
pub fn offers(hint: &Hint, typed: &str) -> Vec<String> {
    if typed.is_empty() {
        return Vec::new();
    }
    let (all, present): (&[&str], &[String]) = match hint {
        Hint::Name { present } => (header_names(), present),
        Hint::Value { name } => (header_values(name), &[]),
    };
    let needle = typed.to_lowercase();
    let mut hits: Vec<(bool, bool, usize, &str)> = all
        .iter()
        .enumerate()
        .filter(|(_, c)| **c != typed)
        .filter_map(|(i, c)| {
            let lower = c.to_lowercase();
            lower.contains(&needle).then(|| {
                let used = present.iter().any(|p| p.eq_ignore_ascii_case(c));
                (used, !lower.starts_with(&needle), i, *c)
            })
        })
        .collect();
    hits.sort_by_key(|&(used, substring, i, _)| (used, substring, i));
    hits.into_iter().map(|(.., c)| c.to_owned()).collect()
}

/// The edit accepting `choice` in a cell holding `text`: the whole text is replaced, and the
/// cursor lands after it.
pub fn accept(text: &str, choice: &str) -> (Range<usize>, String, usize) {
    (0..text.len(), choice.to_owned(), choice.len())
}
