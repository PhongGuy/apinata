//! The update check: a build's configuration, the rules applied to a signed release
//! manifest, and the release a newer manifest offers. Compiled only with the `update` feature.

mod apply;
mod fetch;
mod install;
mod manifest;
mod sign;

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use url::{Host, Url};

pub use apply::{app_staging_dir, install_app, stage_app, start_installer};
pub use fetch::{check, download};
pub use install::{install_kind, install_standalone};
pub use manifest::evaluate;
pub use sign::verify_manifest;

/// Which program is asking about updates.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Product {
    /// The desktop app.
    App,
    /// The `apinata` command-line tool.
    Cli,
}

/// An operating system a release can target.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    /// Linux.
    Linux,
    /// macOS.
    Macos,
    /// Windows.
    Windows,
}

impl Os {
    /// The system this build runs on, or `None` for one no release targets.
    pub fn current() -> Option<Os> {
        match std::env::consts::OS {
            "linux" => Some(Os::Linux),
            "macos" => Some(Os::Macos),
            "windows" => Some(Os::Windows),
            _ => None,
        }
    }
}

/// How a release artifact is packaged.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// A Linux AppImage (`appimage`).
    AppImage,
    /// A macOS app bundle (`app`).
    App,
    /// A Windows NSIS installer (`nsis`).
    Nsis,
    /// A bare executable (`binary`).
    Binary,
}

/// Network time limits for the update check and the artifact download.
#[non_exhaustive]
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Establishing a connection.
    pub connect: Duration,
    /// Fetching the manifest and its signature, each.
    pub manifest: Duration,
    /// The whole artifact download.
    pub artifact_total: Duration,
    /// The longest pause between artifact bytes.
    pub artifact_idle: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            connect: Duration::from_secs(10),
            manifest: Duration::from_secs(30),
            artifact_total: Duration::from_secs(15 * 60),
            artifact_idle: Duration::from_secs(30),
        }
    }
}

/// Everything the check needs: where the manifest lives, which keys may sign it, and what
/// this build is.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct UpdateConfig {
    /// The URL of `latest.json`; its signature is at the same URL plus `.sig`.
    pub manifest_url: String,
    /// The page a user downloads releases from by hand.
    pub download_page: String,
    /// Trusted minisign public keys, one base64 `.pub` line each.
    pub keys: Vec<String>,
    /// This build's version.
    pub current_version: String,
    /// The manifest target this build installs from, as [`target_id`] gives it.
    pub target_id: String,
    /// Network time limits.
    pub limits: Limits,
}

/// A newer stable release, with the artifact for this build's target.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct Available {
    /// The release version.
    pub version: String,
    /// Plain-text release notes.
    pub notes: Option<String>,
    /// The publication time, as the manifest states it.
    pub pub_date: Option<String>,
    /// How the artifact is packaged.
    pub format: Format,
    /// Where the artifact downloads from.
    pub url: String,
    /// The artifact's SHA-256, 64 lowercase hex characters.
    pub sha256: String,
    /// The artifact's size in bytes.
    pub size: u64,
    pub(crate) limits: Limits,
}

/// A downloaded artifact whose size and checksum matched, ready to install.
#[non_exhaustive]
#[derive(Debug)]
pub struct Staged {
    /// The verified file, `apinata-update-<version>` in the staging directory.
    pub path: PathBuf,
    /// The release version it holds.
    pub version: String,
}

/// How the updated app starts once this process is done.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Relaunch {
    /// Start this AppImage or `.app` bundle after this process exits.
    Restart(PathBuf),
    /// Start this installer now, then exit; the installer restarts the app.
    Installer {
        /// The staged installer.
        program: PathBuf,
        /// Its command-line arguments.
        args: Vec<OsString>,
    },
}

/// Why an update check, download or install failed.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UpdateError {
    /// The build has no manifest URL, download page or trusted key.
    #[error("Update checks are not configured for this build")]
    Disabled,
    /// The server could not be reached.
    #[error("could not reach the update server: {0}")]
    Offline(String),
    /// The server took longer than its time limit.
    #[error("the update server timed out")]
    Timeout,
    /// The manifest request answered with a non-success status.
    #[error("the update server answered HTTP {0}")]
    Http(u16),
    /// The manifest, or a configured URL, breaks the manifest rules.
    #[error("the update manifest is invalid: {0}")]
    ManifestInvalid(String),
    /// The manifest signature is missing, malformed or does not verify.
    #[error("the update manifest signature is invalid: {0}")]
    ManifestSignature(String),
    /// The manifest is signed by none of the trusted keys.
    #[error("the update manifest is signed by an unknown key")]
    UnknownKey,
    /// The release has no artifact for this target id.
    #[error("the release has no download for {0}")]
    NoEntryForTarget(String),
    /// The artifact URL does not answer yet.
    #[error("the release download is not available yet")]
    ArtifactMissing,
    /// The artifact's length differs from the manifest's size.
    #[error("the download was {got} bytes, expected {expected}")]
    SizeMismatch {
        /// The size the manifest states.
        expected: u64,
        /// The bytes received.
        got: u64,
    },
    /// The artifact's digest differs from the manifest's checksum.
    #[error("the download does not match its checksum")]
    HashMismatch,
    /// The install location cannot be written.
    #[error("cannot write to {}", .0.display())]
    NotWritable(PathBuf),
    /// This copy belongs to a package manager or the app, so it does not replace itself.
    #[error("this copy cannot update itself: {0}")]
    NotOwned(Reason),
    /// A file operation failed.
    #[error("{}: {message}", path.display())]
    Io {
        /// The file involved.
        path: PathBuf,
        /// What went wrong.
        message: String,
    },
}

