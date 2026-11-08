//! The client side of the WebSocket opening handshake (RFC 6455 section 4.1).

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use reqwest::header::HeaderMap;
use sha1::Digest as _;
use url::Url;

use crate::resolve::redact_url;

/// The subprotocols offered, preferred first.
pub(crate) const OFFER: &str = "graphql-transport-ws, graphql-ws";

/// 16 OS-random bytes, base64 (24 characters); None when the OS generator fails.
pub(crate) fn new_key() -> Option<String> {
    use rand::{TryRngCore, rngs::OsRng};
    let mut bytes = [0u8; 16];
    OsRng.try_fill_bytes(&mut bytes).ok()?;
    Some(STANDARD.encode(bytes))
}

/// Upgrade: websocket, Connection: Upgrade, Sec-WebSocket-Key, Sec-WebSocket-Version: 13,
/// Sec-WebSocket-Protocol: OFFER.
pub(crate) fn upgrade_headers(key: &str) -> Vec<(String, String)> {
    [
        ("Upgrade", "websocket"),
        ("Connection", "Upgrade"),
        ("Sec-WebSocket-Key", key),
        ("Sec-WebSocket-Version", "13"),
        ("Sec-WebSocket-Protocol", OFFER),
    ]
    .into_iter()
    .map(|(n, v)| (n.to_owned(), v.to_owned()))
    .collect()
}

/// The handshake URL as messages name it: `ws` for `http`, `wss` for `https`, redacted.
pub(crate) fn ws_url(url: &Url) -> String {
    let text = url.as_str();
    let ws = match url.scheme() {
        "http" => format!("ws{}", &text[4..]),
        "https" => format!("wss{}", &text[5..]),
        _ => text.to_owned(),
    };
    redact_url(&ws)
}

/// RFC 6455's Sec-WebSocket-Accept for `key`: base64(SHA-1(key + 258EAFA5-E914-47DA-95CA-C5AB0DC85B11)).
pub(crate) fn accept_for(key: &str) -> String {
    const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    STANDARD.encode(sha1::Sha1::digest(format!("{key}{GUID}").as_bytes()))
}

/// Judges a 101 reply's headers; Err holds the reason that follows "failed: ".
pub(crate) fn check_reply(key: &str, headers: &HeaderMap) -> Result<(), String> {
    let accept = headers
        .get("sec-websocket-accept")
        .and_then(|v| v.to_str().ok());
    if accept != Some(accept_for(key).as_str()) {
        return Err("Sec-WebSocket-Accept does not match".to_owned());
    }
    match headers
        .get("sec-websocket-protocol")
        .map(|v| String::from_utf8_lossy(v.as_bytes()).trim().to_owned())
        .as_deref()
    {
        Some("graphql-transport-ws") => Ok(()),
        None | Some("") => Err("the server chose no subprotocol".to_owned()),
        Some("graphql-ws") => {
            Err("the server chose graphql-ws, which is not supported yet".to_owned())
        }
        Some(other) => Err(format!("the server chose subprotocol {other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_24_base64_characters_and_fresh_each_time() {
        let (a, b) = (new_key().expect("key"), new_key().expect("key"));
        assert_eq!(
            (a.len(), STANDARD.decode(&a).expect("base64").len()),
            (24, 16)
        );
        assert_ne!(a, b);
    }

    #[test]
    fn the_accept_matches_rfc_6455s_example() {
        assert_eq!(
            super::accept_for("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }
}
