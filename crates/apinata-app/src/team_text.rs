//! What the team dialogs say when a server call fails, when an address is unsafe, and what
//! stands in the way of a team collection's sync.

use apinata_core::team::{Block, Finding, ServerUrl, TeamError, TeamState, TeamStatus};

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

/// One secret the server refused, as `{path}: {rule} ({code}) on {git_ref}`, naming the code
/// only when it differs from the rule.
///
/// The server reports a finding outside any file (a commit or tag message, or a
/// tag's target) with the path `/`, which no repository file can have. A word joiner follows
/// each slash of the ref, so a wrapped line breaks before the ref rather than inside it.
pub fn finding_line(f: &Finding) -> String {
    let place = if f.path == "/" {
        "not in a file (a commit or tag message, or a tag's target)"
    } else {
        &f.path
    };
    let git_ref = f.git_ref.replace('/', "/\u{2060}");
    if f.code == f.rule {
        format!("{place}: {} on {git_ref}", f.rule)
    } else {
        format!("{place}: {} ({}) on {git_ref}", f.rule, f.code)
    }
}

/// What the sync dialog says about `status` with `clashes` unsettled, one line each. Offline,
/// read-only and blocked folders end with how many local commits wait for the server.
pub fn problem_lines(status: &TeamStatus, clashes: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let waits = match &status.state {
        // Clashes listed below are not in sync, even when the loop itself is idle.
        TeamState::Idle => {
            lines.push(match clashes {
                0 => "In sync.".into(),
                1 => "1 clash to settle.".into(),
                n => format!("{n} clashes to settle."),
            });
            false
        }
        TeamState::Offline => {
            lines.push(
                "The server cannot be reached. Changes are committed here and pushed once it is back."
                    .into(),
            );
            true
        }
        TeamState::ReadOnly { paused } => {
            lines.push("You can only read this workspace, so changes are not saved.".into());
            if *paused {
                lines.push(
                    "Files changed here outside the app hold back updates. Discard them to keep pulling."
                        .into(),
                );
            }
            true
        }
        TeamState::Blocked(block) => {
            block_lines(block, &mut lines);
            true
        }
        _ => {
            lines.push("Syncing.".into());
            false
        }
    };
    if waits && status.unpushed > 0 {
        lines.push(match status.unpushed {
            1 => "1 local commit is not on the server yet.".into(),
            n => format!("{n} local commits are not on the server yet."),
        });
    }
    lines
}

fn block_lines(block: &Block, lines: &mut Vec<String>) {
    match block {
        Block::Secret { findings, more } => {
            lines.push("The server refused the push because it found secrets:".into());
            lines.extend(findings.iter().map(finding_line));
            if *more > 0 {
                lines.push(format!("and {more} more"));
            }
            lines.push("Remove them, save, then Retry.".into());
        }
        Block::SignedOut => lines.push(
            "You are signed out of this team server. Changes are committed here until you sign in again."
                .into(),
        ),
        Block::Gone => lines.push("This workspace is no longer on the server.".into()),
        Block::Hook { output } => {
            lines.push("A commit hook refused the commit:".into());
            lines.push(output.trim().into());
        }
        Block::Failed { message } => {
            lines.push("Sync failed:".into());
            lines.push(message.trim().into());
        }
        _ => lines.push("Sync stopped.".into()),
    }
}

/// How a team collection's sync stands, for the status bar's tone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncKind {
    /// Up to date with the server.
    Synced,
    /// Pulling, committing or pushing.
    Syncing,
    /// The server cannot be reached; commits wait locally.
    Offline,
    /// Something needs the user before sync goes on.
    Problem,
}

/// The status-bar kind and text for a team collection. A block wins, then a paused
/// read-only folder, then unsettled clashes, then the loop's state.
pub fn sync_status(status: &TeamStatus, read_only: bool, clashes: usize) -> (SyncKind, String) {
    let problem = |text: &str| (SyncKind::Problem, format!("problem: {text}"));
    match &status.state {
        TeamState::Blocked(block) => problem(match block {
            Block::Secret { .. } => "secrets found",
            Block::SignedOut => "signed out",
            Block::Gone => "workspace gone",
            Block::Hook { .. } => "commit hook refused",
            Block::Failed { .. } => "sync failed",
            _ => "sync stopped",
        }),
        TeamState::ReadOnly { paused: true } => problem("outside edits"),
        _ if clashes == 1 => problem("1 clash to settle"),
        _ if clashes > 1 => problem(&format!("{clashes} clashes to settle")),
        TeamState::Offline if status.unpushed > 0 => (
            SyncKind::Offline,
            format!("offline, {} unpushed", status.unpushed),
        ),
        TeamState::Offline => (SyncKind::Offline, "offline".into()),
        TeamState::Idle | TeamState::ReadOnly { .. } if read_only => {
            (SyncKind::Synced, "synced, read only".into())
        }
        TeamState::Idle | TeamState::ReadOnly { .. } => (SyncKind::Synced, "synced".into()),
        _ => (SyncKind::Syncing, "syncing".into()),
    }
}

/// The toast that announces `block` on the collection called `name`.
pub fn problem_toast(name: &str, block: &Block) -> String {
    let reason = match block {
        Block::Secret { .. } => "the server found secrets",
        Block::SignedOut => "you are signed out",
        Block::Gone => "the workspace is gone from the server",
        Block::Hook { .. } => "a commit hook refused the commit",
        Block::Failed { .. } => "git failed",
        _ => "it needs you",
    };
    format!("Sync of {name} stopped: {reason}.")
}

/// The toast for the first refused edit in the read-only collection called `name`.
pub fn read_only_toast(name: &str) -> String {
    format!("You can only read {name}, so your changes are not saved.")
}
