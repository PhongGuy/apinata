//! The demo collection bundled into the binary and the helper that copies it to disk.

use apinata_core::Collection;
use rust_embed::RustEmbed;
use std::io;
use std::path::{Path, PathBuf};

// Secrets and git metadata never ship, even if one appears in the source folder.
#[derive(RustEmbed)]
#[folder = "../../examples/demo"]
#[exclude = ".env"]
#[exclude = ".env.*"]
#[exclude = ".git*"]
#[exclude = "**/.env"]
#[exclude = "**/.env.*"]
#[exclude = "**/.git*"]
struct Demo;

/// Every bundled file as a slash-separated relative path and its bytes.
pub fn files() -> Vec<(String, Vec<u8>)> {
    Demo::iter()
        .filter_map(|name| Some((name.to_string(), Demo::get(&name)?.data.into_owned())))
        .collect()
}

/// Copies the demo into a new folder of `parent` named "Apinata Demo" (or the first free
/// "Apinata Demo - n") and answers its root. The tree is staged beside the target and renamed
/// once, so a failure leaves nothing half written and an existing folder is never touched.
pub fn install(parent: &Path) -> io::Result<PathBuf> {
    let stage = tempfile::Builder::new()
        .prefix(".apinata-demo-")
        .tempdir_in(parent)?;
    for (name, data) in files() {
        let path = stage.path().join(&name);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, data)?;
    }
    let target = parent.join(Collection::unique_dir_name(parent, "Apinata Demo"));
    std::fs::rename(stage.path(), &target)?;
    // The staging folder now lives at `target`; keep the guard from deleting a missing path.
    let _ = stage.keep();
    Ok(target)
}
