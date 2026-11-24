//! The team server's contract: its address, the session token and the records it returns.

use std::fmt;

use serde::Deserialize;
use unicode_properties::{GeneralCategory, UnicodeGeneralCategory};
use url::{Host, Url};

use super::{APINATA_KIND, Identity, Role, TeamError, valid_name};

/// A team server's base address: http or https, lowercase scheme and host, no trailing slash.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ServerUrl(String);

impl ServerUrl {
    /// Trims; adds `https://` when there is no `://`; requires http or https, a host, and no
    /// userinfo, query or fragment; lowercases scheme and host; drops a trailing `/`.
    pub fn parse(input: &str) -> Result<ServerUrl, TeamError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(TeamError::BadServerUrl("Enter the server address.".into()));
        }
        let bad =
            || TeamError::BadServerUrl("Use an address like https://team.example.com.".into());
        let full = if trimmed.contains("://") {
            trimmed.to_owned()
        } else {
            format!("https://{trimmed}")
        };
        // The url crate already lowercases the scheme and a domain host and drops default ports.
        let url = Url::parse(&full).map_err(|_| bad())?;
        let usable = matches!(url.scheme(), "http" | "https")
            && url.host().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none();
        if !usable {
            return Err(bad());
        }
        let text = url.as_str();
        Ok(ServerUrl(text.strip_suffix('/').unwrap_or(text).to_owned()))
    }

    /// The normalized address.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Plain http to a host that is not localhost, 127.0.0.0/8 or ::1.
    pub fn is_unencrypted(&self) -> bool {
        let Ok(url) = Url::parse(&self.0) else {
            return true;
        };
        let loopback = match url.host() {
            Some(Host::Domain(name)) => name == "localhost",
            Some(Host::Ipv4(ip)) => ip.is_loopback(),
            Some(Host::Ipv6(ip)) => ip.is_loopback(),
            None => false,
        };
        url.scheme() == "http" && !loopback
    }
}

impl fmt::Display for ServerUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A session token for one server. Its value shows only through `expose`, never in Debug.
#[derive(Clone)]
#[non_exhaustive]
pub struct TeamToken(String);

impl TeamToken {
    /// `plk_` plus exactly 43 base64url characters, else `Protocol`.
    pub fn parse(text: &str) -> Result<TeamToken, TeamError> {
        let ok = text.strip_prefix("plk_").is_some_and(|rest| {
            rest.len() == 43
                && rest
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        });
        if ok {
            Ok(TeamToken(text.to_owned()))
        } else {
            Err(TeamError::Protocol("the session token is malformed".into()))
        }
    }

    /// The token text, for the keychain and the Authorization header only.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for TeamToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TeamToken(plk_…)")
    }
}

/// A user account on the team server.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct User {
    /// The sign-in name.
    pub username: String,
    /// The name shown to people.
    pub display_name: String,
    /// The email, when the user set one.
    pub email: Option<String>,
    /// Whether the user administers the whole server.
    pub server_admin: bool,
    /// Whether the account is turned off.
    pub disabled: bool,
}

/// A team on the server.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct Team {
    /// The team's name in addresses.
    pub slug: String,
    /// The name shown to people.
    pub name: String,
    /// The usernames of its members.
    pub members: Vec<String>,
}

/// A workspace (one git repository) a team keeps on the server.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct Workspace {
    /// The owning team's slug.
    pub team: String,
    /// The workspace's name in addresses.
    pub slug: String,
    /// The name shown to people.
    pub name: String,
    /// What client the workspace is for; Apinata's is `APINATA_KIND`.
    pub kind: String,
    /// The branch the client syncs.
    pub default_branch: String,
    /// The signed-in user's role on it.
    pub role: Role,
    /// Where git clones and pushes it.
    pub git_url: String,
    /// The stream of change notices for it.
    pub events_url: String,
}

impl Workspace {
    /// `team/slug`.
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.team, self.slug)
    }
}

/// The signed-in user, the identity to commit under and the workspaces they can reach.
#[derive(Debug, Clone, Deserialize)]
#[serde(from = "MeWire")]
#[non_exhaustive]
pub struct Me {
    /// The user.
    pub user: User,
    /// The name and email team commits carry.
    pub identity: Identity,
    /// Every workspace the user can reach, of any kind.
    pub workspaces: Vec<Workspace>,
}

impl Me {
    /// The workspaces of kind `APINATA_KIND`, sorted by team then slug.
    pub fn apinata_workspaces(&self) -> Vec<&Workspace> {
        let mut found: Vec<&Workspace> = self
            .workspaces
            .iter()
            .filter(|w| w.kind == APINATA_KIND)
            .collect();
        found.sort_by(|a, b| (&a.team, &a.slug).cmp(&(&b.team, &b.slug)));
        found
    }
}

