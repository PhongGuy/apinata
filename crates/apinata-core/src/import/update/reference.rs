//! The OpenAPI source the reference app records under `extensions.bruno.openapi`. A collection
//! with that record and none of our own is adopted on its first update.

use super::scope_baseline;
use crate::collection::Collection;
use crate::config_file::CollectionFile;
use crate::import::GroupBy;
use crate::item_path::ItemPath;
use std::path::PathBuf;

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

/// The first usable source in the reference app's record, offered only while the collection
/// root has no update record of our own; `None` otherwise.
pub fn recorded_spec(c: &Collection) -> Option<RecordedSpec> {
    if c.file().spec_baseline().is_some() {
        return None;
    }
    let (source, group) = c.file().reference_openapi_source()?;
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
