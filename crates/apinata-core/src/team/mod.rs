//! Team mode: a collection kept in sync with a team server by committing, pushing and pulling
//! on its own. This module holds the git env that carries the server token, the reading of git's
//! output as a sync failure, the state machine that decides what the loop runs next, and the
//! driver thread that runs it.

mod auth;
mod driver;
mod failure;
mod machine;

pub use auth::{Identity, TeamAuth, redact_token, team_options};
pub use driver::{TeamConfig, TeamStatus, TeamSync};
pub use failure::{Finding, SyncFailure, classify};
pub use machine::{Block, Role, TeamAction, TeamEvent, TeamMachine, TeamState};
