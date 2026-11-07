//! Creating a new collection root, as an import does.

use super::{Collection, ROOT_FILE};
use crate::document::Document;
use crate::error::{CreateError, OpenError};
use crate::naming::{sanitize_name, validate_name};
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use yaml_serde::{Mapping, Value};

/// File systems cap a name at 255 bytes, a limit `validate_name` (UTF-16 units) does not see.
const MAX_NAME_BYTES: usize = 255;

impl Collection {
    /// Whether a sanitised `stem` can be a directory name: `validate_name` plus the byte cap.
    pub(crate) fn is_valid_dir_stem(stem: &str) -> bool {
        validate_name(stem) && stem.len() <= MAX_NAME_BYTES
    }

    /// Bruno's `findUniqueFolderName`: `name`, else `name - 1`, `name - 2`..., the first whose
    /// sanitised form does not exist in `parent_dir`. The result is the display name.
    // The candidate sequence never ends, so `find` returns only once a name is free.
    #[allow(clippy::expect_used)]
    pub fn unique_dir_name(parent_dir: &Path, name: &str) -> String {
        (0u64..)
            .map(|n| {
                if n == 0 {
                    name.to_owned()
                } else {
                    format!("{name} - {n}")
                }
            })
            .find(|candidate| !parent_dir.join(sanitize_name(candidate)).exists())
            .expect("an unbounded sequence has a free name")
    }

    /// Create `dir`, or fill an existing one that is empty or holds only `.git`, `.gitignore`,
    /// `.gitattributes` or `.DS_Store`, with the `opencollection.yml` an import writes for a
    /// collection called `name`, then open it.
    ///
    /// Fails with `InvalidName` when the sanitised `name` cannot be a directory name, `NotEmpty`
    /// when `dir` is a directory with entries, `Exists` when it is not a directory, and `Io`
    /// otherwise; a failure removes only what this call wrote.
    pub fn create(dir: &Path, name: &str) -> Result<Collection, CreateError> {
        let stem = sanitize_name(name);
        if !Self::is_valid_dir_stem(&stem) {
            return Err(CreateError::InvalidName {
                name: name.to_owned(),
            });
        }
        let io = |source| CreateError::Io {
            path: dir.to_path_buf(),
            source,
        };
        let made_dir = match fs::create_dir(dir) {
            Ok(()) => true,
            Err(source) if source.kind() == ErrorKind::AlreadyExists => {
                let path = dir.to_path_buf();
                if !dir.is_dir() {
                    return Err(CreateError::Exists { path });
                }
                let mut entries = fs::read_dir(dir).map_err(io)?;
                if entries.any(|e| e.is_ok_and(|e| !is_ignorable(&e.file_name()))) {
                    return Err(CreateError::NotEmpty { path });
                }
                false
            }
            Err(source) => return Err(io(source)),
        };
        let root_file = dir.join(ROOT_FILE);
        if let Err(source) = Document::create(&root_file, root_mapping(name)) {
            if made_dir {
                let _ = fs::remove_dir(dir);
            }
            return Err(CreateError::Io {
                path: root_file,
                source,
            });
        }
        // A failed open undoes this call's writes so a retry starts clean; a folder the user
        // supplied stays.
        Collection::open(dir).map_err(|e| {
            if made_dir {
                let _ = fs::remove_dir_all(dir);
            } else {
                let _ = fs::remove_file(&root_file);
            }
            match e {
                OpenError::Io { path, source } => CreateError::Io { path, source },
                other => CreateError::Io {
                    path: dir.to_path_buf(),
                    source: std::io::Error::other(other),
                },
            }
        })
    }
}

/// Entries a fresh repository or a file manager leaves behind; a directory holding only these
/// counts as empty.
fn is_ignorable(name: &std::ffi::OsStr) -> bool {
    [".git", ".gitignore", ".gitattributes", ".DS_Store"]
        .iter()
        .any(|n| name == *n)
}

/// The root file's keys in the order Bruno's import writes them.
fn root_mapping(name: &str) -> Mapping {
    let ignore = Value::Sequence(vec!["node_modules".into(), ".git".into()]);
    let bruno = Mapping::from_iter([(Value::from("ignore"), ignore)]);
    let extensions = Mapping::from_iter([(Value::from("bruno"), Value::Mapping(bruno))]);
    let info = Mapping::from_iter([(Value::from("name"), Value::from(name))]);
    Mapping::from_iter([
        (Value::from("opencollection"), Value::from("1.0.0")),
        (Value::from("info"), Value::Mapping(info)),
        (Value::from("bundled"), Value::Bool(false)),
        (Value::from("extensions"), Value::Mapping(extensions)),
    ])
}
