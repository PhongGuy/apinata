//! Plain-words reasons for an import input that cannot be read.

use std::io::{Error, ErrorKind};

/// Why an import input could not be read, in plain words from the failure's kind and never
/// with an OS error code. The caller names the file.
pub fn read_reason(error: &Error) -> String {
    match error.kind() {
        ErrorKind::NotFound => "there is no file at that path".to_owned(),
        ErrorKind::IsADirectory => "it is a folder, not a file".to_owned(),
        ErrorKind::NotADirectory => "part of the path is a file, not a folder".to_owned(),
        ErrorKind::PermissionDenied => "you do not have permission to read it".to_owned(),
        ErrorKind::InvalidData => "it is not UTF-8 text".to_owned(),
        other => other.to_string(),
    }
}
