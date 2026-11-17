//! Apinata's YAML reader kernel: the acceptance gate every collection file passes before it is
//! trusted, shared by the app and the server rules. It links no async, network or UI crate.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod error;
mod prepass;

pub use error::{ReadError, ReadErrorKind};
pub use prepass::{MAX_NESTING, Refusal, prepass};

use yaml_serde::{Mapping, Value};

/// The most YAML events a collection file may expand to, counting each alias as the node it names.
pub const MAX_EXPANDED_EVENTS: u64 = 2_000_000;

/// Refuse a YAML text whose alias expansion would exhaust memory or time, before `yaml_serde`
/// builds anything from it. The pre-pass can reject text `yaml_serde` accepts, so past such an
/// error the aliases are uncounted; text with both an anchor and an alias marker is then refused
/// rather than trusted. Any other syntax or nesting error passes through to `yaml_serde`.
pub fn bound_expansion(text: &str) -> Result<(), String> {
    expansion(text).map_err(expansion_message)
}

/// The expansion bound as a kind, so callers that type their errors need not reparse the text.
fn expansion(text: &str) -> Result<(), ReadErrorKind> {
    match prepass(text, MAX_EXPANDED_EVENTS) {
        Err(Refusal::TooManyEvents) => Err(ReadErrorKind::TooManyValues),
        Err(Refusal::At { .. }) if text.contains('&') && text.contains('*') => {
            Err(ReadErrorKind::UnfollowableAliases)
        }
        _ => Ok(()),
    }
}

fn expansion_message(kind: ReadErrorKind) -> String {
    match kind {
        ReadErrorKind::TooManyValues => {
            format!("file is too large: more than {MAX_EXPANDED_EVENTS} values once aliases expand")
        }
        _ => "file has aliases and a syntax the safety check cannot follow".to_owned(),
    }
}

/// Read `bytes` as a collection file: UTF-8 with an optional BOM, within the expansion bound,
/// valid YAML, free of tags, with a mapping at the root. Returns the root mapping in document
/// order, or a [`ReadError`] naming the first rule the file broke.
pub fn read_mapping(bytes: &[u8]) -> Result<Mapping, ReadError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| ReadError::new(ReadErrorKind::NotUtf8, None, "file is not valid UTF-8"))?;
    // yaml_serde reports a BOM as a second document, so drop it first.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    expansion(text).map_err(|kind| ReadError::new(kind, None, expansion_message(kind)))?;
    let value: Value = yaml_serde::from_str(text).map_err(|e| {
        let at = e.location().map(|l| (l.line(), l.column()));
        // A file git left conflicted is better named than described by the parser.
        let message = match conflict_marker_line(text) {
            Some(n) => format!("merge conflict markers at line {n}; resolve them in git"),
            None => e.to_string(),
        };
        ReadError::new(ReadErrorKind::Syntax, at, message)
    })?;
    // Tags first, so a tagged root is reported as a tag rather than a non-mapping.
    if has_tag(&value) {
        return Err(ReadError::new(
            ReadErrorKind::Tag,
            None,
            "YAML tags are not supported",
        ));
    }
    let Value::Mapping(root) = value else {
        return Err(ReadError::new(
            ReadErrorKind::NotMapping,
            None,
            "the file root must be a mapping",
        ));
    };
    Ok(root)
}

/// The 1-based line of the first git conflict marker in `text`, if any. Only consulted
/// after a parse failure, so marker-like lines inside a block scalar never matter.
fn conflict_marker_line(text: &str) -> Option<usize> {
    text.lines()
        .position(|l| {
            let l = l.trim_end();
            l == "<<<<<<<"
                || l == ">>>>>>>"
                || l == "======="
                || l.starts_with("<<<<<<< ")
                || l.starts_with(">>>>>>> ")
        })
        .map(|i| i + 1)
}

/// Whether a tagged value appears anywhere under `value`.
fn has_tag(value: &Value) -> bool {
    match value {
        Value::Tagged(_) => true,
        Value::Sequence(items) => items.iter().any(has_tag),
        Value::Mapping(map) => map.iter().any(|(k, v)| has_tag(k) || has_tag(v)),
        _ => false,
    }
}
