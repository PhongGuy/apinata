//! Why a file failed the acceptance gate, as a kind to branch on and the text a person reads.

use std::fmt;

/// Why [`read_mapping`](crate::read_mapping) refused a file.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadError {
    kind: ReadErrorKind,
    position: Option<(usize, usize)>,
    message: String,
}

impl ReadError {
    pub(crate) fn new(
        kind: ReadErrorKind,
        position: Option<(usize, usize)>,
        message: impl Into<String>,
    ) -> ReadError {
        ReadError {
            kind,
            position,
            message: message.into(),
        }
    }

    /// Which rule the file broke.
    pub fn kind(&self) -> ReadErrorKind {
        self.kind
    }

    /// The 1-based (line, column) the parser stopped at; set for [`ReadErrorKind::Syntax`] only.
    pub fn position(&self) -> Option<(usize, usize)> {
        self.position
    }
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// The rule a refused file broke.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadErrorKind {
    /// The bytes are not valid UTF-8.
    NotUtf8,
    /// Alias expansion would pass [`MAX_EXPANDED_EVENTS`](crate::MAX_EXPANDED_EVENTS).
    TooManyValues,
    /// The text has aliases past a syntax the expansion count cannot follow.
    UnfollowableAliases,
    /// The YAML does not parse, git conflict markers included.
    Syntax,
    /// A YAML tag appears somewhere in the file.
    Tag,
    /// The root is not a mapping.
    NotMapping,
}
