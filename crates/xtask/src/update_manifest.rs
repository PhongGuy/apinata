//! `cargo xtask update-manifest`: writes the unsigned `latest.json` the updater reads.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context, bail};
use serde::Serialize;
use serde_json::Map;
use sha2::{Digest, Sha256};

#[derive(Serialize)]
struct Manifest<'a> {
    version: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub_date: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    notes: Option<&'a str>,
    targets: Map<String, serde_json::Value>,
}

#[derive(Serialize)]
struct Entry {
    url: String,
    format: &'static str,
    size: u64,
    sha256: String,
}

/// The `latest.json` text for the files in `dir` named by `map_text` ("<target-id> <file-name>" per line).
pub fn build(
    version: &str,
    base_url: &str,
    notes: &str,
    map_text: &str,
    pub_date: Option<&str>,
    dir: &Path,
) -> anyhow::Result<String> {
    let parsed = semver::Version::parse(version).with_context(|| format!("version {version:?}"))?;
    if !parsed.pre.is_empty() {
        bail!("version {version} is a pre-release");
    }
    let base = base_url.trim_end_matches('/');
    let mut seen = HashSet::new();
    let mut targets = Map::new();
    for line in map_text.lines().filter(|l| !l.trim().is_empty()) {
        let [id, file] = line.split_whitespace().collect::<Vec<_>>()[..] else {
            bail!("map line {line:?} needs exactly a target id and a file name");
        };
        let format = format_for(id).with_context(|| format!("map line {line:?}"))?;
        if !seen.insert(id) {
            bail!("target {id} appears twice in the map");
        }
        // The name becomes a URL path segment, so only characters that need no escaping pass.
        if !file
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._+-".contains(c))
        {
            bail!("file name {file:?} has a character outside [A-Za-z0-9._+-]");
        }
        let bytes =
            std::fs::read(dir.join(file)).with_context(|| format!("reading artifact {file}"))?;
        let entry = Entry {
            url: format!("{base}/{file}"),
            format,
            size: bytes.len() as u64,
            sha256: Sha256::digest(&bytes)
                .iter()
                .fold(String::new(), |mut hex, b| {
                    let _ = write!(hex, "{b:02x}");
                    hex
                }),
        };
        targets.insert(id.to_owned(), serde_json::to_value(entry)?);
    }
    if targets.is_empty() {
        bail!("the map is empty");
    }
    let notes = Some(notes.trim()).filter(|n| !n.is_empty());
    let manifest = Manifest {
        version,
        pub_date,
        notes,
        targets,
    };
    Ok(serde_json::to_string_pretty(&manifest)? + "\n")
}

/// The package format a target id ships, or an error when the id is not `<product>-<os>-<arch>`.
fn format_for(id: &str) -> anyhow::Result<&'static str> {
    let [product, os, arch] = id.split('-').collect::<Vec<_>>()[..] else {
        bail!("target id {id} is not <product>-<os>-<arch>");
    };
    if !matches!(arch, "x86_64" | "aarch64") {
        bail!("target id {id} has unknown arch {arch}");
    }
    Ok(match (product, os) {
        ("cli", "linux" | "macos" | "windows") => "binary",
        ("app", "linux") => "appimage",
        ("app", "macos") => "app",
        ("app", "windows") => "nsis",
        _ => bail!("target id {id} has an unknown product or os"),
    })
}

/// Reads the notes and map files, then writes `dir/latest.json`.
pub fn run(
    version: &str,
    base_url: &str,
    notes: &Path,
    map: &Path,
    pub_date: Option<&str>,
    dir: &Path,
) -> anyhow::Result<ExitCode> {
    let notes = std::fs::read_to_string(notes)
        .with_context(|| format!("reading notes {}", notes.display()))?;
    let map_text =
        std::fs::read_to_string(map).with_context(|| format!("reading map {}", map.display()))?;
    let json = build(version, base_url, &notes, &map_text, pub_date, dir)?;
    let out = dir.join("latest.json");
    std::fs::write(&out, json).with_context(|| format!("writing {}", out.display()))?;
    println!("wrote {}", out.display());
    Ok(ExitCode::SUCCESS)
}
