//! Where a PAC script lives, parsed once from user input into a URL that can be fetched and a
//! redacted form that can be shown.

use std::fmt;
use std::path::Path;

use url::Url;

/// The longest PAC source accepted, in bytes.
const MAX_LEN: usize = 2048;

/// Where the PAC script lives: an http(s) URL or a file, normalised to a file:// URL.
#[derive(Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct PacSource {
    url: String,
    shown: String,
}

/// Why a PAC source was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PacSourceError {
    /// The input was empty or only whitespace.
    #[error("the PAC source is empty")]
    Empty,
    /// The input was longer than 2048 bytes.
    #[error("the PAC source is longer than 2048 bytes")]
    TooLong,
    /// The URL's scheme is not http, https or file.
    #[error("the PAC source must be an http, https or file URL, not {scheme}")]
    Scheme {
        /// The refused scheme.
        scheme: String,
    },
    /// The input reads as `host:port`, with no scheme in front.
    #[error("the PAC source needs http:// or https:// in front of the host")]
    MissingScheme,
    /// A relative path was given without a directory to resolve it against.
    #[error("the PAC path {path} is relative and there is no directory to resolve it against")]
    Relative {
        /// The path as given, any user name and password before an `@` shown as `***`.
        path: String,
    },
    /// The input could not be read as a URL.
    #[error("the PAC source is not a valid URL: {message}")]
    Invalid {
        /// What the URL parser said.
        message: String,
    },
}

impl PacSource {
    /// Parses `input` as an http, https or file URL, or as a path; a relative path is joined to
    /// `base` when there is one.
    pub fn parse(input: &str, base: Option<&Path>) -> Result<PacSource, PacSourceError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(PacSourceError::Empty);
        }
        if input.len() > MAX_LEN {
            return Err(PacSourceError::TooLong);
        }
        // A one-letter scheme is a Windows drive, so only two or more letters make a URL.
        let url = if let Some(scheme) = sniff_scheme(input).filter(|scheme| scheme.len() >= 2) {
            if looks_like_port(&input[scheme.len() + 1..]) {
                return Err(PacSourceError::MissingScheme);
            }
            let url = Url::parse(input).map_err(|e| PacSourceError::Invalid {
                message: e.to_string(),
            })?;
            if !matches!(url.scheme(), "http" | "https" | "file") {
                return Err(PacSourceError::Scheme {
                    scheme: url.scheme().to_owned(),
                });
            }
            url
        } else if ip_and_port(input) {
            return Err(PacSourceError::MissingScheme);
        } else {
            file_url(input, base)?
        };
        Ok(PacSource {
            shown: redact(&url),
            url: url.into(),
        })
    }

    /// The full URL, credentials included; meant for fetching only, never for display.
    pub fn as_str(&self) -> &str {
        &self.url
    }
}

/// Whether the text after a sniffed scheme is a port: digits up to the end, a path, query or
/// fragment.
fn looks_like_port(after: &str) -> bool {
    let end = after.find(['/', '?', '#']).unwrap_or(after.len());
    end > 0 && after[..end].bytes().all(|b| b.is_ascii_digit())
}

/// Whether `input` starts with an IP literal host and a port, such as `127.0.0.1:8080` or
/// `[::1]:8080`, which `sniff_scheme` cannot catch because no scheme starts with a digit.
fn ip_and_port(input: &str) -> bool {
    let after = if let Some(rest) = input.strip_prefix('[') {
        rest.split_once("]:").map(|(_, after)| after)
    } else {
        input
            .split_once(':')
            .filter(|(host, _)| {
                !host.is_empty() && host.bytes().all(|b| b.is_ascii_digit() || b == b'.')
            })
            .map(|(_, after)| after)
    };
    after.is_some_and(looks_like_port)
}

/// The text before the first `:` when it has the shape of a URL scheme.
fn sniff_scheme(input: &str) -> Option<&str> {
    let (scheme, _) = input.split_once(':')?;
    let mut chars = scheme.chars();
    let first = chars.next()?;
    (first.is_ascii_alphabetic()
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')))
    .then_some(scheme)
}

fn file_url(input: &str, base: Option<&Path>) -> Result<Url, PacSourceError> {
    let path = Path::new(input);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        match base {
            Some(base) => base.join(path),
            None => {
                return Err(PacSourceError::Relative {
                    path: redact_path(input),
                });
            }
        }
    };
    Url::from_file_path(&absolute).map_err(|()| PacSourceError::Invalid {
        message: format!("{} is not a usable file path", absolute.display()),
    })
}

/// `input` shown up to its last `@` as `***` when its first segment holds a `:` or `@`, since a
/// `u:pw@host/p.pac` is taken for a relative path and its password may itself hold a `/`.
fn redact_path(input: &str) -> String {
    let end = input.find(['/', '\\', '?', '#']).unwrap_or(input.len());
    let credentialed = input[..end].contains([':', '@']);
    match input.rfind('@').filter(|_| credentialed) {
        Some(at) => format!("***{}", &input[at..]),
        None => input.to_owned(),
    }
}

/// The URL with any user name and password replaced by `***`.
fn redact(url: &Url) -> String {
    if url.username().is_empty() && url.password().is_none() {
        return url.to_string();
    }
    let mut bare = url.clone();
    // Clearing credentials only fails for URLs that cannot carry them, which never reach here.
    let _ = bare.set_username("");
    let _ = bare.set_password(None);
    let text = bare.to_string();
    let prefix = format!("{}://", bare.scheme());
    match text.strip_prefix(&prefix) {
        Some(rest) => format!("{prefix}***@{rest}"),
        None => text,
    }
}

impl fmt::Display for PacSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.shown)
    }
}

impl fmt::Debug for PacSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("PacSource").field(&self.shown).finish()
    }
}
