//! The words for a failed connection, shared by the app, the CLI report and the MCP server.

use crate::execute::{ConnectCause, ExecError};
use crate::runner::RunError;

/// A connect failure in plain words: a short title and one sentence naming the host and, where
/// the cause is known, the likely fix. Built from the failed URL and the [`ConnectCause`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ConnectHint {
    title: &'static str,
    sentence: String,
}

impl ConnectHint {
    /// The hint for a failure to reach `url` for `cause`.
    pub fn new(url: &str, cause: ConnectCause) -> ConnectHint {
        let host = host(url);
        let (title, sentence) = match cause {
            ConnectCause::Refused => (
                "Connection refused",
                format!(
                    "Nothing is listening at {}. Is the server running?",
                    host_port(url)
                ),
            ),
            ConnectCause::Dns => (
                "DNS lookup failed",
                format!("The host {host} could not be found. Check the URL for typos."),
            ),
            ConnectCause::TlsUnknownIssuer => (
                "TLS error",
                format!(
                    "The certificate of {host} is not trusted (unknown issuer or self-signed). \
                     Trust its CA or turn certificate verification off."
                ),
            ),
            ConnectCause::TlsWrongProtocol => (
                "TLS error",
                format!("{host} did not answer TLS. The URL may need http:// instead of https://."),
            ),
            ConnectCause::TlsExpired => (
                "TLS error",
                format!("The certificate of {host} has expired."),
            ),
            ConnectCause::Tls => (
                "TLS error",
                format!("The secure connection to {host} failed."),
            ),
            _ => (
                "Could not connect",
                format!("The connection to {host} failed."),
            ),
        };
        ConnectHint { title, sentence }
    }

    /// The short heading, such as `Connection refused`.
    pub fn title(&self) -> &'static str {
        self.title
    }

    /// The explanation, a full sentence.
    pub fn sentence(&self) -> &str {
        &self.sentence
    }

    /// `title: sentence` on one line.
    pub fn text(&self) -> String {
        format!("{}: {}", self.title, self.sentence)
    }

    /// `url: title: sentence (detail)` on one line, `detail` being the transport error.
    pub fn line(&self, url: &str, detail: &str) -> String {
        format!("{url}: {} ({detail})", self.text())
    }
}

impl ExecError {
    /// The hint for a connect failure; `None` for every other error.
    pub fn connect_hint(&self) -> Option<ConnectHint> {
        match self {
            ExecError::Connect { url, cause, .. } => Some(ConnectHint::new(url, *cause)),
            _ => None,
        }
    }

    /// The cause of a connect failure; `None` for every other error.
    pub fn connect_cause(&self) -> Option<ConnectCause> {
        match self {
            ExecError::Connect { cause, .. } => Some(*cause),
            _ => None,
        }
    }

    /// The start of [`ExecError::hinted_line`] up to the end of the sentence, where a front end
    /// may add advice of its own before the transport detail; `None` unless a connect failure.
    pub fn hinted_prefix(&self) -> Option<String> {
        match self {
            ExecError::Connect { url, cause, .. } => {
                Some(format!("{url}: {}", ConnectHint::new(url, *cause).text()))
            }
            _ => None,
        }
    }

    /// A connect failure as one line with its hint and transport detail; `None` otherwise.
    pub fn hinted_line(&self) -> Option<String> {
        match self {
            ExecError::Connect {
                url,
                message,
                cause,
                ..
            } => Some(ConnectHint::new(url, *cause).line(url, message)),
            _ => None,
        }
    }
}

impl RunError {
    /// The hint for a failed request whose cause was a connect failure, also inside an OAuth2
    /// token fetch; `None` otherwise.
    pub fn connect_hint(&self) -> Option<ConnectHint> {
        self.exec_error().and_then(ExecError::connect_hint)
    }

    /// The error as one line: the hinted line for a connect failure, else its `Display` text.
    pub fn hinted_line(&self) -> String {
        self.exec_error()
            .and_then(ExecError::hinted_line)
            .unwrap_or_else(|| self.to_string())
    }

    /// [`ExecError::connect_cause`] of the underlying error.
    pub fn connect_cause(&self) -> Option<ConnectCause> {
        self.exec_error().and_then(ExecError::connect_cause)
    }

    /// [`ExecError::hinted_prefix`] of the underlying error.
    pub fn hinted_prefix(&self) -> Option<String> {
        self.exec_error().and_then(ExecError::hinted_prefix)
    }

    fn exec_error(&self) -> Option<&ExecError> {
        match self {
            RunError::Exec(e) | RunError::OAuth2(crate::OAuth2Error::Exec(e)) => Some(e),
            _ => None,
        }
    }
}

// The URL's host, or the URL text when it has none.
fn host(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_else(|| url.to_owned())
}

// "host:port" with the scheme's default port filled in, or the URL text.
fn host_port(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| Some(format!("{}:{}", u.host_str()?, u.port_or_known_default()?)))
        .unwrap_or_else(|| url.to_owned())
}
