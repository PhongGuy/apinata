//! The Akamai EdgeGrid EG1-HMAC-SHA256 signer, built from interpolated fields.

use crate::edit::EdgeGridSpec;
use crate::js::{is_js_whitespace, js_trim, number_to_string};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};
use url::Url;

/// The reference's `MAX_BODY_SIZE_DEFAULT`.
const DEFAULT_BODY_LIMIT: usize = 131_072;

/// The bytes a hop sends: text bodies are cut in UTF-16 units, the rest in bytes.
pub(crate) enum Payload<'a> {
    Text(&'a str),
    Bytes(&'a [u8]),
}

/// One outgoing hop as the signer sees it.
pub(crate) struct Outgoing<'a> {
    pub(crate) method: &'a str,
    pub(crate) url: &'a Url,
    pub(crate) headers: &'a [(String, String)],
    pub(crate) payload: Payload<'a>,
}

/// A validated EdgeGrid configuration ready to sign hops.
#[derive(Clone, PartialEq)] // no Debug: it holds the secret
pub(crate) struct Signer {
    access_token: String,
    client_token: String,
    client_secret: String,
    /// A fixed nonce, `None` for a fresh one per signature.
    nonce: Option<String>,
    /// A fixed timestamp, `None` for the current time.
    timestamp: Option<String>,
    /// The trimmed baseURL as given, possibly without a scheme.
    base_url: Option<String>,
    headers_to_sign: String,
    /// The truncation limit from [`body_limit`].
    max_body_size: Option<usize>,
}

impl Signer {
    /// From interpolated fields. Errors, in this order: "accessToken is required",
    /// "clientToken is required", "clientSecret is required", "baseURL is not a valid URL".
    /// A nonce, timestamp or baseURL that is not present is generated or ignored.
    pub(crate) fn new(spec: &EdgeGridSpec, url: &Url) -> Result<Signer, String> {
        for (value, name) in [
            (&spec.access_token, "accessToken"),
            (&spec.client_token, "clientToken"),
            (&spec.client_secret, "clientSecret"),
        ] {
            if !is_present(value) {
                return Err(format!("{name} is required"));
            }
        }
        let base_url = is_present(&spec.base_url).then(|| js_trim(&spec.base_url).to_owned());
        if let Some(base) = &base_url {
            base_origin(base, url.scheme()).ok_or("baseURL is not a valid URL")?;
        }
        let present = |v: &String| is_present(v).then(|| v.clone());
        Ok(Signer {
            access_token: spec.access_token.clone(),
            client_token: spec.client_token.clone(),
            client_secret: spec.client_secret.clone(),
            nonce: present(&spec.nonce),
            timestamp: present(&spec.timestamp),
            base_url,
            headers_to_sign: spec.headers_to_sign.clone(),
            max_body_size: body_limit(spec.max_body_size),
        })
    }

    /// The access token, client token and client secret, so a log can hide them.
    pub(crate) fn secrets(&self) -> [&str; 3] {
        [&self.access_token, &self.client_token, &self.client_secret]
    }

    /// The seven tab-joined fields signed for `hop` with this nonce and timestamp.
    ///
    /// Errors when the baseURL, valid for the first hop, cannot be parsed with this hop's scheme.
    pub(crate) fn data_to_sign(
        &self,
        hop: &Outgoing,
        nonce: &str,
        timestamp: &str,
    ) -> Result<String, String> {
        // A baseURL replaces only the scheme and host; the hop keeps its path and query.
        let origin = match self.base_url.as_deref() {
            Some(base) => {
                Some(base_origin(base, hop.url.scheme()).ok_or("baseURL is not a valid URL")?)
            }
            None => None,
        };
        let target = origin.as_ref().unwrap_or(hop.url);
        let mut host = target.host_str().unwrap_or_default().to_owned();
        if let Some(port) = target.port() {
            host = format!("{host}:{port}");
        }
        // JavaScript's `search` is empty for a bare `?`, so only a non-empty query is signed.
        let mut path = hop.url.path().to_owned();
        if let Some(query) = hop.url.query().filter(|q| !q.is_empty()) {
            path = format!("{path}?{query}");
        }
        Ok([
            hop.method.to_uppercase(),
            target.scheme().to_owned(),
            host,
            path,
            self.canonical_headers(hop.headers),
            self.content_hash(hop),
            self.auth_header(nonce, timestamp),
        ]
        .join("\t"))
    }

