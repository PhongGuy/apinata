//! Turning a server workspace into a clone request that only ever talks to that server.

use std::path::Path;

use super::{ServerUrl, TeamError, Workspace, valid_name};
use crate::CloneRequest;

/// A request to clone `ws` into `parent/<slug>`. The workspace's git address must be exactly
/// `{server}/git/{team}/{slug}.git` and its events address the one `events_url` accepts, since
/// the token travels to both; any other address is `TeamError::Protocol`, as is a team or slug
/// that is not a name label, since both become folder names.
pub fn clone_request(
    server: &ServerUrl,
    ws: &Workspace,
    parent: &Path,
) -> Result<CloneRequest, TeamError> {
    if !valid_name(&ws.team) || !valid_name(&ws.slug) {
        return Err(TeamError::Protocol(
            "the workspace's name is not a valid folder name".into(),
        ));
    }
    let expected = format!("{}/git/{}/{}.git", server.as_str(), ws.team, ws.slug);
    if ws.git_url != expected {
        return Err(TeamError::Protocol(
            "the workspace's git address is not on this server".into(),
        ));
    }
    events_url(server, ws)?;
    Ok(CloneRequest::new(expected, parent, ws.slug.clone()))
}

/// The workspace's change stream, which must be exactly
/// `{server}/api/v1/workspaces/{team}/{slug}/events` since the token is sent there; any other
/// address is `TeamError::Protocol`.
pub fn events_url(server: &ServerUrl, ws: &Workspace) -> Result<String, TeamError> {
    let expected = format!(
        "{}/api/v1/workspaces/{}/{}/events",
        server.as_str(),
        ws.team,
        ws.slug
    );
    if ws.events_url != expected {
        return Err(TeamError::Protocol(
            "the workspace's events address is not on this server".into(),
        ));
    }
    Ok(expected)
}
