//! The rules applied to a manifest whose signature has already been verified.

use std::collections::BTreeMap;

use semver::Version;
use serde::Deserialize;

use super::notes::{is_sha256, notes_asset};
use super::{Available, Format, UpdateConfig, UpdateError, allowed};

// Unknown fields are ignored so later manifests can add to the format.
#[derive(Deserialize)]
struct Manifest {
    version: String,
    notes: Option<String>,
    pub_date: Option<String>,
    // Read loosely, so a malformed pointer costs only the notes.
    notes_asset: Option<serde_json::Value>,
    targets: BTreeMap<String, Entry>,
}

#[derive(Deserialize)]
struct Entry {
    url: String,
    format: String,
    size: u64,
    sha256: String,
}

/// Applies the manifest rules to verified `manifest` bytes: a stable release strictly newer
/// than `cfg.current_version` is `Some`, with the entry for `cfg.target_id`; an older or equal
/// one is `None`. A malformed document, a pre-release or a bad entry is
/// [`UpdateError::ManifestInvalid`]; a newer release without this target is
/// [`UpdateError::NoEntryForTarget`].
pub fn evaluate(manifest: &[u8], cfg: &UpdateConfig) -> Result<Option<Available>, UpdateError> {
    let invalid = |m: String| UpdateError::ManifestInvalid(m);
    let doc: Manifest = serde_json::from_slice(manifest).map_err(|e| invalid(e.to_string()))?;
    let remote = Version::parse(&doc.version)
        .map_err(|e| invalid(format!("version {:?}: {e}", doc.version)))?;
    if !remote.pre.is_empty() {
        return Err(invalid(format!("{} is a pre-release", doc.version)));
    }
    let current = Version::parse(&cfg.current_version)
        .map_err(|e| invalid(format!("current version {:?}: {e}", cfg.current_version)))?;
    // Build metadata carries no precedence, so a rebuild of the same release is not newer.
    if remote.cmp_precedence(&current).is_le() {
        return Ok(None);
    }

    let mut targets = doc.targets;
    let entry = targets
        .remove(&cfg.target_id)
        .ok_or_else(|| UpdateError::NoEntryForTarget(cfg.target_id.clone()))?;
    let format = match entry.format.as_str() {
        "appimage" => Format::AppImage,
        "app" => Format::App,
        "nsis" => Format::Nsis,
        "binary" => Format::Binary,
        other => return Err(invalid(format!("unknown format {other:?}"))),
    };
    if !url::Url::parse(&entry.url).is_ok_and(|u| allowed(&u)) {
        return Err(invalid(format!(
            "the download URL must be https: {}",
            entry.url
        )));
    }
    if !is_sha256(&entry.sha256) {
        return Err(invalid(format!(
            "sha256 {:?} is not 64 lowercase hex",
            entry.sha256
        )));
    }
    let (asset, notes_error) = match doc.notes_asset.map(notes_asset) {
        Some(Ok(asset)) => (Some(asset), None),
        Some(Err(why)) => (None, Some(why)),
        None => (None, None),
    };
    Ok(Some(Available {
        version: doc.version,
        notes: doc.notes,
        release_notes: None,
        notes_error,
        pub_date: doc.pub_date,
        format,
        url: entry.url,
        sha256: entry.sha256,
        size: entry.size,
        limits: cfg.limits,
        notes_asset: asset,
    }))
}