/// What kind of install a running executable belongs to, and so what may replace it.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallKind {
    /// A CLI binary the updater may replace in place.
    Standalone(PathBuf),
    /// The app running from this AppImage file.
    AppImage(PathBuf),
    /// The app running from this `.app` bundle directory.
    MacApp(PathBuf),
    /// The app installed by the NSIS installer.
    WindowsNsis,
    /// An install the updater must leave alone.
    NotOwned(Reason),
}

/// Why the updater leaves an install alone.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Reason {
    /// A system package manager installed it.
    #[error("it was installed by the system package manager")]
    SystemPackage,
    /// It is the CLI bundled with the app.
    #[error("it ships with the Apinata app, so update the app instead")]
    ShippedWithApp,
    /// It runs from a mounted disk image.
    #[error("it runs from a read-only disk image")]
    ReadOnlyVolume,
    /// macOS App Translocation runs it from a temporary copy.
    #[error("macOS runs it from a temporary location; move it to Applications first")]
    Translocated,
    /// No rule recognises its layout.
    #[error("the updater does not manage this install layout")]
    Unrecognised,
}

/// The manifest target id for `product` on this build: `<app|cli>-<os>-<arch>`.
pub fn target_id(product: Product) -> String {
    let product = match product {
        Product::App => "app",
        Product::Cli => "cli",
    };
    format!(
        "{product}-{}-{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

/// The configuration for `product` from explicit inputs. `keys_text` holds one key per line;
/// blank lines and `#` comments are ignored. A missing or empty URL, or no key, is
/// [`UpdateError::Disabled`]; a URL outside the URL rule is [`UpdateError::ManifestInvalid`].
pub fn build_config_with(
    product: Product,
    manifest_url: Option<&str>,
    download_page: Option<&str>,
    keys_text: &str,
    current_version: &str,
) -> Result<UpdateConfig, UpdateError> {
    let keys: Vec<String> = keys_text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect();
    let (Some(manifest_url), Some(download_page)) = (
        manifest_url.filter(|u| !u.is_empty()),
        download_page.filter(|p| !p.is_empty()),
    ) else {
        return Err(UpdateError::Disabled);
    };
    if keys.is_empty() {
        return Err(UpdateError::Disabled);
    }
    if !parse(manifest_url).is_some_and(|u| allowed(&u)) {
        return Err(UpdateError::ManifestInvalid(format!(
            "the manifest URL must be https: {manifest_url}"
        )));
    }
    // A loopback download page serves local runs only, so release builds insist on https.
    let page_ok = parse(download_page)
        .is_some_and(|u| u.scheme() == "https" || (cfg!(debug_assertions) && allowed(&u)));
    if !page_ok {
        return Err(UpdateError::ManifestInvalid(format!(
            "the download page must be https: {download_page}"
        )));
    }
    Ok(UpdateConfig {
        manifest_url: manifest_url.to_owned(),
        download_page: download_page.to_owned(),
        keys,
        current_version: current_version.to_owned(),
        target_id: target_id(product),
        limits: Limits::default(),
    })
}

/// The configuration this build was released with. Each URL comes from the run-time
/// environment first, then from the value compiled in; the keys from `assets/update/keys.txt`.
pub fn build_config(product: Product) -> Result<UpdateConfig, UpdateError> {
    let read = |runtime: Option<String>, compiled: Option<&str>| {
        runtime
            .filter(|v| !v.is_empty())
            .or(compiled.map(str::to_owned))
    };
    let manifest_url = read(
        std::env::var("APINATA_UPDATE_MANIFEST_URL").ok(),
        option_env!("APINATA_UPDATE_MANIFEST_URL"),
    );
    let download_page = read(
        std::env::var("APINATA_DOWNLOAD_PAGE_URL").ok(),
        option_env!("APINATA_DOWNLOAD_PAGE_URL"),
    );
    #[allow(unused_mut)]
    let mut keys = include_str!("../../../../assets/update/keys.txt").to_owned();
    // Tests sign with a throwaway key; release builds never read this variable.
    #[cfg(debug_assertions)]
    if let Ok(extra) = std::env::var("APINATA_UPDATE_TEST_KEYS") {
        keys.push('\n');
        keys.push_str(&extra);
    }
    build_config_with(
        product,
        manifest_url.as_deref(),
        download_page.as_deref(),
        &keys,
        env!("CARGO_PKG_VERSION"),
    )
}

fn parse(text: &str) -> Option<Url> {
    Url::parse(text).ok()
}

/// The URL rule for update traffic: `https`, or plain `http` to a loopback host.
pub(crate) fn allowed(url: &Url) -> bool {
    match url.scheme() {
        "https" => true,
        "http" => match url.host() {
            Some(Host::Domain(name)) => name == "localhost",
            Some(Host::Ipv4(ip)) => crate::is_loopback(ip.into()),
            Some(Host::Ipv6(ip)) => crate::is_loopback(ip.into()),
            None => false,
        },
        _ => false,
    }
}