#[derive(Deserialize)]
struct MeWire {
    user: User,
    identity: IdentityWire,
    workspaces: Vec<Workspace>,
}

#[derive(Deserialize)]
struct IdentityWire {
    name: String,
    email: String,
}

impl From<MeWire> for Me {
    fn from(wire: MeWire) -> Me {
        Me {
            user: wire.user,
            identity: Identity::new(wire.identity.name, wire.identity.email),
            workspaces: wire.workspaces,
        }
    }
}

/// What the server says about itself before sign-in.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ServerInfo {
    /// The server's version.
    pub version: String,
    /// The API version it speaks.
    pub api: u32,
    /// Whether it accepts a username and password.
    pub password_sign_in: bool,
    /// The single sign-on provider it offers, if any.
    pub sso: Option<SsoProvider>,
}

/// A team server's single sign-on provider.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SsoProvider {
    /// What the sign-in button names, cut to 64 characters with control, format and line and
    /// paragraph separator characters removed.
    pub name: String,
}

impl SsoProvider {
    /// A provider named by the server's `name`, made safe to show.
    pub fn named(name: &str) -> SsoProvider {
        // Format characters (bidi controls among them) and line breaks could reorder or split
        // the button's text, so they go with the other controls.
        let hidden = |c: char| {
            matches!(
                c.general_category(),
                GeneralCategory::Control
                    | GeneralCategory::Format
                    | GeneralCategory::LineSeparator
                    | GeneralCategory::ParagraphSeparator
            )
        };
        let clean: String = name.chars().filter(|&c| !hidden(c)).collect();
        let name: String = clean.trim().chars().take(64).collect();
        let name = name.trim_end();
        SsoProvider {
            name: if name.is_empty() {
                "single sign-on".into()
            } else {
                name.into()
            },
        }
    }
}

/// A successful sign-in, by password or single sign-on.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SignIn {
    /// The new session token.
    pub token: TeamToken,
    /// The token's id, used to revoke it.
    pub token_id: String,
    /// The signed-in user.
    pub user: User,
    /// When the token stops working, RFC 3339; None when it does not expire.
    pub expires_at: Option<String>,
}

/// A request to create a workspace, its fields already checked.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct NewWorkspace {
    /// The owning team's slug.
    pub team: String,
    /// The new workspace's slug.
    pub slug: String,
    /// The name shown to people.
    pub name: String,
}

impl NewWorkspace {
    /// Trims all three. Err(Invalid { field: Some("team" | "slug" | "name"), message }).
    pub fn new(team: &str, slug: &str, name: &str) -> Result<NewWorkspace, TeamError> {
        let (team, slug, name) = (team.trim(), slug.trim(), name.trim());
        let invalid = |field: &str, message: &str| TeamError::Invalid {
            field: Some(field.into()),
            message: message.into(),
        };
        const LABEL: &str =
            "Use lowercase letters, digits and dashes, starting with a letter or digit.";
        if !valid_name(team) {
            return Err(invalid("team", LABEL));
        }
        if !valid_name(slug) {
            return Err(invalid("slug", LABEL));
        }
        if name.is_empty() {
            return Err(invalid("name", "Enter a name."));
        }
        Ok(NewWorkspace {
            team: team.into(),
            slug: slug.into(),
            name: name.into(),
        })
    }
}

/// What the server's last check for its own new release found, as a server admin reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ServerUpdate {
    /// The version the server runs.
    pub current: String,
    /// The newer release it found, if any.
    pub latest: Option<ServerRelease>,
    /// When it last checked, RFC 3339; None before the first check.
    pub checked_at: Option<String>,
    /// Whether its daily check is turned off.
    pub checks_off: bool,
}

/// A newer server release.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ServerRelease {
    /// Its version.
    pub version: String,
    /// Where its release notes are.
    pub notes_url: String,
}

/// The one line an admin sees about server updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum UpdateState {
    /// A newer release is out.
    Newer,
    /// The last check found nothing newer.
    UpToDate,
    /// Nothing checked yet and the daily check is off.
    Off,
    /// Nothing checked yet.
    NotChecked,
}

impl ServerUpdate {
    /// A found release wins, then a past check; "off" only shows while nothing was checked, so a
    /// manual check with the daily one off still reads as up to date.
    pub fn state(&self) -> UpdateState {
        match (&self.latest, &self.checked_at) {
            (Some(_), _) => UpdateState::Newer,
            (None, Some(_)) => UpdateState::UpToDate,
            (None, None) if self.checks_off => UpdateState::Off,
            (None, None) => UpdateState::NotChecked,
        }
    }
}
