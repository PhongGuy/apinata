//! The HTTP version policy a client offers, and the version a hop was answered with.

use std::fmt;
use std::str::FromStr;

/// Which HTTP versions the executor's clients may speak.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum HttpVersionPolicy {
    /// HTTP/1.1 only; over TLS the client offers just `http/1.1`.
    #[default]
    Http1Only,
    /// Let TLS negotiate: offer `h2` and `http/1.1`; plain HTTP stays HTTP/1.1.
    Auto,
    /// Speak HTTP/2 from the first byte, over TLS and plain HTTP alike.
    Http2PriorKnowledge,
}

/// The CLI spelling: `1.1`, `auto` or `2-prior-knowledge`.
impl fmt::Display for HttpVersionPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            HttpVersionPolicy::Http1Only => "1.1",
            HttpVersionPolicy::Auto => "auto",
            HttpVersionPolicy::Http2PriorKnowledge => "2-prior-knowledge",
        })
    }
}

/// Parses exactly the spellings `Display` writes, case-sensitively.
impl FromStr for HttpVersionPolicy {
    type Err = String;

    fn from_str(s: &str) -> Result<HttpVersionPolicy, String> {
        match s {
            "1.1" => Ok(HttpVersionPolicy::Http1Only),
            "auto" => Ok(HttpVersionPolicy::Auto),
            "2-prior-knowledge" => Ok(HttpVersionPolicy::Http2PriorKnowledge),
            _ => Err(format!(
                "unknown HTTP version {s:?}; expected 1.1, auto or 2-prior-knowledge"
            )),
        }
    }
}

/// The HTTP version a response arrived over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum HttpVersion {
    /// HTTP/1.0.
    Http10,
    /// HTTP/1.1.
    Http11,
    /// HTTP/2.
    Http2,
}

/// The protocol name: `HTTP/1.0`, `HTTP/1.1` or `HTTP/2`.
impl fmt::Display for HttpVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            HttpVersion::Http10 => "HTTP/1.0",
            HttpVersion::Http11 => "HTTP/1.1",
            HttpVersion::Http2 => "HTTP/2",
        })
    }
}

impl HttpVersion {
    /// The version a response arrived over; `None` for one outside the enum (0.9, 3).
    pub(crate) fn from_wire(v: http::Version) -> Option<HttpVersion> {
        match v {
            http::Version::HTTP_10 => Some(HttpVersion::Http10),
            http::Version::HTTP_11 => Some(HttpVersion::Http11),
            http::Version::HTTP_2 => Some(HttpVersion::Http2),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wire_version_outside_the_enum_maps_to_none() {
        assert_eq!(HttpVersion::from_wire(http::Version::HTTP_09), None);
        assert_eq!(HttpVersion::from_wire(http::Version::HTTP_3), None);
    }
}
