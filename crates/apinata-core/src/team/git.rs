//! Turning a server workspace into a clone request that only ever talks to that server.

use std::path::Path;

use super::{ServerUrl, TeamError, Workspace, valid_name};
use crate::{CloneRequest, GitOptions};

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

/// Whether `root` is a clone of the workspace at `url`: git resolves `root` itself as its
/// repository's top level (canonical paths compared), and `origin`'s fetch address is exactly
/// `url`. Anything git cannot answer counts as not a clone.
pub fn is_clone_of(root: &Path, url: &str, opts: &GitOptions) -> bool {
    // An empty or broken `.git` would let git fall back to an outer repository's origin.
    crate::git::is_top_level(root, opts)
        && crate::git::origin_url(root, opts).is_some_and(|u| u == url)
}

/// The workspace's change stream, which must be exactly
/// `{server}/api/v1/workspaces/{team}/{slug}/events` since the token is sent there; any other
/// address is `TeamError::Protocol`.
pub fn events_url(server: &ServerUrl, ws: &Workspace) -> Result<String, TeamError> {
    let expected = workspace_events(server.as_str(), &ws.team, &ws.slug);
    if ws.events_url != expected {
        return Err(TeamError::Protocol(
            "the workspace's events address is not on this server".into(),
        ));
    }
    Ok(expected)
}

/// The one spelling of a workspace's events address on `server`.
pub(super) fn workspace_events(server: &str, team: &str, slug: &str) -> String {
    format!("{server}/api/v1/workspaces/{team}/{slug}/events")
}
