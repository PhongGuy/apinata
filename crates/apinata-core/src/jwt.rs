//! Reads the claims of a JWT for display. Nothing is verified: no signature check, no
//! key, no network.

use base64::Engine as _;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use serde_json::{Map, Value};

// Accepts both padded and unpadded input, as tokens in the wild use either.
const B64: GeneralPurpose = GeneralPurpose::new(
    &base64::alphabet::URL_SAFE,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// Why a text is not a readable JWT.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum JwtError {
    /// The text is not three dot-separated parts.
    #[error("not three dot-separated parts")]
    NotThreeParts,
    /// The named part is not base64url.
    #[error("the {0} is not base64url")]
    NotBase64(&'static str),
    /// The named part is not a JSON object.
    #[error("the {0} is not a JSON object")]
    NotAnObject(&'static str),
    /// A variable reference in the text did not resolve.
    #[error("a variable in the token is undefined")]
    Unresolved,
}

/// The decoded parts of a JWT; the signature is neither decoded nor checked.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DecodedJwt {
    /// The header as pretty JSON, keys in written order.
    pub header: String,
    /// The payload as pretty JSON, keys in written order.
    pub payload: String,
    /// The numeric `exp` claim in Unix seconds, when there is one.
    pub expires_at: Option<i64>,
}

/// Where a token stands against a clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Expiry {
    /// The payload has no numeric `exp`.
    Absent,
    /// `exp` is still ahead.
    Valid {
        /// The `exp` claim in Unix seconds.
        expires_at: i64,
    },
    /// `exp` has been reached.
    Expired {
        /// The `exp` claim in Unix seconds.
        expires_at: i64,
    },
}

impl DecodedJwt {
    /// The token's standing at `now_unix` (Unix seconds); the caller owns the clock.
    pub fn expiry(&self, now_unix: i64) -> Expiry {
        match self.expires_at {
            None => Expiry::Absent,
            Some(expires_at) if now_unix >= expires_at => Expiry::Expired { expires_at },
            Some(expires_at) => Expiry::Valid { expires_at },
        }
    }
}

/// Decodes the header and payload of `token`, which must be three dot-separated parts whose
/// first two are base64url JSON objects. Never panics.
pub fn decode_jwt(token: &str) -> Result<DecodedJwt, JwtError> {
    let parts: Vec<&str> = token.trim().split('.').collect();
    let [header, payload, _signature] = parts[..] else {
        return Err(JwtError::NotThreeParts);
    };
    let header = object(header, "header")?;
    let payload = object(payload, "payload")?;
    // A float `exp` counts down to whole seconds; anything else is no expiry.
    let expires_at = payload.get("exp").and_then(|v| {
        v.as_i64().or_else(|| {
            v.as_f64()
                .filter(|f| f.is_finite())
                .map(|f| f.floor() as i64)
        })
    });
    Ok(DecodedJwt {
        header: pretty(header),
        payload: pretty(payload),
        expires_at,
    })
}

fn object(part: &str, name: &'static str) -> Result<Map<String, Value>, JwtError> {
    let bytes = B64.decode(part).map_err(|_| JwtError::NotBase64(name))?;
    match serde_json::from_slice(&bytes) {
        Ok(Value::Object(map)) => Ok(map),
        _ => Err(JwtError::NotAnObject(name)),
    }
}

fn pretty(map: Map<String, Value>) -> String {
    serde_json::to_string_pretty(&Value::Object(map)).unwrap_or_default()
}
