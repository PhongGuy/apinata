//! Why a legacy collection cannot be planned or converted.

use std::fmt;
use std::path::PathBuf;

/// Why [`plan_conversion`](super::plan_conversion) or a conversion refused.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConvertError {
    /// The folder has no `bruno.json`, so it is not a legacy collection.
    #[error("{} is not a collection in the legacy .bru format", root.display())]
    NotLegacy {
        /// The folder that was tried.
        root: PathBuf,
    },
    /// Legacy files that could not be read or parsed, sorted by path.
    #[error("{} of the legacy files could not be read", errors.len())]
    Parse {
        /// One entry per failing file.
        errors: Vec<BruError>,
    },
    /// Paths the conversion would write already exist.
    #[error("the conversion would overwrite existing paths ({})", paths.len())]
    TargetExists {
        /// The colliding paths, relative to the source root when inside it.
        paths: Vec<PathBuf>,
    },
    /// A source file changed or vanished after the plan was made.
    #[error("{} changed since the conversion was planned", path.display())]
    SourceChanged {
        /// The changed file, relative to the source root.
        path: PathBuf,
    },
    /// The collection name gives no usable folder name.
    #[error("the collection name {name:?} cannot be a folder name")]
    InvalidName {
        /// The collection name.
        name: String,
    },
    /// A filesystem operation failed.
    #[error("{}: {message}", path.display())]
    Io {
        /// The path that failed.
        path: PathBuf,
        /// The underlying error, as text.
        message: String,
    },
}

/// A legacy file that could not be read or parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BruError {
    /// The file, relative to the collection root.
    pub path: PathBuf,
    /// The 1-based line of the problem, or 0 when no line applies.
    pub line: usize,
    /// The parser's message, which may span several lines.
    pub message: String,
}

impl fmt::Display for BruError {
    /// `path:line: message` on one line: a multi-line parser message shows its last
    /// non-empty line, which is where the parser states what it expected.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = self
            .message
            .lines()
            .rev()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("");
        match self.line {
            0 => write!(f, "{}: {message}", self.path.display()),
            line => write!(f, "{}:{line}: {message}", self.path.display()),
        }
    }
}

impl std::error::Error for BruError {}
