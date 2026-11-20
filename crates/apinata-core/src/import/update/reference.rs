//! Where a collection's spec comes from: our own record under `extensions.apinata.openapi`,
//! written by each import and update from a link or file, or else the reference app's under
//! `extensions.bruno.openapi`. A collection with only the latter is adopted on its first update.

use super::scope_baseline;
use crate::collection::Collection;
use crate::collection::ROOT_FILE;
use crate::config_file::CollectionFile;
use crate::import::{GroupBy, ImportNote, NoteKind};
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

/// Record `spec` as the source the root at `root` came from, unless it already is. A file is
/// kept relative to the root, so the record is shareable; a link that may carry a secret, or a
/// file no relative path reaches, leaves any earlier record as it was.
pub(crate) fn record_source(file: &mut CollectionFile, spec: &RecordedSpec, root: &Path) {
    let source = match &spec.location {
        SpecLocation::Link(url) if may_hold_secret(url) => return,
        SpecLocation::Link(url) => url.clone(),
        SpecLocation::File(path) => match stored_path(path, root) {
            Some(path) => path,
            None => return,
        },
    };
    let group = match spec.group_by {
        GroupBy::Path => "path",
        _ => "tags",
    };
    if file.spec_source() != Some((source.clone(), Some(group.to_owned()))) {
        file.set_spec_source(&source, group);
    }
}

/// The note for a source that is read but not recorded because its link may hold a secret.
pub(crate) fn unrecorded_note(spec: Option<&RecordedSpec>) -> Option<ImportNote> {
    let SpecLocation::Link(url) = &spec?.location else {
        return None;
    };
    may_hold_secret(url).then(|| {
        let detail = "the link was not remembered for the next update because it has a query or \
                      fragment, which can hold a token";
        ImportNote::new(ROOT_FILE, NoteKind::Dropped, detail)
    })
}

// A query or fragment can carry a token or a signed, expiring address.
fn may_hold_secret(url: &str) -> bool {
    url.contains(['?', '#'])
}

// `path` relative to `root` with `/` separators, both resolved first so a symlinked temp or
// home directory still compares equal. `None` when they share no root, such as two drives.
fn stored_path(path: &Path, root: &Path) -> Option<String> {
    let (path, root) = (resolve(path)?, resolve(root)?);
    let (path, root): (Vec<_>, Vec<_>) = (path.components().collect(), root.components().collect());
    let shared = path.iter().zip(&root).take_while(|(a, b)| a == b).count();
    if shared == 0 {
        return None;
    }
    let up = root[shared..].iter().map(|_| "..".into());
    let down = path[shared..]
        .iter()
        .map(|c| c.as_os_str().to_string_lossy().into_owned());
    Some(up.chain(down).collect::<Vec<String>>().join("/"))
}

// The real path of `p`; a part not created yet (a collection still being written) is joined
// onto its nearest existing ancestor.
fn resolve(p: &Path) -> Option<PathBuf> {
    let p = std::path::absolute(p).ok()?;
    let mut missing = Vec::new();
    let mut at = p.as_path();
    loop {
        if let Ok(real) = at.canonicalize() {
            return Some(
                missing
                    .iter()
                    .rev()
                    .fold(real, |acc: PathBuf, part| acc.join(part)),
            );
        }
        missing.push(at.file_name()?.to_owned());
        at = at.parent()?;
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
