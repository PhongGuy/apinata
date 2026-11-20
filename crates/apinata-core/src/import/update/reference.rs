//! Where a collection's spec comes from: our own record under `extensions.apinata.openapi`,
//! written by each import and update from a link or file, or else the reference app's under
//! `extensions.bruno.openapi`. A collection with only the latter is adopted on its first update.

use super::scope_baseline;
use crate::collection::Collection;
use crate::config_file::CollectionFile;
use crate::import::GroupBy;
use crate::item_path::ItemPath;
use std::path::{Path, PathBuf};

/// Where a recorded spec lives.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SpecLocation {
    /// An http or https link, fetched like any other import link.
    Link(String),
    /// A local file, resolved against the collection root.
    File(PathBuf),
}

/// The spec a collection's reference app record points at, and how it groups operations.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RecordedSpec {
    /// The spec's link or file.
    pub location: SpecLocation,
    /// The grouping the record names; tags when it names none.
    pub group_by: GroupBy,
}

/// The source the collection root was last imported or updated from: our own record when it
/// holds one, else the first usable source in the reference app's record.
pub fn recorded_spec(c: &Collection) -> Option<RecordedSpec> {
    let (source, group) = c
        .file()
        .spec_source()
        .or_else(|| c.file().reference_openapi_source())?;
    // The reference app treats anything that is not an http(s) link as a collection-relative path.
    let lower = source.to_ascii_lowercase();
    let location = if lower.starts_with("http://") || lower.starts_with("https://") {
        SpecLocation::Link(source)
    } else {
        SpecLocation::File(c.root().join(source))
    };
    let group_by = match group.as_deref() {
        Some("path") => GroupBy::Path,
        _ => GroupBy::Tags,
    };
    Some(RecordedSpec { location, group_by })
}

/// Record `spec` as the source the root at `root` came from, unless it already is. A file
/// inside the collection is kept relative to it, so the record survives a move or a clone.
pub(crate) fn record_source(file: &mut CollectionFile, spec: &RecordedSpec, root: &Path) {
    let source = match &spec.location {
        SpecLocation::Link(url) => url.clone(),
        SpecLocation::File(path) => stored_path(path, root),
    };
    let group = match spec.group_by {
        GroupBy::Path => "path",
        _ => "tags",
    };
    if file.spec_source() != Some((source.clone(), Some(group.to_owned()))) {
        file.set_spec_source(&source, group);
    }
}

// Both sides are resolved first, so a symlinked temp or home directory still compares equal; a
// root not created yet falls back to its absolute form.
fn stored_path(path: &Path, root: &Path) -> String {
    let resolve = |p: &Path| p.canonicalize().or_else(|_| std::path::absolute(p));
    let (Ok(path), Ok(root)) = (resolve(path), resolve(root)) else {
        return path.to_string_lossy().into_owned();
    };
    match path.strip_prefix(&root) {
        Ok(inside) => inside
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

/// Whether `folder` (the root when it is the root path) can be updated from a spec: it has
/// an update record, or it is the root of a collection the reference app records a source for.
pub fn can_update(c: &Collection, folder: &ItemPath) -> bool {
    update_baseline(c, folder).is_some()
}

/// The baseline an update of `scope` starts from, and whether it is adopting: an empty one
/// for a root that only the reference app records.
pub(super) fn update_baseline(
    c: &Collection,
    scope: &ItemPath,
) -> Option<(super::SpecBaseline, bool)> {
    if let Some(baseline) = scope_baseline(c, scope) {
        return Some((baseline, false));
    }
    let adoptable = scope.is_root() && adoptable(c.file());
    adoptable.then(|| (super::SpecBaseline::default(), true))
}

fn adoptable(file: &CollectionFile) -> bool {
    file.spec_baseline().is_none() && file.reference_openapi_source().is_some()
}
