//! Carries out a [`ConversionPlan`]: into a new folder (write the YAML, copy the rest) or in
//! place (write the YAML beside the sources, then delete the legacy files).

use super::error::ConvertError;
use super::plan::ConversionPlan;
use crate::Collection;
use crate::naming::sanitize_name;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests;

/// Where a conversion puts the converted collection.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ConvertTarget {
    /// Exactly this directory, which must not exist; its parent must.
    NewFolder(PathBuf),
    /// A new directory inside this one, named after the collection like an import names it.
    NewFolderIn(PathBuf),
    /// The legacy collection's own root: the YAML is written beside the sources, which are
    /// then deleted.
    InPlace,
}

/// One filesystem step a conversion is about to take, as seen by a test hook.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "only fault-injection tests read the paths")
)]
pub(crate) enum Step<'a> {
    /// A file about to be written or copied, at its final or staged absolute path.
    Write(&'a Path),
    /// The staged root about to be renamed into place.
    Rename(&'a Path),
    /// A legacy file about to be deleted in place.
    Remove(&'a Path),
}

/// Writes `plan` to `target` and returns the converted collection's root.
///
/// Fails with `SourceChanged` when a source differs from when it was planned, `InvalidName`
/// when the collection name gives no folder name, `TargetExists` when the target directory
/// (or, in place, a file to create) exists, and `Io` otherwise. A new-folder failure leaves
/// the target's parent as it was; an in-place failure before the deletes leaves the root as
/// it was. A failed delete stops with the YAML written and the rest of the sources kept. A
/// source edited during the write is kept and reported as `SourceChanged` after the others
/// are deleted.
pub fn convert(plan: &ConversionPlan, target: ConvertTarget) -> Result<PathBuf, ConvertError> {
    convert_with(plan, target, &mut |_| Ok(()))
}

/// Fails with `TargetExists` when `dir`, a new-folder target, already exists (a dangling
/// symlink counts); the check `convert` makes before it stages anything.
pub fn ensure_new_folder_free(dir: &Path) -> Result<(), ConvertError> {
    match fs::symlink_metadata(dir) {
        Ok(_) => Err(ConvertError::TargetExists {
            paths: vec![dir.to_path_buf()],
        }),
        Err(_) => Ok(()),
    }
}

/// `convert` with `hook` called before each file written or copied (its final or staged
/// absolute path), before the rename (the staged root) and before each in-place delete;
/// a hook error becomes `ConvertError::Io` for that path.
pub(crate) fn convert_with(
    plan: &ConversionPlan,
    target: ConvertTarget,
    hook: &mut dyn FnMut(Step<'_>) -> io::Result<()>,
) -> Result<PathBuf, ConvertError> {
    // Verify the sources before touching the target, so a refusal leaves no debris.
    check_sources(plan)?;
    let dir = match target {
        ConvertTarget::NewFolder(dir) => dir,
        ConvertTarget::NewFolderIn(parent) => {
            // Validate the folder name actually chosen: a taken name's " - N" variant can
            // outgrow the byte cap even when the name itself fits.
            let folder = sanitize_name(&Collection::unique_dir_name(&parent, plan.name()));
            if !Collection::is_valid_dir_stem(&sanitize_name(plan.name()))
                || !Collection::is_valid_dir_stem(&folder)
            {
                return Err(ConvertError::InvalidName {
                    name: plan.name().to_owned(),
                });
            }
            parent.join(folder)
        }
        ConvertTarget::InPlace => return in_place(plan, hook),
    };
    new_folder(plan, dir, hook)
}

/// Re-read every hashed source; a different digest or an unreadable file is `SourceChanged`.
fn check_sources(plan: &ConversionPlan) -> Result<(), ConvertError> {
    for (rel, digest) in &plan.hashes {
        let same = fs::read(plan.root.join(rel))
            .is_ok_and(|bytes| Sha256::digest(&bytes).as_slice() == digest.as_slice());
        if !same {
            return Err(ConvertError::SourceChanged { path: rel.clone() });
        }
    }
    Ok(())
}

/// Build the collection in a staging directory beside `dir`, then rename it into place, so
/// any failure is cleaned up by dropping the staging directory.
fn new_folder(
    plan: &ConversionPlan,
    dir: PathBuf,
    hook: &mut dyn FnMut(Step<'_>) -> io::Result<()>,
) -> Result<PathBuf, ConvertError> {
    ensure_new_folder_free(&dir)?;
    let parent = match dir.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let staging = tempfile::Builder::new()
        .prefix(".apinata-convert-")
        .tempdir_in(parent)
        .map_err(|e| io_error(parent, e))?;
    let staged = staging.path().join("collection");
    fs::create_dir(&staged).map_err(|e| io_error(&dir, e))?;

    // Errors name the path under the real target, never the hidden staging directory; only
    // the hook sees the staged one.
    for (rel, text) in &plan.files {
        let path = staged.join(rel);
        let real = dir.join(rel);
        prepare(&path, &real, hook)?;
        fs::write(&path, text).map_err(|e| io_error(&real, e))?;
    }
    for rel in &plan.copies {
        let path = staged.join(rel);
        let real = dir.join(rel);
        prepare(&path, &real, hook)?;
        copy_verified(plan, rel, &path, &real)?;
    }

    hook(Step::Rename(&staged)).map_err(|e| io_error(&staged, e))?;
    // `rename(2)` silently replaces an empty directory on Unix, so a target that appeared
    // since the check above is refused before the rename rather than by it.
    ensure_new_folder_free(&dir)?;
    fs::rename(&staged, &dir).map_err(|e| match e.kind() {
        ErrorKind::AlreadyExists | ErrorKind::DirectoryNotEmpty => ConvertError::TargetExists {
            paths: vec![dir.clone()],
        },
        _ => io_error(&dir, e),
    })?;
    drop(staging);
    Ok(dir)
}

/// Write each YAML document beside its source, `opencollection.yml` last so an interrupted
/// run still opens as legacy, then delete the `.bru` files and `bruno.json` last. A write
/// failure removes only the files this call created; a delete failure is not undone.
fn in_place(
    plan: &ConversionPlan,
    hook: &mut dyn FnMut(Step<'_>) -> io::Result<()>,
) -> Result<PathBuf, ConvertError> {
    let existing: Vec<PathBuf> = plan
        .files
        .iter()
        .map(|(rel, _)| rel)
        .filter(|rel| fs::symlink_metadata(plan.root.join(rel)).is_ok())
        .cloned()
        .collect();
    if !existing.is_empty() {
        return Err(ConvertError::TargetExists { paths: existing });
    }

    let root_file = Path::new(crate::collection::ROOT_FILE);
    let (last, rest): (Vec<_>, Vec<_>) = plan.files.iter().partition(|(rel, _)| rel == root_file);
    let mut written = Vec::new();
    for (rel, text) in rest.into_iter().chain(last) {
        let path = plan.root.join(rel);
        if let Err(error) = write_new(&path, rel, text, &mut written, hook) {
            // Undo only this call's files; a cleanup error is ignored so the cause is reported.
            for done in written.iter().rev() {
                let _ = fs::remove_file(done);
            }
            return Err(error);
        }
    }

    let bruno_json = Path::new("bruno.json");
    let (json, bru): (Vec<_>, Vec<_>) = plan.removals.iter().partition(|rel| *rel == bruno_json);
    // A source edited since it was hashed is kept, so the edit is not lost; the first such
    // file is reported once every unchanged source is gone.
    let mut changed = None;
    for rel in bru.into_iter().chain(json) {
        let path = plan.root.join(rel);
        hook(Step::Remove(&path)).map_err(|e| io_error(&path, e))?;
        if source_changed(plan, rel) {
            changed.get_or_insert_with(|| rel.clone());
            continue;
        }
        fs::remove_file(&path).map_err(|e| io_error(&path, e))?;
    }
    match changed {
        Some(path) => Err(ConvertError::SourceChanged { path }),
        None => Ok(plan.root.clone()),
    }
}

/// Whether `rel` no longer matches the digest taken when it was planned (unreadable counts).
fn source_changed(plan: &ConversionPlan, rel: &Path) -> bool {
    let Some((_, digest)) = plan.hashes.iter().find(|(p, _)| p == rel) else {
        return false;
    };
    fs::read(plan.root.join(rel))
        .map(|bytes| Sha256::digest(&bytes).as_slice() != digest.as_slice())
        .unwrap_or(true)
}

/// Create `path` exclusively and write `text`, recording it in `written` once it exists, so
/// a file another writer created is reported as `TargetExists` and never touched.
fn write_new(
    path: &Path,
    rel: &Path,
    text: &str,
    written: &mut Vec<PathBuf>,
    hook: &mut dyn FnMut(Step<'_>) -> io::Result<()>,
) -> Result<(), ConvertError> {
    hook(Step::Write(path)).map_err(|e| io_error(path, e))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| match e.kind() {
            ErrorKind::AlreadyExists => ConvertError::TargetExists {
                paths: vec![rel.to_path_buf()],
            },
            _ => io_error(path, e),
        })?;
    written.push(path.to_path_buf());
    file.write_all(text.as_bytes())
        .map_err(|e| io_error(path, e))
}

/// Create the staged file's directory and give the hook its turn; failures name `real`.
fn prepare(
    path: &Path,
    real: &Path,
    hook: &mut dyn FnMut(Step<'_>) -> io::Result<()>,
) -> Result<(), ConvertError> {
    if let Some(dir) = path.parent() {
        let real_dir = real.parent().unwrap_or(real);
        fs::create_dir_all(dir).map_err(|e| io_error(real_dir, e))?;
    }
    hook(Step::Write(path)).map_err(|e| io_error(real, e))
}

/// Copy the planned source `rel` to `staged`, hashing the bytes as they stream through and
/// comparing with the digest taken at planning. A source that is now a symlink, is gone, or
/// differs (or was never hashed) is `SourceChanged`; other failures name `real`.
fn copy_verified(
    plan: &ConversionPlan,
    rel: &Path,
    staged: &Path,
    real: &Path,
) -> Result<(), ConvertError> {
    let changed = || ConvertError::SourceChanged {
        path: rel.to_path_buf(),
    };
    let mut source = open_regular(&plan.root.join(rel)).map_err(|e| match e.kind() {
        ErrorKind::InvalidData | ErrorKind::NotFound => changed(),
        _ => io_error(real, e),
    })?;
    let mut dest = File::create(staged).map_err(|e| io_error(real, e))?;
    let digest = hash_stream(&mut source, Some(&mut dest)).map_err(|e| io_error(real, e))?;
    let permissions = source.metadata().map(|m| m.permissions());
    if let Ok(permissions) = permissions {
        dest.set_permissions(permissions)
            .map_err(|e| io_error(real, e))?;
    }
    match plan.copy_hashes.iter().find(|(p, _)| p == rel) {
        Some((_, planned)) if planned == &digest => Ok(()),
        _ => Err(changed()),
    }
}

/// Open `path` for reading only if it is a regular file and not a symlink; otherwise fail
/// with `InvalidData`. On Unix the opened handle must also be the file the link check saw,
/// so a link swapped in between the check and the open is caught.
pub(super) fn open_regular(path: &Path) -> io::Result<File> {
    let before = fs::symlink_metadata(path)?;
    if !before.file_type().is_file() {
        return Err(io::Error::new(ErrorKind::InvalidData, "not a regular file"));
    }
    let file = File::open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let opened = file.metadata()?;
        if opened.dev() != before.dev() || opened.ino() != before.ino() {
            return Err(io::Error::new(ErrorKind::InvalidData, "file was replaced"));
        }
    }
    Ok(file)
}

/// SHA-256 of everything `reader` yields, also writing it to `sink` when one is given.
pub(super) fn hash_stream(
    reader: &mut impl Read,
    mut sink: Option<&mut File>,
) -> io::Result<Vec<u8>> {
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 16 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            return Ok(hasher.finalize().to_vec());
        }
        hasher.update(&buf[..n]);
        if let Some(sink) = sink.as_mut() {
            sink.write_all(&buf[..n])?;
        }
    }
}

fn io_error(path: &Path, error: io::Error) -> ConvertError {
    ConvertError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}
