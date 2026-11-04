//! Runs a write as an attempt that is retried once after a save conflict, dropping the
//! draft of any attempt that fails so the next call cannot save it by accident.

use std::path::{Path, PathBuf};

use apinata_core::{Collection, EnvError, ItemError, ItemPath, SaveError, item_ops};

use crate::apply::{ConfigPatch, RequestPatch, apply_request, folder_patch};
use crate::dto::{self, ConfigDoc, RequestDoc};
use crate::reads;

/// Why one write attempt stopped.
#[derive(Debug)]
pub(crate) enum Failure {
    /// `path` changed on disk between the reload and the save.
    Conflict(PathBuf),
    /// Refused or failed for another reason; the draft at `revert`, if any, is dropped.
    Failed {
        message: String,
        revert: Option<PathBuf>,
    },
}

impl Failure {
    /// A failure whose draft at `path` must be dropped.
    pub(crate) fn at(path: &Path, message: impl Into<String>) -> Failure {
        Failure::Failed {
            message: message.into(),
            revert: Some(path.to_owned()),
        }
    }

    /// A failure decided before anything was edited.
    pub(crate) fn refused(message: impl Into<String>) -> Failure {
        Failure::Failed {
            message: message.into(),
            revert: None,
        }
    }

    /// Map a failed save: a conflict is retried, anything else drops the draft.
    pub(crate) fn save(e: SaveError) -> Failure {
        match e {
            SaveError::Conflict { path } => Failure::Conflict(path),
            SaveError::Io { ref path, .. } => Failure::at(path, e.to_string()),
            other => Failure::refused(other.to_string()),
        }
    }

    /// Map a failed item operation: a conflict is retried, anything else was refused or is
    /// already on disk, so no draft is left to drop.
    pub(crate) fn item(e: ItemError) -> Failure {
        match e.conflict_path() {
            Some(path) => Failure::Conflict(path.to_owned()),
            None => Failure::refused(e.to_string()),
        }
    }

    /// Map a failed environment call: a failed save maps as [`Failure::save`]; anything else
    /// was refused before a file changed.
    pub(crate) fn env(e: EnvError) -> Failure {
        match e {
            EnvError::Save(e) => Failure::save(e),
            other => Failure::refused(other.to_string()),
        }
    }
}

/// Parse an agent-supplied item path, naming it when it is not a valid relative path.
pub(crate) fn item_path(raw: &str) -> Result<ItemPath, String> {
    ItemPath::new(raw).ok_or_else(|| format!("invalid item path: {raw}"))
}

/// The advice for a write that lost to another program twice: most likely the app with
/// unsaved edits, which the user settles there.
pub(crate) fn conflict_message(path: &Path) -> String {
    format!(
        "{} was changed by another program, likely the Apinata app with unsaved edits; nothing \
         was written. Ask the user to save or revert there, then retry.",
        path.display()
    )
}

/// Run `attempt`; after a conflict, drop the stale draft and run it once more against what
/// the other writer saved. Any other failure drops its draft and returns its message.
pub(crate) fn write_retrying<T>(
    c: &mut Collection,
    mut attempt: impl FnMut(&mut Collection) -> Result<T, Failure>,
) -> Result<T, String> {
    let mut retried = false;
    loop {
        match attempt(c) {
            Ok(value) => return Ok(value),
            Err(Failure::Conflict(path)) => {
                // Reverting, not reloading: a reload keeps the dirty draft, so the retry would
                // conflict again; dropping it re-reads the other writer's bytes.
                let _ = c.revert(&path);
                if retried {
                    return Err(conflict_message(&path));
                }
                retried = true;
            }
            Err(Failure::Failed { message, revert }) => {
                if let Some(path) = revert {
                    let _ = c.revert(&path);
                }
                return Err(message);
            }
        }
    }
}

/// Apply `patch` to the request it names in the reloaded `c`, save it and return the document
/// as saved. Errors name the path; a refused section leaves no draft behind.
pub(crate) fn update_request(
    c: &mut Collection,
    patch: &RequestPatch,
) -> Result<RequestDoc, String> {
    let path = item_path(&patch.path)?;
    if c.request(&path).is_none() {
        // Reading names an opaque item's kind or the missing path, so its error is the answer.
        return reads::request(c, &patch.path);
    }
    write_retrying(c, |c| {
        let r = c
            .request_mut(&path)
            .ok_or_else(|| Failure::refused(format!("no request at {}", patch.path)))?;
        let file = r.path().to_owned();
        apply_request(r, patch).map_err(|m| Failure::at(&file, format!("{}: {m}", patch.path)))?;
        r.save().map_err(Failure::save)?;
        dto::request_doc(r).map_err(Failure::refused)
    })
}

