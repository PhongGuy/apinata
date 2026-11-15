//! Signing in to a team server, reading the current session and signing out. The token goes to
//! the keychain and the account to the team file; no flow holds the file across an `.await`.

use std::path::Path;

use super::{Account, Me, TeamApi, TeamError, TeamStore, TeamToken, token_key};
use crate::{SecretError, SecretStore};

pub(super) fn secrets_error(e: SecretError) -> TeamError {
    TeamError::Secrets(e.to_string())
}

/// The token kept for `account`. A value that is not a token reads as none.
pub(super) fn kept_token(
    api: &dyn TeamApi,
    secrets: &dyn SecretStore,
    account: &Account,
) -> Result<Option<TeamToken>, SecretError> {
    let text = secrets.get(&token_key(api.server(), &account.username))?;
    Ok(text.and_then(|t| TeamToken::parse(&t).ok()))
}

/// Signs `username` in with a password and keeps the session, replacing any earlier account
/// on this server. The earlier token is revoked only once the new one is kept, so a failure
/// at any step leaves the working session as it was.
pub async fn sign_in(
    api: &dyn TeamApi,
    secrets: &dyn SecretStore,
    team_file: &Path,
    username: &str,
    password: &str,
) -> Result<Account, TeamError> {
    if !api.server_info().await?.password_sign_in {
        return Err(TeamError::PasswordSignInOff);
    }
    let signed = api.sign_in(username, password).await?;
    let server = api.server();
    let previous = TeamStore::load(team_file)?.account(server).cloned();
    // A keychain error here only means the old token cannot be revoked; sign-in goes on.
    let previous_token = previous
        .as_ref()
        .and_then(|a| kept_token(api, secrets, a).ok().flatten());

    let account = Account::new(
        server,
        &signed.user.username,
        &signed.user.display_name,
        &signed.token_id,
        signed.user.server_admin,
    );
    let key = token_key(server, &account.username);
    if let Err(e) = secrets.set(&key, signed.token.expose()) {
        let _ = api.revoke(&signed.token, &signed.token_id).await;
        return Err(secrets_error(e));
    }
    if let Err(e) = TeamStore::update(team_file, |t| t.put_account(account.clone())) {
        // The file still names the old session, so the key must hold its token again (same
        // user) or nothing (a new user), never the token about to be revoked.
        let same_user = previous
            .as_ref()
            .is_some_and(|a| a.username == account.username);
        let _ = match previous_token.as_ref().filter(|_| same_user) {
            Some(old) => secrets.set(&key, old.expose()),
            None => secrets.delete(&key),
        };
        let _ = api.revoke(&signed.token, &signed.token_id).await;
        return Err(e);
    }

    // The new session is kept; retire the old one as far as the server and keychain allow.
    if let Some(old) = previous {
        if let Some(token) = previous_token {
            let _ = api.revoke(&token, &old.token_id).await;
        }
        if old.username != account.username {
            let _ = secrets.delete(&token_key(server, &old.username));
        }
    }
    Ok(account)
}

/// Revokes this server's token and forgets it and the account; bindings are kept. A revoke
/// the server already did (`Unauthenticated`, `NotFound`) still cleans up; any other failure
/// changes nothing, so sign-out can be retried.
pub async fn sign_out(
    api: &dyn TeamApi,
    secrets: &dyn SecretStore,
    team_file: &Path,
) -> Result<(), TeamError> {
    let Some(account) = TeamStore::load(team_file)?.account(api.server()).cloned() else {
        return Ok(());
    };
    if let Some(token) = kept_token(api, secrets, &account).map_err(secrets_error)? {
        match api.revoke(&token, &account.token_id).await {
            Ok(()) | Err(TeamError::Unauthenticated | TeamError::NotFound(_)) => {}
            Err(e) => return Err(e),
        }
    }
    secrets
        .delete(&token_key(api.server(), &account.username))
        .map_err(secrets_error)?;
    TeamStore::update(team_file, |t| t.remove_account(api.server()))
}

/// The signed-in user on this server, refreshing the account's name, admin flag and identity. Without an
/// account or token it is `Unauthenticated` with no request; a refused token is deleted while
/// the account stays, so the list can show the server as signed out.
pub async fn current(
    api: &dyn TeamApi,
    secrets: &dyn SecretStore,
    team_file: &Path,
) -> Result<Me, TeamError> {
    let Some(account) = TeamStore::load(team_file)?.account(api.server()).cloned() else {
        return Err(TeamError::Unauthenticated);
    };
    let Some(token) = kept_token(api, secrets, &account).map_err(secrets_error)? else {
        return Err(TeamError::Unauthenticated);
    };
    let me = match api.me(&token).await {
        Err(TeamError::Unauthenticated) => {
            let _ = secrets.delete(&token_key(api.server(), &account.username));
            return Err(TeamError::Unauthenticated);
        }
        other => other?,
    };
    TeamStore::update(team_file, |t| {
        // Only the account this call read; a sign-in meanwhile may have replaced it.
        if let Some(found) = t
            .account(api.server())
            .filter(|a| a.username == account.username)
            .cloned()
        {
            t.put_account(Account {
                display_name: me.user.display_name.clone(),
                server_admin: me.user.server_admin,
                identity: Some(me.identity.clone()),
                ..found
            });
        }
    })?;
    Ok(me)
}
