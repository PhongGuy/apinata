//! OAuth 2.0 tokens: an encrypted on-disk cache shared by every shell.

mod authorize;
mod cache;
mod client;
pub(crate) mod config;
mod interactive;
mod listener;
mod prepare;
mod target;

use crate::error::ResolveError;
use crate::execute::ExecError;
use cache::CacheKey;
use std::collections::HashSet;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

pub use authorize::check_redirect;
pub use cache::TokenCache;
pub use target::{
    OAuth2Target, TokenStatus, clear_oauth2_token, is_oauth2, oauth2_name_hidden, oauth2_status,
};

/// How [`Executor::prepare_auth`](crate::Executor::prepare_auth) may obtain a token.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct AuthorizeOptions {
    /// With refetch on, the keys already fetched afresh; shared by clones.
    refetched: Option<Arc<Mutex<HashSet<CacheKey>>>>,
    cache_only: bool,
    authorizer: Option<Arc<dyn Authorizer>>,
}

/// The shell's part of a browser sign-in: show the authorization URL and, for the paste flow,
/// collect the redirect the user copies back.
pub trait Authorizer: Send + Sync {
    /// Open `prompt.url` in the system browser. With `prompt.wants_redirect`, return the URL
    /// the user pasted; otherwise return `Ok(None)` once opened, or stay pending until the
    /// sign-in ends (core drops the future then). `Err` means the user cancelled.
    fn authorize(
        &self,
        prompt: AuthorizationPrompt,
    ) -> Pin<Box<dyn Future<Output = Result<Option<String>, AuthorizeCancelled>> + Send>>;
}

/// What an [`Authorizer`] is asked to show.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AuthorizationPrompt {
    /// The authorization URL to open.
    pub url: String,
    /// Whether the user must paste the redirect back (no loopback listener).
    pub wants_redirect: bool,
    /// The callback URL the provider redirects to.
    pub callback: String,
}

impl AuthorizationPrompt {
    /// A prompt for `url` that redirects to `callback`.
    pub fn new(url: String, wants_redirect: bool, callback: String) -> AuthorizationPrompt {
        AuthorizationPrompt {
            url,
            wants_redirect,
            callback,
        }
    }
}

/// The user cancelled a sign-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorizeCancelled;

impl AuthorizeOptions {
    /// Use the cache, refresh and fetch as each auth's settings allow.
    pub fn new() -> AuthorizeOptions {
        AuthorizeOptions::default()
    }

    /// Ignore the cache once per key for this value and its clones; a key counts only once
    /// its fetch succeeds.
    pub fn refetch(mut self, on: bool) -> AuthorizeOptions {
        self.refetched = on.then(Arc::default);
        self
    }

    /// Never fetch or refresh: use whatever the cache holds, even an expired token.
    pub fn cache_only(mut self, on: bool) -> AuthorizeOptions {
        self.cache_only = on;
        self
    }

    /// Sign in through `authorizer` when a grant needs a browser.
    pub fn authorizer(mut self, authorizer: Arc<dyn Authorizer>) -> AuthorizeOptions {
        self.authorizer = Some(authorizer);
        self
    }

    /// Whether the cache must be ignored for `key`.
    fn must_refetch(&self, key: &CacheKey) -> bool {
        self.refetched
            .as_ref()
            .is_some_and(|keys| !keys.lock().unwrap_or_else(|e| e.into_inner()).contains(key))
    }

    /// Records that `key` was fetched afresh under these options.
    fn fetched(&self, key: CacheKey) {
        if let Some(keys) = &self.refetched {
            keys.lock().unwrap_or_else(|e| e.into_inner()).insert(key);
        }
    }
}

impl fmt::Debug for AuthorizeOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthorizeOptions")
            .field("refetch", &self.refetched.is_some())
            .field("cache_only", &self.cache_only)
            .field("authorizer", &self.authorizer.is_some())
            .finish()
    }
}