    /// The `Authorization` value, with the fixed nonce and timestamp or fresh ones.
    pub(crate) fn sign(&self, hop: &Outgoing) -> Result<String, String> {
        let nonce = self
            .nonce
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let timestamp = self.timestamp.clone().unwrap_or_else(|| {
            chrono::Utc::now()
                .format("%Y%m%dT%H:%M:%S+0000")
                .to_string()
        });
        // The signing key is the base64 text of HMAC(secret, timestamp), used as UTF-8 bytes.
        let key = hmac_base64(self.client_secret.as_bytes(), timestamp.as_bytes());
        let data = self.data_to_sign(hop, &nonce, &timestamp)?;
        let signature = hmac_base64(key.as_bytes(), data.as_bytes());
        Ok(format!(
            "{}signature={signature}",
            self.auth_header(&nonce, &timestamp)
        ))
    }

    /// The header without its signature, which is also the last field signed.
    fn auth_header(&self, nonce: &str, timestamp: &str) -> String {
        format!(
            "EG1-HMAC-SHA256 client_token={};access_token={};timestamp={timestamp};nonce={nonce};",
            self.client_token, self.access_token
        )
    }

    /// The listed headers the hop sends, in list order, values trimmed and whitespace collapsed.
    fn canonical_headers(&self, headers: &[(String, String)]) -> String {
        if !is_present(&self.headers_to_sign) {
            return String::new();
        }
        self.headers_to_sign
            .split(',')
            .map(|name| js_trim(name).to_lowercase())
            .filter(|name| !name.is_empty())
            .filter_map(|name| {
                let value = headers
                    .iter()
                    .rev()
                    .find(|(k, _)| k.to_lowercase() == name)?;
                Some(format!("{name}:{}", collapse(js_trim(&value.1))))
            })
            .collect::<Vec<_>>()
            .join("\t")
    }

    /// The base64 SHA-256 of a non-empty POST body cut to the limit; empty otherwise.
    fn content_hash(&self, hop: &Outgoing) -> String {
        if hop.method.to_uppercase() != "POST" {
            return String::new();
        }
        let limit = self.max_body_size.unwrap_or(usize::MAX);
        let digest = match hop.payload {
            Payload::Text("") | Payload::Bytes([]) => return String::new(),
            Payload::Text(text) => {
                let units: Vec<u16> = text.encode_utf16().collect();
                if units.len() > limit {
                    Sha256::digest(String::from_utf16_lossy(&units[..limit]).as_bytes())
                } else {
                    Sha256::digest(text.as_bytes())
                }
            }
            Payload::Bytes(bytes) => Sha256::digest(&bytes[..bytes.len().min(limit)]),
        };
        STANDARD.encode(digest)
    }
}

/// JavaScript's `replace(/\s+/g, ' ')`.
fn collapse(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_space = false;
    for c in text.chars() {
        if is_js_whitespace(c) {
            if !in_space {
                out.push(' ');
            }
            in_space = true;
        } else {
            out.push(c);
            in_space = false;
        }
    }
    out
}

/// Base64 HMAC-SHA256 of `data` under `key`.
fn hmac_base64(key: &[u8], data: &[u8]) -> String {
    // HMAC accepts a key of any length, so building the MAC cannot fail.
    #[allow(clippy::expect_used)]
    let mut mac =
        <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(data);
    STANDARD.encode(mac.finalize().into_bytes())
}

/// The baseURL parsed with `scheme://` borrowed from the hop when it names none; `None` when
/// it does not parse or has no host.
fn base_origin(base: &str, scheme: &str) -> Option<Url> {
    let parsed = if has_scheme(base) {
        Url::parse(base)
    } else {
        Url::parse(&format!("{scheme}://{base}"))
    };
    parsed.ok().filter(|u| u.host_str().is_some())
}

/// The reference's `/^[a-z][a-z0-9+.-]*:\/\//i`.
fn has_scheme(text: &str) -> bool {
    let Some((scheme, _)) = text.split_once("://") else {
        return false;
    };
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
}

/// The reference's `isStrPresent`: non-empty after a JavaScript trim and not `undefined`.
pub(crate) fn is_present(value: &str) -> bool {
    let trimmed = js_trim(value);
    !trimmed.is_empty() && trimmed != "undefined"
}

/// The truncation limit: 131072 for absent, 0 or NaN; else `parseInt(String(n), 10)`, clamped
/// at 0; `None` (never truncate) when that is NaN.
pub(crate) fn body_limit(max: Option<f64>) -> Option<usize> {
    let n = match max {
        Some(n) if n != 0.0 && !n.is_nan() => n,
        _ => return Some(DEFAULT_BODY_LIMIT),
    };
    // parseInt reads an optional sign and the leading decimal digits of String(n), so an
    // exponent form such as "1e+21" stops at the "e".
    let text = number_to_string(n);
    let (negative, rest) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.as_str()),
    };
    let digits: &str = &rest[..rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len())];
    if digits.is_empty() {
        return None;
    }
    if negative {
        return Some(0);
    }
    Some(digits.parse().unwrap_or(usize::MAX))
}

#[cfg(test)]
mod tests;
