//! A Postman data export read in one step: the zip Postman mails out, the folder it unzips to,
//! or that folder's `archive.json`. Every entry is read in memory and recognized by its content,
//! so entry names are only labels and nothing in the archive decides where a file is written.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::load::load;
use super::{
    CollectionFormat, CollectionOptions, EnvironmentFormat, ImportBundle, ImportNote,
    ImportedCollection, ImportedEnvironment, NoteKind, parse_collections, parse_environments,
};

/// The file that marks a folder or zip as a Postman data export.
const MARKER: &str = "archive.json";
/// How deep below the export root a folder export is searched; Postman nests one level.
const MAX_DEPTH: usize = 3;
const ZIP_MAGIC: &[u8] = b"PK";

const MIB: u64 = 1024 * 1024;

/// The caps that keep a hostile archive from exhausting memory.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    /// The zip file's own size.
    pub(crate) archive: u64,
    /// Files in the archive or folder.
    pub(crate) entries: usize,
    /// One entry's content, counted while reading rather than trusting a declared size.
    pub(crate) entry: u64,
    /// Every entry's content together.
    pub(crate) total: u64,
}

impl Limits {
    pub(crate) const DEFAULT: Limits = Limits {
        archive: 256 * MIB,
        entries: 10_000,
        entry: 64 * MIB,
        total: 512 * MIB,
    };
}

/// What a Postman data export holds, ready to write.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct PostmanExport {
    /// One bundle per collection, sorted by collection name, each carrying every environment of
    /// the export, since Postman environments are not tied to a collection.
    pub bundles: Vec<ImportBundle>,
    /// One `Skipped` note per entry not imported, its `source` the entry's path in the export.
    pub skipped: Vec<ImportNote>,
}

/// Why a Postman data export could not be read at all.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ExportError {
    /// The path holds no `archive.json`, so it is not a Postman data export.
    #[error("{} is not a Postman data export: it holds no archive.json", path.display())]
    NotExport {
        /// The path given.
        path: PathBuf,
    },
    /// The path or an entry could not be read.
    #[error("cannot read {}: {reason}", path.display())]
    Unreadable {
        /// The path given.
        path: PathBuf,
        /// Why, in plain words.
        reason: String,
    },
    /// The export is over a size or count limit.
    #[error("the export is larger than {limit} {unit}")]
    TooLarge {
        /// The limit exceeded.
        limit: u64,
        /// What the limit counts: `bytes` or `files`.
        unit: &'static str,
    },
}

/// Whether `path` should be read as a Postman data export: a folder holding `archive.json`,
/// an `archive.json` itself, or a zip archive, which no other import reads. A cheap check; a zip
/// that turns out not to be an export fails `parse_postman_export` with `NotExport`.
pub fn looks_like_postman_export(path: &Path) -> bool {
    if path.is_dir() {
        return path.join(MARKER).is_file();
    }
    if path.file_name().is_some_and(|n| n == MARKER) {
        return path.is_file();
    }
    let mut magic = [0u8; 2];
    File::open(path)
        .and_then(|mut f| f.read_exact(&mut magic))
        .is_ok_and(|()| magic == ZIP_MAGIC)
}

/// Read the Postman data export at `path`, a zip, its unzipped folder or that folder's
/// `archive.json`. An entry that is not a readable collection or environment is skipped with a
/// note and the rest still import; the result may hold no collection at all.
pub fn parse_postman_export(path: &Path) -> Result<PostmanExport, ExportError> {
    parse_with(path, Limits::DEFAULT)
}

pub(crate) fn parse_with(path: &Path, limits: Limits) -> Result<PostmanExport, ExportError> {
    let unreadable = |e: std::io::Error| ExportError::Unreadable {
        path: path.to_path_buf(),
        reason: super::read_reason(&e),
    };
    let meta = fs::metadata(path).map_err(unreadable)?;
    let mut gathered = Gathered::default();
    if meta.is_dir() {
        read_folder(path, path, limits, &mut gathered)?;
    } else if path.file_name().is_some_and(|n| n == MARKER) {
        let root = path.parent().unwrap_or(Path::new("."));
        read_folder(root, root, limits, &mut gathered)?;
    } else {
        read_zip(path, limits, &mut gathered)?;
    }
    gathered.finish(path)
}

