//! Team mode: a collection kept in sync with a team server by committing, pushing and pulling
//! on its own. This module holds the git env that carries the server token, the reading of git's
//! output as a sync failure, the state machine that decides what the loop runs next, the
//! driver thread that runs it, the folder that keeps conflict copies, the server's contract
//! types, the client for its JSON API, and the team file with the sign-in and sign-out flows.

mod auth;
mod client;
mod copies;
mod driver;
mod error;
mod events;
mod failure;
mod machine;
mod model;
mod session;
mod store;

pub use auth::{Identity, TeamAuth, redact_token, team_options};
pub use client::{HttpTeamClient, TOKEN_NAME, TeamApi, TeamFuture};
pub use copies::ConflictCopies;
pub use driver::{TeamConfig, TeamStatus, TeamSync};
pub use error::TeamError;
pub use events::{EventsConfig, EventsEnd, TeamNotice, follow_events, reconnect_delay};
pub use failure::{Finding, SyncFailure, classify};
pub use machine::{Block, Role, TeamAction, TeamEvent, TeamMachine, TeamState};
pub use model::{
    Me, NewWorkspace, ServerInfo, ServerUrl, SignIn, Team, TeamToken, User, Workspace,
};
pub use session::{current, sign_in, sign_out};
pub use store::{Account, Binding, TeamStore, token_key};

/// The workspace kind the server keeps for Apinata collections.
pub const APINATA_KIND: &str = "apinata";

/// Whether `name` is a server name label: `^[a-z0-9][a-z0-9-]{0,62}$`.
pub fn valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    let label = |b: &u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    matches!(bytes.first(), Some(b) if label(b))
        && bytes.len() <= 63
        && bytes.iter().all(|b| label(b) || *b == b'-')
}
