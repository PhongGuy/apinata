//! The token panel's view of an auth: status, Get token, Refresh and Clear for the auth a
//! request, a folder or the collection uses.

use super::OAuth2Error;
use super::cache::Entry;
use super::config::TokenConfig;
use crate::context::VarContext;
use crate::document::Document;
use crate::item_path::ItemPath;
use crate::request::RequestFile;
use crate::resolve::effective_oauth2;
use std::ops::Deref;
use std::time::SystemTime;
use yaml_serde::{Mapping, Value};

/// Whose auth a token action applies to.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum OAuth2Target<'a> {
    /// A request's effective auth, inherited or its own.
    Request(&'a RequestFile),
    /// A folder's effective auth.
    Folder(&'a ItemPath),
    /// The collection's auth.
    Collection,
}

/// What is known about a cached token, never its value.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TokenStatus {
    /// When the token was obtained.
    pub obtained_at: SystemTime,
    /// When it stops being used, already brought forward by the expiry margin.
    pub expires_at: Option<SystemTime>,
    /// Whether a refresh token was issued with it.
    pub has_refresh_token: bool,
    /// The scope the provider granted, when it said.
    pub scope: Option<String>,
}

impl TokenStatus {
    /// A status from its parts.
    pub fn new(
        obtained_at: SystemTime,
        expires_at: Option<SystemTime>,
        has_refresh_token: bool,
        scope: Option<String>,
    ) -> TokenStatus {
        TokenStatus {
            obtained_at,
            expires_at,
            has_refresh_token,
            scope,
        }
    }

    /// A clock earlier than `obtained_at` also counts as expired.
    pub fn is_expired(&self, now: SystemTime) -> bool {
        now < self.obtained_at || self.expires_at.is_some_and(|end| now >= end)
    }

    pub(crate) fn of(entry: &Entry) -> TokenStatus {
        TokenStatus::new(
            entry.obtained_at(),
            entry.expires_at(),
            entry.field("refresh_token").is_some_and(|t| !t.is_empty()),
            entry.field("scope").map(str::to_owned),
        )
    }
}

/// The request whose effective auth is the target's: the request itself, or an unsaved probe
/// placed in the folder or at the root so it inherits that auth and its variables.
pub(crate) enum Probe<'a> {
    Borrowed(&'a RequestFile),
    Owned(Box<RequestFile>),
}

impl Deref for Probe<'_> {
    type Target = RequestFile;
    fn deref(&self) -> &RequestFile {
        match self {
            Probe::Borrowed(req) => req,
            Probe::Owned(req) => req,
        }
    }
}

impl<'a> Probe<'a> {
    pub(crate) fn new(target: OAuth2Target<'a>, ctx: &VarContext) -> Probe<'a> {
        let dir = match target {
            OAuth2Target::Request(req) => return Probe::Borrowed(req),
            OAuth2Target::Folder(item) => item.to_path(ctx.root()),
            OAuth2Target::Collection => ctx.root().to_owned(),
        };
        let text = |s: &str| Value::String(s.to_owned());
        let http: Mapping = [("method", "GET"), ("url", ""), ("auth", "inherit")]
            .into_iter()
            .map(|(k, v)| (text(k), text(v)))
            .collect();
        let mapping = Mapping::from_iter([(text("http"), Value::Mapping(http))]);
        let doc = Document::draft(dir.join(".oauth2-target.yml"), mapping);
        Probe::Owned(Box::new(RequestFile::from_document(doc)))
    }

    /// The token settings of the probe's auth; `NotOAuth2` for any other auth.
    pub(crate) fn config(&self, ctx: &VarContext) -> Result<TokenConfig, OAuth2Error> {
        let spec = effective_oauth2(self, ctx).ok_or(OAuth2Error::NotOAuth2)?;
        TokenConfig::new(&spec, self, ctx)
    }
}

/// Whether the target's effective auth is OAuth 2.0.
pub fn is_oauth2(target: OAuth2Target<'_>, ctx: &VarContext) -> bool {
    effective_oauth2(&Probe::new(target, ctx), ctx).is_some()
}

/// Whether naming the undefined or cyclic variable `name` for the target's auth could reveal
/// a sensitive value; the request rule applied to the target's probe.
pub fn oauth2_name_hidden(target: OAuth2Target<'_>, ctx: &VarContext, name: &str) -> bool {
    ctx.name_inside_sensitive_value(&Probe::new(target, ctx), name)
}

/// The loaded cache's entry for the target's auth; `Ok(None)` when none or not oauth2.
pub fn oauth2_status(
    target: OAuth2Target<'_>,
    ctx: &VarContext,
) -> Result<Option<TokenStatus>, OAuth2Error> {
    let cfg = match Probe::new(target, ctx).config(ctx) {
        Err(OAuth2Error::NotOAuth2) => return Ok(None),
        cfg => cfg?,
    };
    let entry = ctx
        .oauth2_tokens()
        .lookup(&cfg.key(ctx), &cfg.fingerprint());
    Ok(entry.as_ref().map(TokenStatus::of))
}

/// Removes the target's credentials id entries (all token URLs); the count removed.
pub async fn clear_oauth2_token(
    target: OAuth2Target<'_>,
    ctx: &VarContext,
) -> Result<usize, OAuth2Error> {
    let cfg = Probe::new(target, ctx).config(ctx)?;
    let cache = ctx.oauth2_tokens();
    cache.load().await;
    Ok(cache
        .clear(&ctx.key_root(), ctx.environment(), &cfg.credentials_id)
        .await)
}
