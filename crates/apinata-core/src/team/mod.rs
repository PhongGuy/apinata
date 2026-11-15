//! Team mode: a collection kept in sync with a team server by committing, pushing and pulling
//! on its own. This module holds the git env that carries the server token and the reading of
//! git's output as a sync failure, and the state machine that decides what the loop runs next.

mod auth;
mod failure;
mod machine;

pub use auth::{Identity, TeamAuth, redact_token, team_options};
pub use failure::{Finding, SyncFailure, classify};
pub use machine::{Block, Role, TeamAction, TeamEvent, TeamMachine, TeamState};