/// Apply `patch` to the folder it names in the reloaded `c` and save it: `""` edits
/// opencollection.yml, a folder edits folder.yml, created on first save. Returns the saved
/// document; a refused section leaves no draft behind.
pub(crate) fn update_folder(c: &mut Collection, patch: &ConfigPatch) -> Result<ConfigDoc, String> {
    // Reading names a bad path, a missing folder or the load error of a broken folder.yml, so
    // it is checked before anything is edited.
    reads::folder(c, &patch.path)?;
    write_retrying(c, |c| {
        let current = reads::folder(c, &patch.path).map_err(Failure::refused)?;
        let core = folder_patch(&current, patch).map_err(|m| {
            Failure::refused(if patch.path.is_empty() {
                m
            } else {
                format!("{}: {m}", patch.path)
            })
        })?;
        item_ops::update_folder(c, &patch.path, &core).map_err(Failure::item)?;
        reads::folder(c, &patch.path).map_err(Failure::refused)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: shop\n";

    fn request(url: &str) -> String {
        format!("info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: {url}\n")
    }

    fn open() -> (tempfile::TempDir, Collection) {
        let dir = tempfile::TempDir::new().expect("tempdir");
        fs::write(dir.path().join("opencollection.yml"), ROOT).expect("write");
        fs::write(dir.path().join("r.yml"), request("http://localhost/a")).expect("write");
        let c = Collection::open(dir.path()).expect("opens");
        (dir, c)
    }

    fn r() -> ItemPath {
        ItemPath::new("r.yml").expect("path")
    }

    /// Set the method to POST, but let another program save `url` first when `interfere` says so.
    fn post(c: &mut Collection, dir: &Path, interfere: Option<&str>) -> Result<(), Failure> {
        let req = c.request_mut(&r()).expect("request");
        req.set_method("POST");
        if let Some(url) = interfere {
            fs::write(dir.join("r.yml"), request(url)).expect("other writer");
        }
        req.save().map_err(Failure::save)
    }

    #[test]
    fn a_conflict_is_retried_once_from_what_the_other_writer_saved() {
        let (dir, mut c) = open();
        let mut calls = 0;
        write_retrying(&mut c, |c| {
            calls += 1;
            post(
                c,
                dir.path(),
                (calls == 1).then_some("http://localhost/gui"),
            )
        })
        .expect("the retry succeeds");
        assert_eq!(calls, 2);
        let text = fs::read_to_string(dir.path().join("r.yml")).expect("read");
        assert!(
            text.contains("method: POST") && text.contains("localhost/gui"),
            "{text}"
        );
    }

    #[test]
    fn a_second_conflict_names_the_file_and_keeps_the_other_writers_bytes() {
        let (dir, mut c) = open();
        let mut calls = 0;
        let err = write_retrying(&mut c, |c| {
            calls += 1;
            post(c, dir.path(), Some(&format!("http://localhost/gui{calls}")))
        })
        .expect_err("gives up");
        assert_eq!(calls, 2);
        assert!(err.contains("r.yml"), "{err}");
        assert_eq!(
            fs::read_to_string(dir.path().join("r.yml")).expect("read"),
            request("http://localhost/gui2")
        );
        let req = c.request(&r()).expect("request");
        assert!(
            !req.is_dirty() && req.method() == "GET",
            "the draft is dropped"
        );
    }

    #[test]
    fn a_failed_attempt_drops_its_draft_and_reports_the_message() {
        let (dir, mut c) = open();
        let file = dir.path().join("r.yml");
        let err = write_retrying::<()>(&mut c, |c| {
            c.request_mut(&r()).expect("request").set_method("POST");
            Err(Failure::at(&file, "refused for a reason"))
        })
        .expect_err("fails");
        assert_eq!(err, "refused for a reason");
        let req = c.request(&r()).expect("request");
        assert!(!req.is_dirty() && req.method() == "GET");
    }
}