/// The entries read so far, sorted into collections, environments and skips.
#[derive(Default)]
struct Gathered {
    marked: bool,
    files: usize,
    bytes: u64,
    collections: Vec<(String, ImportedCollection)>,
    environments: Vec<(String, ImportedEnvironment)>,
    skipped: Vec<ImportNote>,
}

impl Gathered {
    /// Count one more file against the entry cap.
    fn count(&mut self, limits: Limits) -> Result<(), ExportError> {
        self.files += 1;
        if self.files > limits.entries {
            return Err(ExportError::TooLarge {
                limit: limits.entries as u64,
                unit: "files",
            });
        }
        Ok(())
    }

    fn skip(&mut self, label: &str, detail: impl Into<String>) {
        self.skipped
            .push(ImportNote::new(label, NoteKind::Skipped, detail));
    }

    /// Read `reader` within the caps and recognize what it holds. An entry over the entry cap is
    /// skipped; going over the total cap ends the whole read.
    fn take(&mut self, label: &str, reader: impl Read, limits: Limits) -> Result<(), ExportError> {
        let mut bytes = Vec::new();
        if let Err(e) = reader.take(limits.entry + 1).read_to_end(&mut bytes) {
            self.skip(label, format!("cannot be read: {e}"));
            return Ok(());
        }
        self.bytes += bytes.len() as u64;
        if self.bytes > limits.total {
            return Err(ExportError::TooLarge {
                limit: limits.total,
                unit: "bytes",
            });
        }
        if bytes.len() as u64 > limits.entry {
            self.skip(label, format!("larger than {} bytes", limits.entry));
            return Ok(());
        }
        self.recognize(label, &bytes);
        Ok(())
    }

    // The folder layout inside an export is not documented, so each file is judged by its
    // content alone.
    fn recognize(&mut self, label: &str, bytes: &[u8]) {
        if is_marker(label) {
            self.marked = true;
            return;
        }
        if CollectionFormat::detect(bytes) == Some(CollectionFormat::Postman) {
            match parse_collections(
                Some(CollectionFormat::Postman),
                bytes,
                &CollectionOptions::default(),
            ) {
                Ok(bundles) => self.collections.extend(
                    bundles
                        .into_iter()
                        .map(|b| (label.to_owned(), b.collection)),
                ),
                Err(e) => self.skip(label, e.to_string()),
            }
        } else if EnvironmentFormat::detect(bytes) == Some(EnvironmentFormat::Postman) {
            match parse_environments(Some(EnvironmentFormat::Postman), bytes) {
                Ok(envs) => self
                    .environments
                    .extend(envs.into_iter().map(|e| (label.to_owned(), e))),
                Err(e) => self.skip(label, e.to_string()),
            }
        } else if is_globals(bytes) {
            self.skip(
                label,
                "Postman globals are not imported; add them to a global environment",
            );
        } else {
            self.skip(label, "not a Postman collection or environment");
        }
    }

    // Collections and environments are sorted by name, then entry, so the same export always
    // writes the same folders in the same order.
    fn finish(mut self, path: &Path) -> Result<PostmanExport, ExportError> {
        if !self.marked {
            return Err(ExportError::NotExport {
                path: path.to_path_buf(),
            });
        }
        self.collections
            .sort_by(|a, b| (&a.1.name, &a.0).cmp(&(&b.1.name, &b.0)));
        self.environments
            .sort_by(|a, b| (&a.1.name, &a.0).cmp(&(&b.1.name, &b.0)));
        self.skipped.sort_by(|a, b| a.source.cmp(&b.source));
        let environments: Vec<ImportedEnvironment> =
            self.environments.into_iter().map(|(_, e)| e).collect();
        let bundles = self
            .collections
            .into_iter()
            .map(|(_, collection)| ImportBundle {
                collection,
                environments: environments.clone(),
            })
            .collect();
        Ok(PostmanExport {
            bundles,
            skipped: self.skipped,
        })
    }
}

