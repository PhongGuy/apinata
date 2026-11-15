//! The team server's token as git environment, and masking it in text.

use std::fmt;

use crate::GitOptions;

/// What every team token starts with; the rest is base64url.
const TOKEN_PREFIX: &str = "plk_";
const REDACTED: &str = "plk_[redacted]";

/// A team server's public URL and the token that signs git in to it.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TeamAuth {
    public_url: String,
    token: String,
}

impl TeamAuth {
    /// `public_url` loses any trailing `/`, so the header's config key matches git's URLs.
    pub fn new(public_url: impl Into<String>, token: impl Into<String>) -> TeamAuth {
        let public_url = public_url.into().trim_end_matches('/').to_owned();
        TeamAuth {
            public_url,
            token: token.into(),
        }
    }

    /// The server's URL, with no trailing `/`.
    pub fn public_url(&self) -> &str {
        &self.public_url
    }

    /// The secret token.
    pub fn token(&self) -> &str {
        &self.token
    }
}

impl fmt::Debug for TeamAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TeamAuth")
            .field("public_url", &self.public_url)
            .field("token", &REDACTED)
            .finish()
    }
}

/// The name and email team commits are authored and committed under.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Identity {
    name: String,
    email: String,
}

impl Identity {
    /// An identity from a display name and an email.
    pub fn new(name: impl Into<String>, email: impl Into<String>) -> Identity {
        Identity {
            name: name.into(),
            email: email.into(),
        }
    }

    /// The display name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The email.
    pub fn email(&self) -> &str {
        &self.email
    }
}

/// `base` plus the env that sends the token as a header to the server only, turns off stored
/// credentials and redirects, redacts git's traces, and sets the commit identity. Appended last,
/// so each key wins over the same key in `base`.
pub fn team_options(base: GitOptions, auth: &TeamAuth, identity: &Identity) -> GitOptions {
    let header_key = format!("http.{}/.extraHeader", auth.public_url);
    let header = format!("Authorization: Bearer {}", auth.token);
    [
        ("GIT_CONFIG_COUNT", "3"),
        ("GIT_CONFIG_KEY_0", header_key.as_str()),
        ("GIT_CONFIG_VALUE_0", header.as_str()),
        ("GIT_CONFIG_KEY_1", "credential.helper"),
        ("GIT_CONFIG_VALUE_1", ""),
        ("GIT_CONFIG_KEY_2", "http.followRedirects"),
        ("GIT_CONFIG_VALUE_2", "false"),
        ("GIT_TRACE_REDACT", "1"),
        ("GIT_AUTHOR_NAME", identity.name()),
        ("GIT_AUTHOR_EMAIL", identity.email()),
        ("GIT_COMMITTER_NAME", identity.name()),
        ("GIT_COMMITTER_EMAIL", identity.email()),
    ]
    .into_iter()
    .fold(base, |opts, (key, value)| opts.env(key, value))
}

/// `text` with every `plk_` that one or more base64url characters follow replaced by
/// `plk_[redacted]`, wherever it sits in a word.
pub fn redact_token(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(TOKEN_PREFIX) {
        let after = &rest[at + TOKEN_PREFIX.len()..];
        let len = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .unwrap_or(after.len());
        out.push_str(&rest[..at]);
        if len == 0 {
            out.push_str(TOKEN_PREFIX);
        } else {
            out.push_str(REDACTED);
        }
        rest = &after[len..];
    }
    out.push_str(rest);
    out
}
