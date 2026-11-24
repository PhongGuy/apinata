//! The team file: the accounts signed in to team servers and the folders bound to their
//! workspaces. It never holds a token; tokens live in the keychain under `token_key`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::{Identity, Role, ServerUrl, TeamError};
use crate::SecretKey;

/// Serializes every `TeamStore::update` in this process, so no two read-change-write cycles
/// interleave and drop each other's change.
static UPDATE_LOCK: Mutex<()> = Mutex::new(());

/// The keychain key for the token `username` holds on `server`: a per-install key (empty
/// collection) in the `team-server` environment, named `{server}#{username}`.
pub fn token_key(server: &ServerUrl, username: &str) -> SecretKey {
    SecretKey::new("", "team-server", format!("{server}#{username}"))
}

/// The one account signed in to a team server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Account {
    /// The server's normalized address.
    pub server: String,
    /// The sign-in name.
    pub username: String,
    /// The name shown to people.
    pub display_name: String,
    /// The id of the token kept in the keychain, used to revoke it.
    pub token_id: String,
    /// Whether the user administers the whole server.
    pub server_admin: bool,
    /// The commit identity the server gave last, used while it cannot be reached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<Identity>,
    /// When the kept token stops working, RFC 3339; None when it does not expire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

impl Account {
    /// An account for `username` on `server`.
    pub fn new(
        server: &ServerUrl,
        username: &str,
        display_name: &str,
        token_id: &str,
        server_admin: bool,
    ) -> Account {
        Account {
            server: server.as_str().to_owned(),
            username: username.to_owned(),
            display_name: display_name.to_owned(),
            token_id: token_id.to_owned(),
            server_admin,
            identity: None,
            expires_at: None,
        }
    }

    /// Whether the kept token's expiry has passed. An expiry that does not parse is left for
    /// the server to judge.
    pub fn expired(&self) -> bool {
        self.expires_at
            .as_deref()
            .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
            .is_some_and(|at| at <= chrono::Utc::now())
    }
}

/// A local folder kept in sync with one team workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Binding {
    /// The collection folder.
    pub root: PathBuf,
    /// The server's normalized address.
    pub server: String,
    /// The owning team's slug.
    pub team: String,
    /// The workspace's slug.
    pub workspace: String,
    /// Who opened it.
    pub username: String,
    /// The user's role on the workspace when it was opened.
    pub role: Role,
    /// The branch the workspace syncs, as the server last said; empty when not known.
    #[serde(default)]
    pub branch: String,
}

impl Binding {
    /// Binds `root` to `team/workspace` on `account`'s server.
    pub fn new(
        root: PathBuf,
        account: &Account,
        team: &str,
        workspace: &str,
        role: Role,
    ) -> Binding {
        Binding {
            root,
            server: account.server.clone(),
            team: team.to_owned(),
            workspace: workspace.to_owned(),
            username: account.username.clone(),
            role,
            branch: String::new(),
        }
    }
}

/// The team file's contents: `{"accounts": [...], "bindings": [...]}`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct TeamStore {
    accounts: Vec<Account>,
    bindings: Vec<Binding>,
}

impl TeamStore {
    /// Reads `path`; a missing file is an empty store, an unreadable file or bad JSON is `Store`.
    pub fn load(path: &Path) -> Result<TeamStore, TeamError> {
        let store_error =
            |e: &dyn std::fmt::Display| TeamError::Store(format!("{}: {e}", path.display()));
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| store_error(&e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(TeamStore::default()),
            Err(e) => Err(store_error(&e)),
        }
    }

    /// Under one process-wide lock: load `path`, run `change`, save atomically (creating the
    /// parent dir), return what `change` returned. The only way anything writes the team file.
    pub fn update<T>(
        path: &Path,
        change: impl FnOnce(&mut TeamStore) -> T,
    ) -> Result<T, TeamError> {
        // A panic in another update cannot leave the file half-written, so recover the lock.
        let _guard = UPDATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut store = TeamStore::load(path)?;
        let out = change(&mut store);
        let bytes = serde_json::to_vec_pretty(&store)
            .map_err(|e| TeamError::Store(format!("{}: {e}", path.display())))?;
        crate::trust::write_atomic(path, &bytes).map_err(|e| TeamError::Store(e.to_string()))?;
        Ok(out)
    }

    /// Every signed-in account.
    pub fn accounts(&self) -> &[Account] {
        &self.accounts
    }

    /// The account on `server`, if any.
    pub fn account(&self, server: &ServerUrl) -> Option<&Account> {
        self.accounts.iter().find(|a| a.server == server.as_str())
    }

    /// Adds `account` at the end, or replaces the one for the same server in its place.
    pub fn put_account(&mut self, account: Account) {
        match self
            .accounts
            .iter_mut()
            .find(|a| a.server == account.server)
        {
            Some(kept) => *kept = account,
            None => self.accounts.push(account),
        }
    }

    /// Forgets the account on `server`.
    pub fn remove_account(&mut self, server: &ServerUrl) {
        self.accounts.retain(|a| a.server != server.as_str());
    }

    /// Every bound folder.
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    /// The binding for the folder `root`, if any.
    pub fn binding_at(&self, root: &Path) -> Option<&Binding> {
        self.bindings.iter().find(|b| b.root == root)
    }

    /// The binding for `team/workspace` on `server`, if any.
    pub fn binding_for(&self, server: &ServerUrl, team: &str, workspace: &str) -> Option<&Binding> {
        self.bindings
            .iter()
            .find(|b| b.server == server.as_str() && b.team == team && b.workspace == workspace)
    }

    /// Adds `binding`, replacing the one for the same root.
    pub fn put_binding(&mut self, binding: Binding) {
        self.bindings.retain(|b| b.root != binding.root);
        self.bindings.push(binding);
    }

    /// Forgets the binding for the folder `root`.
    pub fn remove_binding(&mut self, root: &Path) {
        self.bindings.retain(|b| b.root != root);
    }
}
