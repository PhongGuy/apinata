//! The client side of the WebSocket opening handshake (RFC 6455 section 4.1).

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

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
}