/// Whether the entry named `label` is the export's `archive.json`, at its root or one folder in.
fn is_marker(label: &str) -> bool {
    let segments: Vec<&str> = label.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
    segments.len() <= 2 && segments.last() == Some(&MARKER)
}

fn is_globals(bytes: &[u8]) -> bool {
    load(bytes).is_ok_and(|l| {
        l.value
            .get("_postman_variable_scope")
            .and_then(Value::as_str)
            == Some("globals")
    })
}

// Symbolic links are never followed, so a folder export cannot reach files outside itself.
fn read_folder(
    root: &Path,
    dir: &Path,
    limits: Limits,
    gathered: &mut Gathered,
) -> Result<(), ExportError> {
    let unreadable = |e: std::io::Error| ExportError::Unreadable {
        path: dir.to_path_buf(),
        reason: super::read_reason(&e),
    };
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(unreadable)?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<_, _>>()
        .map_err(unreadable)?;
    entries.sort();
    for path in entries {
        let label = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        let Ok(meta) = fs::symlink_metadata(&path) else {
            gathered.skip(&label, "cannot be read");
            continue;
        };
        if meta.is_symlink() {
            gathered.skip(&label, "a symbolic link is not followed");
        } else if meta.is_dir() {
            if label.split('/').count() >= MAX_DEPTH {
                gathered.skip(&label, "the folder is nested too deep");
            } else {
                read_folder(root, &path, limits, gathered)?;
            }
        } else {
            gathered.count(limits)?;
            match File::open(&path) {
                Ok(file) => gathered.take(&label, file, limits)?,
                Err(e) => gathered.skip(&label, super::read_reason(&e)),
            }
        }
    }
    Ok(())
}

fn read_zip(path: &Path, limits: Limits, gathered: &mut Gathered) -> Result<(), ExportError> {
    let unreadable = |reason: String| ExportError::Unreadable {
        path: path.to_path_buf(),
        reason,
    };
    let mut file = File::open(path).map_err(|e| unreadable(super::read_reason(&e)))?;
    let size = file
        .metadata()
        .map_err(|e| unreadable(super::read_reason(&e)))?
        .len();
    if size > limits.archive {
        return Err(ExportError::TooLarge {
            limit: limits.archive,
            unit: "bytes",
        });
    }
    check_declared_entries(&mut file, size, limits)
        .map_err(|e| unreadable(super::read_reason(&e)))??;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|_| unreadable("it is not a readable zip archive".to_owned()))?;
    if zip.len() > limits.entries {
        return Err(ExportError::TooLarge {
            limit: limits.entries as u64,
            unit: "files",
        });
    }
    for i in 0..zip.len() {
        let label = match zip.name_for_index(i) {
            Some(Ok(name)) => name.into_owned(),
            _ => format!("entry {i}"),
        };
        let entry = match zip.by_index(i) {
            Ok(entry) => entry,
            Err(e) => {
                gathered.skip(&label, format!("cannot be read: {e}"));
                continue;
            }
        };
        if entry.is_dir() {
            continue;
        }
        gathered.count(limits)?;
        if entry.is_symlink() {
            gathered.skip(&label, "a symbolic link is not followed");
            continue;
        }
        gathered.take(&label, entry, limits)?;
    }
    Ok(())
}

