//! Team mode: a collection kept in sync with a team server by committing, pushing and pulling
//! on its own. This module holds the git env that carries the server token and the reading of
//! git's output as a sync failure.

mod auth;
mod failure;

pub use auth::{Identity, TeamAuth, redact_token, team_options};
pub use failure::{Finding, SyncFailure, classify};
