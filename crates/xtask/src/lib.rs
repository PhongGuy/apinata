//! Repository automation run as `cargo xtask`; each subcommand lives in its own module.

// Developer tooling behind `cargo xtask`: an expect that fires is a bug report naming why.
#![allow(clippy::expect_used)]

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, bail};

pub mod dev_desktop;
pub mod icons;
pub mod packager_config;
pub mod update_manifest;
pub mod windows_check;

/// The repository root, two levels above this crate's manifest.
pub fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|crates| crates.parent())
        .expect("xtask lives at crates/xtask")
        .to_path_buf()
}

/// `cargo metadata --no-deps --format-version 1` for the workspace, parsed.
pub fn cargo_metadata() -> anyhow::Result<serde_json::Value> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let manifest = workspace_root().join("Cargo.toml");
    let out = Command::new(&cargo)
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(&manifest)
        .output()
        .context("running cargo metadata")?;
    if !out.status.success() {
        bail!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    serde_json::from_slice(&out.stdout).context("parsing cargo metadata")
}
