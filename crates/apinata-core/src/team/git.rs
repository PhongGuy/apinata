//! Turning a server workspace into a clone request that only ever talks to that server.

use std::path::Path;

use super::{ServerUrl, TeamError, Workspace};
use crate::CloneRequest;

/// A request to clone `ws` into `parent/<slug>`. The workspace's git address must be exactly
/// `{server}/git/{team}/{slug}.git`, since the token travels to that address; any other address
/// is `TeamError::Protocol`.
pub fn clone_request(
    server: &ServerUrl,
    ws: &Workspace,
    parent: &Path,
) -> Result<CloneRequest, TeamError> {
    let expected = format!("{}/git/{}/{}.git", server.as_str(), ws.team, ws.slug);
    if ws.git_url != expected {
        return Err(TeamError::Protocol(
            "the workspace's git address is not on this server".into(),
        ));
    }
    Ok(CloneRequest::new(expected, parent, ws.slug.clone()))
}
