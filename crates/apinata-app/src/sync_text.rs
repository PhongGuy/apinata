//! What the branch picker says after a pull or a push: the command line's wording without its
//! command names, since here the user clicked a button.

use apinata_core::{Pulled, Pushed, SyncError};

/// Git texts that mean the credentials or the host key were refused.
const AUTH_MARKERS: [&str; 5] = [
    "Authentication failed",
    "could not read Username",
    "terminal prompts disabled",
    "Permission denied",
    "Host key verification failed",
];

/// Which way a sync went, for the wording of its errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// A pull.
    Pull,
    /// A push.
    Push,
}

/// The notification after a pull that finished.
pub fn pulled(p: &Pulled) -> String {
    let (b, u) = (&p.branch, &p.upstream);
    if p.from != p.to {
        return format!(
            "Fast-forwarded {b} to {u} ({}..{})",
            short(&p.from),
            short(&p.to)
        );
    }
    match p.ahead {
        0 => format!("{b} is up to date with {u}"),
        n => {
            let noun = if n > 1 { "commits" } else { "commit" };
            format!("{b} is up to date with {u} and {n} {noun} ahead of it")
        }
    }
}

/// The notification after a push that finished.
pub fn pushed(p: &Pushed) -> String {
    let (b, u) = (&p.branch, &p.upstream);
    let mut line = match &p.from {
        None => format!("Pushed {b} to {u} (new branch)"),
        Some(from) if *from == p.to => format!("{u} is up to date"),
        Some(from) => format!("Pushed {b} to {u} ({}..{})", short(from), short(&p.to)),
    };
    if p.upstream_set {
        line.push_str(&format!("; {b} now tracks {u}"));
    }
    line
}

/// The notice for a sync that failed, with what to do next where there is a next step.
pub fn failure(error: &SyncError, direction: Direction) -> String {
    let text = capitalized(&error.to_string());
    match error {
        SyncError::Diverged { .. } => {
            format!("{text}; merge or rebase in a terminal, then push.")
        }
        SyncError::Rejected { reason, .. }
            if reason == "non-fast-forward" || reason == "fetch first" =>
        {
            format!("{text}. Pull first, then push again.")
        }
        SyncError::Partial { .. } => {
            format!("{text}. Restore a clean tree in a terminal, then pull again.")
        }
        SyncError::Failed { stderr } if AUTH_MARKERS.iter().any(|m| stderr.contains(m)) => {
            format!("{text}\nSet up a credential helper or ssh-agent, then try again.")
        }
        SyncError::Cancelled if direction == Direction::Push => {
            "Cancelled; the push may already have reached the remote".to_owned()
        }
        _ => text,
    }
}

/// The notice for a push of `branch`, which has no upstream, when the repository has `remotes`
/// other than exactly one (one is offered as a publish instead).
pub fn no_upstream(branch: &str, remotes: &[String]) -> String {
    match remotes {
        [] => format!("{branch} has no upstream branch, and there is no remote to publish it to"),
        _ => format!(
            "{branch} has no upstream branch. Publish it to one of {} from a terminal.",
            remotes.join(", ")
        ),
    }
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn short(id: &str) -> &str {
    id.get(..7).unwrap_or(id)
}
