//! `oauth2_token`: the token panel's status, Get token, Refresh and Clear, shown without values.

use std::time::{SystemTime, UNIX_EPOCH};

use apinata_core::{
    Collection, ItemPath, OAuth2Error, OAuth2Target, Persistence, RequestFile, TokenStatus,
    VarContext,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::send;

/// What `oauth2_token` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TokenAction {
    /// Report the cached token, if any.
    Status,
    /// Fetch a new token from the token endpoint.
    Fetch,
    /// Exchange the cached refresh token for a new token.
    Refresh,
    /// Forget the cached token.
    Clear,
}

/// `oauth2_token`'s arguments.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct OAuth2TokenArgs {
    /// A request path (`users/get.yml`), a folder path (`users`), or empty for the
    /// collection's own auth.
    pub path: String,
    pub action: TokenAction,
    /// The environment to resolve against; the server's default when absent.
    pub environment: Option<String>,
    /// Runtime variable overrides, applied over the environment.
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
}

/// A cached token, its values withheld; times in unix seconds.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct TokenDoc {
    pub obtained_at: u64,
    /// Absent when the token has no expiry.
    pub expires_at: Option<u64>,
    pub expired: bool,
    pub has_refresh_token: bool,
    /// The scope the provider granted, when it said.
    pub scope: Option<String>,
}

/// What `oauth2_token` returns.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct OAuth2TokenDoc {
    pub action: TokenAction,
    /// The cached token after the action; null when there is none.
    pub token: Option<TokenDoc>,
    /// How many cached tokens `clear` removed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub removed: Option<usize>,
    /// Why tokens are kept in memory only and do not outlast this session, when they are.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_only: Option<String>,
}

/// The item whose auth a token action applies to, owned so it outlives the collection lock.
pub(crate) enum Owned {
    Request(Box<RequestFile>),
    Folder(ItemPath),
    Collection,
}

impl Owned {
    /// The request, folder or (for an empty path) collection at `raw`.
    pub(crate) fn of(c: &Collection, raw: &str) -> Result<Owned, String> {
        let path = ItemPath::new(raw).ok_or_else(|| format!("`{raw}` is not an item path"))?;
        if path.is_root() {
            return Ok(Owned::Collection);
        }
        if let Some(req) = c.request(&path) {
            return Ok(Owned::Request(Box::new(req.clone())));
        }
        match c.folder(&path) {
            Some(_) => Ok(Owned::Folder(path)),
            None => Err(format!("no request or folder at `{raw}`")),
        }
    }

    pub(crate) fn target(&self) -> OAuth2Target<'_> {
        match self {
            Owned::Request(req) => OAuth2Target::Request(req),
            Owned::Folder(path) => OAuth2Target::Folder(path),
            Owned::Collection => OAuth2Target::Collection,
        }
    }

    pub(crate) fn file(&self) -> Option<&RequestFile> {
        match self {
            Owned::Request(req) => Some(req),
            _ => None,
        }
    }

    /// The name an approval request is filed under.
    pub(crate) fn label<'a>(&self, raw: &'a str) -> &'a str {
        match self {
            Owned::Collection => "opencollection.yml",
            _ => raw,
        }
    }
}

/// `err` as one line, withholding a variable name that could reveal a secret by the same
/// rule a send uses, applied to the target's auth.
pub(crate) fn message(err: &OAuth2Error, target: &Owned, ctx: &VarContext) -> String {
    match err {
        OAuth2Error::Resolve(e) => send::resolve_line(e, |name| {
            apinata_core::oauth2_name_hidden(target.target(), ctx, name)
        }),
        other => format!("{other}; nothing was sent"),
    }
}

/// The reply for `action`, built from the status alone so no value can reach it.
pub(crate) fn doc(
    action: TokenAction,
    status: Option<TokenStatus>,
    removed: Option<usize>,
    persistence: &Persistence,
) -> OAuth2TokenDoc {
    let secs = |at: SystemTime| at.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let token = status.map(|s| TokenDoc {
        obtained_at: secs(s.obtained_at),
        expires_at: s.expires_at.map(secs),
        expired: s.is_expired(SystemTime::now()),
        has_refresh_token: s.has_refresh_token,
        scope: s.scope,
    });
    let memory_only = match persistence {
        Persistence::MemoryOnly { reason } => Some(reason.clone()),
        _ => None,
    };
    OAuth2TokenDoc {
        action,
        token,
        removed,
        memory_only,
    }
}
