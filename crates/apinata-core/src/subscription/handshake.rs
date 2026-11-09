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

/// Upgrade: websocket, Connection: Upgrade, Sec-WebSocket-Key, Sec-WebSocket-Version: 13.
/// The subprotocol offer is not among them, since a user-set one replaces it.
pub(crate) fn upgrade_headers(key: &str) -> Vec<(String, String)> {
    [
        ("Upgrade", "websocket"),
        ("Connection", "Upgrade"),
        ("Sec-WebSocket-Key", key),
        ("Sec-WebSocket-Version", "13"),
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

/// The GraphQL-over-WebSocket protocol a socket speaks, as the server's reply chose it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Codec {
    /// graphql-transport-ws (graphql-ws the library).
    TransportWs,
    /// Legacy graphql-ws (subscriptions-transport-ws).
    Legacy,
}

/// Judges a 101 reply's headers and names the codec it chose; Err holds the reason that
/// follows "failed: ".
pub(crate) fn check_reply(key: &str, headers: &HeaderMap) -> Result<Codec, String> {
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
        Some("graphql-transport-ws") => Ok(Codec::TransportWs),
        Some("graphql-ws") => Ok(Codec::Legacy),
        None | Some("") => Err("the server chose no subprotocol".to_owned()),
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

    #[test]
    fn the_chosen_subprotocol_selects_the_codec() {
        let key = "dGhlIHNhbXBsZSBub25jZQ==";
        let reply = |protocol: Option<&str>| {
            let mut h = HeaderMap::new();
            h.insert(
                "sec-websocket-accept",
                accept_for(key).parse().expect("value"),
            );
            if let Some(p) = protocol {
                h.insert("sec-websocket-protocol", p.parse().expect("value"));
            }
            check_reply(key, &h)
        };
        assert_eq!(reply(Some("graphql-transport-ws")), Ok(Codec::TransportWs));
        assert_eq!(reply(Some("graphql-ws")), Ok(Codec::Legacy));
        assert_eq!(reply(Some(" graphql-ws ")), Ok(Codec::Legacy));
        assert_eq!(
            reply(Some("weird")),
            Err("the server chose subprotocol \"weird\"".to_owned())
        );
        assert_eq!(
            reply(None),
            Err("the server chose no subprotocol".to_owned())
        );
    }
}