/// The label the editor shows for the OAuth 2.0 setting at file key `key`, so errors name a
/// setting the way the user sees it; an unknown key reads as a generic required setting.
pub fn oauth2_field_label(key: &str) -> &'static str {
    match key {
        "authorizationUrl" => "Authorization URL",
        "accessTokenUrl" => "Access token URL",
        "refreshTokenUrl" => "Refresh token URL",
        "callbackUrl" => "Callback URL",
        "clientId" => "Client ID",
        "clientSecret" => "Client secret",
        "username" => "Username",
        "password" => "Password",
        "scope" => "Scope",
        "state" => "State",
        "tokenConfig.id" => "Token ID",
        "tokenConfig.placement.header" => "Header prefix",
        "tokenConfig.placement.query" => "Query parameter name",
        _ => "A required OAuth 2.0 setting",
    }
}

/// Why an OAuth 2.0 token could not be obtained or placed. Text from the provider is already
/// scrubbed of secrets, so the derived `Debug` is safe.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum OAuth2Error {
    /// A setting the token request needs is empty.
    #[error("{} is empty", oauth2_field_label(field))]
    MissingField {
        /// The file key that is empty.
        field: &'static str,
    },
    /// A placeholder in the settings could not be expanded.
    #[error(transparent)]
    Resolve(ResolveError),
    /// The token request could not be sent, or its host is not allowed.
    #[error(transparent)]
    Exec(ExecError),
    /// The token endpoint refused the request.
    #[error("the token endpoint answered {status}: {error}{}", .description.as_ref().map(|d| format!(" ({d})")).unwrap_or_default())]
    Provider {
        /// The HTTP status.
        status: u16,
        /// The provider's `error` code.
        error: String,
        /// The provider's `error_description`, when given.
        description: Option<String>,
    },
    /// The token endpoint answered with something that is not a token.
    #[error("the token endpoint answered {status} with {reason}")]
    InvalidResponse {
        /// The HTTP status.
        status: u16,
        /// What was wrong with the answer.
        reason: &'static str,
    },
    /// There is no cached token and none may be fetched now.
    #[error(
        "OAuth 2.0 token \"{credentials_id}\": no token yet; send the request or get a token first"
    )]
    NoToken {
        /// The token id.
        credentials_id: String,
    },
    /// The grant needs a browser sign-in.
    #[error(
        "OAuth 2.0 token \"{credentials_id}\": this grant needs a browser sign-in; use Get token in the Apinata app, then run again"
    )]
    InteractionRequired {
        /// The token id.
        credentials_id: String,
    },
    /// The user or provider denied the authorization.
    #[error("authorization was denied: {error}{}", .description.as_ref().map(|d| format!(" ({d})")).unwrap_or_default())]
    AuthorizationDenied {
        /// The provider's `error` code.
        error: String,
        /// The provider's `error_description`, when given.
        description: Option<String>,
    },
    /// The callback carried a different `state` than the request.
    #[error("the authorization callback carried an unexpected state")]
    StateMismatch,
    /// No callback arrived in time.
    #[error("timed out waiting for the authorization callback")]
    CallbackTimeout,
    /// The local callback listener could not start.
    #[error("could not listen for the authorization callback on {address}: {message}")]
    CallbackBind {
        /// The address tried.
        address: String,
        /// Why it failed.
        message: String,
    },
    /// The callback URL cannot be used.
    #[error("the callback URL is not usable: {reason}")]
    InvalidRedirect {
        /// What was wrong with it.
        reason: &'static str,
    },
    /// The user cancelled.
    #[error("cancelled")]
    Cancelled,
    /// The target's auth is not OAuth 2.0.
    #[error("this auth is not OAuth 2.0")]
    NotOAuth2,
    /// The operating system's random source failed.
    #[error("the system random source failed")]
    NoRandomness,
}

impl From<ResolveError> for OAuth2Error {
    fn from(e: ResolveError) -> OAuth2Error {
        OAuth2Error::Resolve(e)
    }
}

impl From<ExecError> for OAuth2Error {
    fn from(e: ExecError) -> OAuth2Error {
        OAuth2Error::Exec(e)
    }
}
