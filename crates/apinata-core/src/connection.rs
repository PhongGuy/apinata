//! What a hop's connection was: the TLS layer of an https hop, the size of the response's
//! header block, and a plain-words description of a status code.

use std::fmt;
use std::time::SystemTime;

use http::{HeaderMap, StatusCode};
use x509_cert::Certificate;
use x509_cert::der::Decode;

/// A TLS protocol version a connection negotiated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TlsVersion {
    /// TLS 1.0.
    Tls1_0,
    /// TLS 1.1.
    Tls1_1,
    /// TLS 1.2.
    Tls1_2,
    /// TLS 1.3.
    Tls1_3,
}

impl fmt::Display for TlsVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            TlsVersion::Tls1_0 => "TLS 1.0",
            TlsVersion::Tls1_1 => "TLS 1.1",
            TlsVersion::Tls1_2 => "TLS 1.2",
            TlsVersion::Tls1_3 => "TLS 1.3",
        })
    }
}

/// The TLS layer of an https hop: the negotiated version and the peer's leaf certificate.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TlsDetails {
    /// The negotiated version; `None` when the stack did not report it.
    pub version: Option<TlsVersion>,
    /// The leaf certificate's subject as an RFC 4514 name, such as `CN=example.com`; `None`
    /// when the peer sent no certificate that parses.
    pub subject: Option<String>,
    /// The leaf certificate's issuer, written like [`TlsDetails::subject`].
    pub issuer: Option<String>,
    /// When the leaf certificate stops being valid (its notAfter).
    pub expires: Option<SystemTime>,
    /// Whether the certificate was checked; false when the check was turned off.
    pub verified: bool,
}

impl TlsDetails {
    /// [`TlsDetails::expires`] as `2126-09-15 05:03:13 UTC`.
    pub fn expires_label(&self) -> Option<String> {
        let at: chrono::DateTime<chrono::Utc> = self.expires?.into();
        Some(at.format("%Y-%m-%d %H:%M:%S UTC").to_string())
    }

    /// The heap bytes the details own.
    pub(crate) fn owned_bytes(&self) -> usize {
        self.subject.as_ref().map_or(0, String::capacity)
            + self.issuer.as_ref().map_or(0, String::capacity)
    }
}

/// The local end of the connection `res` came over, as the HTTP stack recorded it.
pub(crate) fn local_addr(res: &reqwest::Response) -> Option<std::net::SocketAddr> {
    use hyper_util::client::legacy::connect::HttpInfo;
    res.extensions().get::<HttpInfo>().map(HttpInfo::local_addr)
}

/// The TLS details reqwest attached to `res`, `verified` saying whether the check ran; `None`
/// for a hop that was not over TLS.
pub(crate) fn tls_details(res: &reqwest::Response, verified: bool) -> Option<TlsDetails> {
    let info = res.extensions().get::<reqwest::tls::TlsInfo>()?;
    let version = info.version().and_then(|v| {
        use reqwest::tls::Version;
        [
            (Version::TLS_1_0, TlsVersion::Tls1_0),
            (Version::TLS_1_1, TlsVersion::Tls1_1),
            (Version::TLS_1_2, TlsVersion::Tls1_2),
            (Version::TLS_1_3, TlsVersion::Tls1_3),
        ]
        .into_iter()
        .find_map(|(wire, ours)| (wire == v).then_some(ours))
    });
    // A certificate that does not parse leaves its fields empty rather than failing the send.
    let cert = info
        .peer_certificate()
        .and_then(|der| Certificate::from_der(der).ok());
    let tbs = cert.as_ref().map(Certificate::tbs_certificate);
    Some(TlsDetails {
        version,
        subject: tbs.map(|t| t.subject().to_string()),
        issuer: tbs.map(|t| t.issuer().to_string()),
        expires: tbs.map(|t| std::time::UNIX_EPOCH + t.validity().not_after.to_unix_duration()),
        verified,
    })
}