// The zip reader loads the whole central directory up front, so the entry count it declares is
// checked first from the end-of-central-directory record. ZIP64 counts are refused outright,
// since an export within the caps never needs them.
fn check_declared_entries(
    file: &mut File,
    size: u64,
    limits: Limits,
) -> std::io::Result<Result<(), ExportError>> {
    const EOCD: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
    let tail_len = size.min(22 + u64::from(u16::MAX));
    file.seek(SeekFrom::Start(size - tail_len))?;
    let mut tail = Vec::new();
    file.by_ref().take(tail_len).read_to_end(&mut tail)?;
    file.seek(SeekFrom::Start(0))?;
    let declared = tail
        .windows(4)
        .rposition(|w| w == EOCD)
        .and_then(|at| tail.get(at + 10..at + 12))
        .map(|count| u16::from_le_bytes([count[0], count[1]]));
    Ok(match declared {
        Some(count) if count != u16::MAX && usize::from(count) <= limits.entries => Ok(()),
        // No record: the zip reader reports the archive as unreadable.
        None => Ok(()),
        Some(_) => Err(ExportError::TooLarge {
            limit: limits.entries as u64,
            unit: "files",
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use zip::write::SimpleFileOptions;

    const ARCHIVE: &str = r#"{"collection":{},"environment":{}}"#;

    fn zip_with(path: &Path, entries: &[(&str, &[u8])]) {
        let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }

    fn small(entries: usize, entry: u64, total: u64) -> Limits {
        Limits {
            archive: MIB,
            entries,
            entry,
            total,
        }
    }

    #[test]
    fn an_entry_that_inflates_past_its_cap_is_skipped_unread() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("e.zip");
        let bomb = vec![b' '; 4096];
        zip_with(
            &path,
            &[("archive.json", ARCHIVE.as_bytes()), ("big.json", &bomb)],
        );
        let export = parse_with(&path, small(10, 1024, MIB)).expect("parsed");
        assert_eq!(export.skipped.len(), 1);
        assert_eq!(export.skipped[0].source, "big.json");
        assert!(export.skipped[0].detail.contains("larger than 1024 bytes"));
    }

    #[test]
    fn going_past_the_total_cap_stops_the_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("e.zip");
        let chunk = vec![b' '; 600];
        zip_with(
            &path,
            &[
                ("archive.json", ARCHIVE.as_bytes()),
                ("a.json", &chunk),
                ("b.json", &chunk),
            ],
        );
        let result = parse_with(&path, small(10, 1024, 1000));
        assert!(matches!(
            result,
            Err(ExportError::TooLarge {
                limit: 1000,
                unit: "bytes"
            })
        ));
    }

    #[test]
    fn a_declared_entry_count_past_the_cap_is_refused_before_reading() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("e.zip");
        let entries: Vec<(String, &[u8])> = (0..5)
            .map(|i| (format!("{i}.json"), b"{}" as &[u8]))
            .collect();
        let refs: Vec<(&str, &[u8])> = entries.iter().map(|(n, b)| (n.as_str(), *b)).collect();
        zip_with(&path, &refs);
        let result = parse_with(&path, small(4, 1024, MIB));
        assert!(matches!(
            result,
            Err(ExportError::TooLarge {
                limit: 4,
                unit: "files"
            })
        ));
    }

    #[test]
    fn a_zip_over_the_archive_cap_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("e.zip");
        zip_with(&path, &[("archive.json", ARCHIVE.as_bytes())]);
        let limits = Limits {
            archive: 10,
            ..small(10, 1024, MIB)
        };
        assert!(matches!(
            parse_with(&path, limits),
            Err(ExportError::TooLarge {
                limit: 10,
                unit: "bytes"
            })
        ));
    }

    #[test]
    fn a_folder_with_too_many_files_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(MARKER), ARCHIVE).unwrap();
        for i in 0..4 {
            fs::write(dir.path().join(format!("{i}.json")), "{}").unwrap();
        }
        assert!(matches!(
            parse_with(dir.path(), small(3, 1024, MIB)),
            Err(ExportError::TooLarge {
                limit: 3,
                unit: "files"
            })
        ));
    }
}
