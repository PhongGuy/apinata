//! What the team dialogs say when a server call fails, and when an address is unsafe.

use apinata_core::team::{ServerUrl, TeamError};

/// The user text for `e` anywhere outside sign-in.
pub fn error_text(e: &TeamError) -> String {
    match e {
        TeamError::BadServerUrl(message) => message.clone(),
        TeamError::PasswordSignInOff => "This server does not offer password sign-in.".into(),
        TeamError::Unauthenticated => "You are signed out of this server. Sign in again.".into(),
        TeamError::Forbidden(_) => "Your role does not allow that.".into(),
        TeamError::NotFound(_) => "The server has no such workspace, or you cannot see it.".into(),
        TeamError::Conflict(_) => "A workspace with that name already exists in this team.".into(),
        TeamError::Invalid {
            field: Some(field),
            message,
        } => format!("{field}: {message}"),
        TeamError::Invalid {
            field: None,
            message,
        } => message.clone(),
        TeamError::RateLimited {
            retry_after_secs: Some(secs),
        } => {
            format!("Too many attempts. Try again in {secs} seconds.")
        }
        TeamError::RateLimited {
            retry_after_secs: None,
        } => "Too many attempts. Try again shortly.".into(),
        TeamError::Unavailable => "The server is unavailable right now. Try again later.".into(),
        TeamError::Server { status, .. } => {
            format!("The server answered with an error (HTTP {status}).")
        }
        TeamError::Offline(_) => {
            "Could not reach the server. Check the address and your connection.".into()
        }
        TeamError::Timeout => "The server took too long to answer.".into(),
        TeamError::Protocol(_) => {
            "The server's answer was not understood. Is this a Potluck server?".into()
        }
        TeamError::Secrets(message) => {
            format!("The sign-in could not be saved in the system keychain: {message}")
        }
        TeamError::Store(message) => format!("The team server list could not be saved: {message}"),
        _ => "Something went wrong talking to the server.".into(),
    }
}

/// The user text for `e` at sign-in, where a refusal means the password was wrong.
pub fn sign_in_error_text(e: &TeamError) -> String {
    match e {
        TeamError::Unauthenticated => "Sign-in failed. Check the username and password.".into(),
        other => error_text(other),
    }
}

/// A warning when `server` would carry the password and token over plain http to another machine.
pub fn server_warning(server: &ServerUrl) -> Option<&'static str> {
    server.is_unencrypted().then_some(
        "This address is not encrypted, so your password and token cross the network in clear.",
    )
}