/// The size of a response's header block written as HTTP/1.1 text: the status line, each
/// `name: value` line and the blank line, so an HTTP/2 answer counts uncompressed.
pub(crate) fn response_header_size(status: StatusCode, headers: &HeaderMap) -> u64 {
    let reason = status.canonical_reason().unwrap_or("");
    let status_line = "HTTP/1.1 000 \r\n".len() + reason.len();
    let fields: usize = headers
        .iter()
        .map(|(name, value)| name.as_str().len() + ": \r\n".len() + value.len())
        .sum();
    (status_line + fields + "\r\n".len()) as u64
}

/// What status `code` means in one sentence; `None` for a code outside the registered set.
pub fn status_description(code: u16) -> Option<&'static str> {
    Some(match code {
        100 => "The server got the request headers; the client may send the body.",
        101 => "The server is switching to the protocol the client asked for.",
        102 => "The server accepted the request and is still working on it.",
        103 => "The server sends some response headers ahead of the final response.",
        200 => "The request succeeded.",
        201 => "The request succeeded and created a new resource.",
        202 => "The request was accepted, but its processing has not finished.",
        203 => "The request succeeded; a proxy changed the returned content.",
        204 => "The request succeeded and the response has no body.",
        205 => "The request succeeded; the client should reset its view of the document.",
        206 => "The server sent only the part of the resource the range asked for.",
        207 => "The body holds a separate status for each of several resources.",
        208 => "Members of this binding were already listed earlier in the response.",
        226 => "The server applied instance manipulations to the resource.",
        300 => "The resource has several representations to choose from.",
        301 => "The resource moved permanently to the URL in the Location header.",
        302 => "The resource is temporarily at the URL in the Location header.",
        303 => "The answer is at the URL in the Location header, fetched with GET.",
        304 => "The resource has not changed since the copy the client already holds.",
        305 => "The resource must be reached through the proxy in the Location header.",
        307 => "The resource is temporarily at another URL; repeat the request there unchanged.",
        308 => "The resource moved permanently; repeat the request at the new URL unchanged.",
        400 => "The server cannot process the request because it is malformed.",
        401 => "The request needs valid authentication credentials.",
        402 => "Payment is required before the request is served.",
        403 => "The server understood the request but refuses to authorize it.",
        404 => "The server cannot find the requested resource.",
        405 => "The resource does not support the request's method.",
        406 => "The server cannot produce a response that matches the Accept headers.",
        407 => "The request needs authentication with the proxy.",
        408 => "The server timed out waiting for the request.",
        409 => "The request conflicts with the current state of the resource.",
        410 => "The resource is gone for good and has no forwarding address.",
        411 => "The server requires a Content-Length header.",
        412 => "A precondition in the request headers was not met.",
        413 => "The request body is larger than the server will accept.",
        414 => "The URL is longer than the server will accept.",
        415 => "The server does not support the request body's media type.",
        416 => "The requested range cannot be served for this resource.",
        417 => "The server cannot meet the Expect header's requirement.",
        418 => "The server refuses to brew coffee because it is a teapot.",
        421 => "The request reached a server that cannot answer for this host.",
        422 => "The request is well formed, but its content cannot be processed.",
        423 => "The resource is locked.",
        424 => "The request failed because a request it depends on failed.",
        425 => "The server will not risk processing a request that might be replayed.",
        426 => "The client must switch to the protocol named in the Upgrade header.",
        428 => "The server requires the request to be conditional.",
        429 => "The client sent too many requests in a given time.",
        431 => "The request's header fields are too large.",
        451 => "The resource is unavailable for legal reasons.",
        500 => "The server hit an unexpected condition and could not complete the request.",
        501 => "The server does not support the functionality the request needs.",
        502 => "The server, acting as a gateway, got an invalid response from upstream.",
        503 => "The server cannot handle the request right now, often from overload.",
        504 => "The server, acting as a gateway, got no timely response from upstream.",
        505 => "The server does not support the request's HTTP version.",
        506 => "The server's content negotiation is misconfigured.",
        507 => "The server cannot store what the request needs.",
        508 => "The server found an infinite loop while processing the request.",
        510 => "The request needs further extensions to be fulfilled.",
        511 => "The client must authenticate to gain network access.",
        _ => return None,
    })
}
