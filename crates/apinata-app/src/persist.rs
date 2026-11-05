//! Reading and atomically writing the app's JSON files.

use apinata_core::{ApprovalInbox, ScriptTrust, TokenCache, TrustStore};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Where the settings, keymap, session, trust file, approval inbox, update state, kept responses, schemas and cookie jar live.
#[derive(Debug, Clone, PartialEq)]
pub struct Paths {
    /// The user's settings file.
    pub settings: PathBuf,
    /// The user's shortcut overrides, beside the settings.
    pub keymap: PathBuf,
    /// The window and collection session file.
    pub session: PathBuf,
    /// The per-collection trusted and denied hosts, shared with the CLI and MCP server.
    pub trusted_hosts: PathBuf,
    /// Each collection's script decision and approved script texts, shared with the CLI and MCP server.
    pub script_trust: PathBuf,
    /// Each collection's last acknowledged network settings; the app's alone.
    pub network_ack: PathBuf,
    /// The directory where the MCP server files approval requests.
    pub approvals: PathBuf,
    /// When updates were last checked and which version the user skipped.
    pub update_state: PathBuf,
    /// The sealed cookie jar; the app's alone.
    pub cookies: PathBuf,
    /// The directory of kept last responses.
    pub responses: PathBuf,
    /// The directory of untitled drafts kept for after a restart.
    pub drafts: PathBuf,
    /// The hidden collection that holds drafts written before any collection is open.
    pub scratch: PathBuf,
    /// The encrypted OAuth 2.0 token cache, shared with the CLI and MCP server.
    pub oauth2_tokens: PathBuf,
    /// The global environments every collection can select, shared with the CLI and MCP server.
    pub global_environments: PathBuf,
    /// The directory of user theme files, one theme set per `*.json`.
    pub themes: PathBuf,
    /// The directory of GraphQL schemas kept between runs; the app's alone.
    pub schemas: PathBuf,
}

impl Paths {
    /// The platform locations: settings and keymap under the config directory, the session and update
    /// state under the local data directory, and core's own trust file and inbox locations. `None` when
    /// the platform has no such directories.
    pub fn platform() -> Option<Paths> {
        Some(Paths {
            settings: dirs::config_dir()?.join("apinata/settings.json"),
            keymap: dirs::config_dir()?.join("apinata/keymap.json"),
            session: dirs::data_local_dir()?.join("apinata/session.json"),
            trusted_hosts: TrustStore::default_path()?,
            script_trust: ScriptTrust::default_path()?,
            network_ack: dirs::config_dir()?.join("apinata/network-ack.json"),
            approvals: ApprovalInbox::default_dir()?,
            update_state: dirs::data_local_dir()?.join("apinata/update-state.json"),
            cookies: dirs::data_local_dir()?.join("apinata/cookies.json"),
            responses: dirs::data_local_dir()?.join("apinata/responses"),
            drafts: dirs::data_local_dir()?.join("apinata/drafts"),
            scratch: dirs::data_local_dir()?.join("apinata/scratch"),
            oauth2_tokens: TokenCache::default_path()?,
            global_environments: dirs::config_dir()?.join("apinata/global-environments"),
            themes: dirs::config_dir()?.join("apinata/themes"),
            schemas: dirs::data_local_dir()?.join("apinata/graphql-schemas"),
        })
    }

    /// `Paths::in_dir(dir)` when `profile` is a non-empty value, else `Paths::platform()`.
    pub fn for_profile(profile: Option<OsString>) -> Option<Paths> {
        match profile.filter(|p| !p.is_empty()) {
            Some(dir) => Some(Paths::in_dir(Path::new(&dir))),
            None => Paths::platform(),
        }
    }

    /// Every file and the inbox directly inside `dir`.
    pub fn in_dir(dir: &Path) -> Paths {
        Paths {
            settings: dir.join("settings.json"),
            keymap: dir.join("keymap.json"),
            session: dir.join("session.json"),
            trusted_hosts: dir.join("trusted-hosts.json"),
            script_trust: dir.join("script-trust.json"),
            network_ack: dir.join("network-ack.json"),
            approvals: dir.join("approvals"),
            update_state: dir.join("update-state.json"),
            cookies: dir.join("cookies.json"),
            responses: dir.join("responses"),
            drafts: dir.join("drafts"),
            scratch: dir.join("scratch"),
            oauth2_tokens: dir.join("oauth2-tokens.json"),
            global_environments: dir.join("global-environments"),
            themes: dir.join("themes"),
            schemas: dir.join("graphql-schemas"),
        }
    }
}

/// A loaded value and, when the file could not be used, a message for the user.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded<T> {
    /// The file's value, or the defaults.
    pub value: T,
    /// Set when defaults replaced an unreadable file; names the file, never its content.
    pub recovered: Option<String>,
}

/// Reads `path` as JSON. A missing file gives defaults silently; an unparsable one is
/// moved to `<name>.bad` and gives defaults with a message; another read error gives
/// defaults with a message and leaves the file alone.
pub fn load_json<T: DeserializeOwned + Default>(path: &Path) -> Loaded<T> {
    let name = file_name(path);
    let defaults = |recovered| Loaded {
        value: T::default(),
        recovered,
    };
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return defaults(None),
        Err(_) => {
            return defaults(Some(format!(
                "{name} could not be read; defaults are in use"
            )));
        }
    };
    match serde_json::from_str(&text) {
        Ok(value) => Loaded {
            value,
            recovered: None,
        },
        Err(_) => {
            // Set the corrupt file aside so the next save does not destroy what the
            // user may want to recover; an older `.bad` is replaced.
            let bad = path.with_file_name(format!("{name}.bad"));
            let message = match std::fs::rename(path, &bad) {
                Ok(()) => format!(
                    "{name} could not be read and was moved to {name}.bad; defaults are in use"
                ),
                Err(_) => format!("{name} could not be read; defaults are in use"),
            };
            defaults(Some(message))
        }
    }
}

/// Writes `value` as pretty JSON to `path`, creating the parent directory. The write
/// goes to a temporary sibling that is synced and renamed over `path`, so a reader
/// sees either the old file or the new one.
pub fn save_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// Writes `bytes` to `path` as `save_json` does: parent created, temporary sibling synced and
/// renamed over `path`.
pub fn save_bytes(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}
