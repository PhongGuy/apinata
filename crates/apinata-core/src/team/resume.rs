//! Resuming a bound folder: what its sync loop starts with, read from the server when it
//! answers and from the kept account and binding when it does not.

use std::path::Path;

use super::git::workspace_events;
use super::session::account_token;
use super::{
    Account, Binding, Identity, Role, TeamApi, TeamAuth, TeamError, TeamStore, current, events_url,
};
use crate::SecretStore;

/// How the server stands with a bound folder at resume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Standing {
    /// The server listed the workspace for the signed-in user.
    Ready,
    /// No usable session for the folder's user on this server.
    SignedOut,
    /// The server no longer lists the workspace for this user.
    Gone,
    /// The server could not be reached; the kept token and role are used.
    Offline,
}

/// What a bound folder's sync loop starts with.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Resumed {
    /// How the server stands with the folder.
    pub standing: Standing,
    /// The user's role on the workspace.
    pub role: Role,
    /// The commit identity.
    pub identity: Identity,
    /// The server and token git signs in with; the token is empty unless Ready or Offline.
    pub auth: TeamAuth,
    /// The change stream to follow, when it is known and may receive the token.
    pub events_url: Option<String>,
    /// The branch the workspace syncs; empty when not known.
    pub branch: String,
}

/// Resumes the folder `binding` names. A session for another user, or none, is `SignedOut`
/// with no request. A server that cannot be reached is `Offline` on the kept token, the
/// cached identity and the binding's role; a server that answers refreshes the binding's role
/// and branch. An events address off this server is `TeamError::Protocol`.
pub async fn resume(
    api: &dyn TeamApi,
    secrets: &dyn SecretStore,
    team_file: &Path,
    binding: &Binding,
) -> Result<Resumed, TeamError> {
    let server = api.server();
    let account = TeamStore::load(team_file)?
        .account(server)
        .filter(|a| a.username == binding.username)
        .cloned();
    let kept = |standing, identity, token: String, events_url| Resumed {
        standing,
        role: binding.role,
        identity,
        auth: TeamAuth::new(server.as_str(), token),
        events_url,
        branch: binding.branch.clone(),
    };
    let Some(account) = account else {
        let identity = fallback_identity(binding, None);
        return Ok(kept(Standing::SignedOut, identity, String::new(), None));
    };
    let cached = account
        .identity
        .clone()
        .unwrap_or_else(|| fallback_identity(binding, Some(&account)));
    let token = account_token(api, secrets, &account)?;

    match current(api, secrets, team_file).await {
        // A sign-in as someone else landing mid-check makes `/me` speak for that user, whose
        // role and identity are not this folder's.
        Ok(me) if me.user.username != binding.username => {
            Ok(kept(Standing::SignedOut, cached, String::new(), None))
        }
        Ok(me) => {
            let listed = me
                .workspaces
                .iter()
                .find(|w| w.team == binding.team && w.slug == binding.workspace);
            let Some(ws) = listed else {
                return Ok(kept(Standing::Gone, me.identity, String::new(), None));
            };
            // No token was kept when the check began; a sign-in landing meanwhile is not used.
            let Some(token) = token else {
                return Ok(kept(Standing::SignedOut, me.identity, String::new(), None));
            };
            let events = events_url(server, ws)?;
            let (role, branch) = (ws.role, ws.default_branch.clone());
            TeamStore::update(team_file, |t| {
                if let Some(found) = t.binding_at(&binding.root).filter(|b| same(b, binding)) {
                    let refreshed = Binding {
                        role,
                        branch: branch.clone(),
                        ..found.clone()
                    };
                    t.put_binding(refreshed);
                }
            })?;
            Ok(Resumed {
                standing: Standing::Ready,
                role,
                identity: me.identity,
                auth: TeamAuth::new(server.as_str(), token.expose()),
                events_url: Some(events),
                branch,
            })
        }
        Err(TeamError::Unauthenticated) => {
            Ok(kept(Standing::SignedOut, cached, String::new(), None))
        }
        Err(
            TeamError::Offline(_)
            | TeamError::Timeout
            | TeamError::Unavailable(_)
            | TeamError::RateLimited { .. }
            | TeamError::Server { .. },
        ) => Ok(match token {
            Some(token) => {
                let events = (!binding.branch.is_empty())
                    .then(|| workspace_events(server.as_str(), &binding.team, &binding.workspace));
                kept(Standing::Offline, cached, token.expose().to_owned(), events)
            }
            None => kept(Standing::SignedOut, cached, String::new(), None),
        }),
        Err(e) => Err(e),
    }
}

/// Whether `found` is the same binding as `wanted`, not a later one for the same folder.
fn same(found: &Binding, wanted: &Binding) -> bool {
    (
        &found.server,
        &found.team,
        &found.workspace,
        &found.username,
    ) == (
        &wanted.server,
        &wanted.team,
        &wanted.workspace,
        &wanted.username,
    )
}

/// The identity used when the server never gave one: the display name (else the username) and
/// the server's noreply address for the username.
fn fallback_identity(binding: &Binding, account: Option<&Account>) -> Identity {
    let host = url::Url::parse(&binding.server)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default();
    let name = account.map_or(binding.username.as_str(), |a| a.display_name.as_str());
    Identity::new(name, format!("{}@users.noreply.{host}", binding.username))
}
