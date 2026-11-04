//! OAuth 1.0 signing primitives: encoding, the base URL, the signing key, body hashes and
//! signatures, matching the reference signer's output.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use hmac::{Hmac, KeyInit, Mac};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use rsa::{Pkcs1v15Sign, RsaPrivateKey};
use sha1::{Digest, Sha1};
use sha2::{Sha256, Sha512};
use std::fmt;
use url::{Url, form_urlencoded};

#[cfg(test)]
mod tests;

/// The `oauth_signature_method` values the signer supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SignatureMethod {
    HmacSha1,
    HmacSha256,
    HmacSha512,
    RsaSha1,
    RsaSha256,
    RsaSha512,
    Plaintext,
}

impl SignatureMethod {
    /// The method's wire name, as sent in `oauth_signature_method`.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::HmacSha1 => "HMAC-SHA1",
            Self::HmacSha256 => "HMAC-SHA256",
            Self::HmacSha512 => "HMAC-SHA512",
            Self::RsaSha1 => "RSA-SHA1",
            Self::RsaSha256 => "RSA-SHA256",
            Self::RsaSha512 => "RSA-SHA512",
            Self::Plaintext => "PLAINTEXT",
        }
    }

    /// Whether the method signs with an RSA private key.
    pub(crate) fn is_rsa(self) -> bool {
        matches!(self, Self::RsaSha1 | Self::RsaSha256 | Self::RsaSha512)
    }
}

/// RFC 3986 unreserved characters stay literal; everything else is percent-encoded.
const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

const ALPHABET: &[u8; 62] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// 32 characters of [a-zA-Z0-9] from the OS RNG (byte % 62, as the reference).
pub(crate) fn nonce() -> Result<String, String> {
    use rand::{TryRngCore, rngs::OsRng};
    let mut bytes = [0u8; 32];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|e| format!("cannot generate a nonce: {e}"))?;
    Ok(bytes
        .iter()
        .map(|b| char::from(ALPHABET[usize::from(b % 62)]))
        .collect())
}

/// Percent-encodes `s` per RFC 3986, keeping only unreserved characters.
fn encode(s: &str) -> String {
    utf8_percent_encode(s, UNRESERVED).to_string()
}

/// `scheme://host[:port]/path`: lowercase scheme and host, a default port dropped, no query or fragment.
fn base_url(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default();
    match url.port() {
        Some(port) => format!("{}://{host}:{port}{}", url.scheme(), url.path()),
        None => format!("{}://{host}{}", url.scheme(), url.path()),
    }
}

/// The HMAC and PLAINTEXT key: both secrets encoded and joined by `&`.
fn signing_key(consumer_secret: &str, token_secret: &str) -> String {
    format!("{}&{}", encode(consumer_secret), encode(token_secret))
}

/// The base64 `oauth_body_hash`: SHA-256 or SHA-512 when the method names one, else SHA-1.
fn body_hash(body: &[u8], method: SignatureMethod) -> String {
    let digest = match method {
        SignatureMethod::HmacSha256 | SignatureMethod::RsaSha256 => Sha256::digest(body).to_vec(),
        SignatureMethod::HmacSha512 | SignatureMethod::RsaSha512 => Sha512::digest(body).to_vec(),
        _ => Sha1::digest(body).to_vec(),
    };
    STANDARD.encode(digest)
}

/// The base64 signature, or for PLAINTEXT the key itself; an RSA method without `key` is an error.
fn signature(
    method: SignatureMethod,
    key: Option<&RsaPrivateKey>,
    signing_key: &str,
    base: &str,
) -> Result<String, String> {
    let (k, b) = (signing_key.as_bytes(), base.as_bytes());
    let bytes = match method {
        SignatureMethod::Plaintext => return Ok(signing_key.to_owned()),
        SignatureMethod::HmacSha1 => hmac::<Hmac<Sha1>>(k, b)?,
        SignatureMethod::HmacSha256 => hmac::<Hmac<Sha256>>(k, b)?,
        SignatureMethod::HmacSha512 => hmac::<Hmac<Sha512>>(k, b)?,
        SignatureMethod::RsaSha1 | SignatureMethod::RsaSha256 | SignatureMethod::RsaSha512 => {
            let key = key.ok_or_else(|| format!("{} needs a private key", method.name()))?;
            rsa_sign(key, method, b).map_err(|e| format!("RSA signing failed: {e}"))?
        }
    };
    Ok(STANDARD.encode(bytes))
}

fn hmac<M: Mac + KeyInit>(key: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    let mut mac =
        <M as KeyInit>::new_from_slice(key).map_err(|e| format!("HMAC key rejected: {e}"))?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().to_vec())
}

pub(crate) const SHA1_PREFIX: [u8; 15] = [
    0x30, 0x21, 0x30, 0x09, 0x06, 0x05, 0x2b, 0x0e, 0x03, 0x02, 0x1a, 0x05, 0x00, 0x04, 0x14,
];

fn rsa_sign(key: &RsaPrivateKey, method: SignatureMethod, base: &[u8]) -> rsa::Result<Vec<u8>> {
    // rsa re-exports sha2 0.10 but not sha1, so SHA-1's DigestInfo prefix is spelled out.
    let (padding, digest) = match method {
        SignatureMethod::RsaSha1 => (
            Pkcs1v15Sign {
                hash_len: Some(20),
                prefix: SHA1_PREFIX.into(),
            },
            Sha1::digest(base).to_vec(),
        ),
        SignatureMethod::RsaSha256 => (
            Pkcs1v15Sign::new::<rsa::sha2::Sha256>(),
            Sha256::digest(base).to_vec(),
        ),
        _ => (
            Pkcs1v15Sign::new::<rsa::sha2::Sha512>(),
            Sha512::digest(base).to_vec(),
        ),
    };
    key.sign_with_rng(&mut rsa::rand_core::OsRng, padding, &digest)
}

/// Where the protocol parameters travel: the `Authorization` header, the query or a form body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Placement {
    Header,
    Query,
    Body,
}

/// One resolved OAuth 1.0 configuration, ready to sign any hop.
#[derive(Clone, PartialEq)]
pub(crate) struct Signer {
    pub(crate) consumer_key: String,
    pub(crate) consumer_secret: String,
    pub(crate) token: String,
    pub(crate) token_secret: String,
    pub(crate) callback: String,
    pub(crate) verifier: String,
    pub(crate) method: SignatureMethod,
    pub(crate) key: Option<RsaPrivateKey>,
    /// Blank means the `now` given to `sign`.
    pub(crate) timestamp: String,
    /// Blank means the nonce given to `sign`.
    pub(crate) nonce: String,
    /// Blank means `1.0`.
    pub(crate) version: String,
    pub(crate) realm: String,
    pub(crate) placement: Placement,
    pub(crate) include_body_hash: bool,
}

// Credentials and the private key stay out of logs.
impl fmt::Debug for Signer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Signer")
            .field("method", &self.method)
            .field("placement", &self.placement)
            .finish_non_exhaustive()
    }
}

/// The parts of one outgoing request that signing reads.
pub(crate) struct Hop<'a> {
    pub(crate) method: &'a str,
    pub(crate) url: &'a Url,
    pub(crate) content_type: Option<&'a str>,
    pub(crate) body: &'a [u8],
}

/// A signed hop: its URL and body, the header to set, a replacement `Content-Type`, and the
/// `oauth_signature` value placed in one of them.
#[derive(Debug, PartialEq)]
pub(crate) struct Signed {
    pub(crate) signature: String,
    pub(crate) url: Url,
    pub(crate) body: Vec<u8>,
    pub(crate) authorization: Option<String>,
    pub(crate) content_type: Option<&'static str>,
}

const FORM: &str = "application/x-www-form-urlencoded";

type Pairs = Vec<(String, String)>;

/// The media type before any `;`, trimmed and lowercased.
fn media_type(content_type: Option<&str>) -> String {
    let ct = content_type.unwrap_or_default();
    ct.split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

/// Sets `key` like `URLSearchParams.set`: the first match is replaced in place and later
/// duplicates dropped, else the pair is appended.
fn set(pairs: &mut Pairs, key: &str, value: &str) {
    match pairs.iter().position(|(k, _)| k == key) {
        Some(at) => {
            pairs[at].1 = value.to_owned();
            let mut i = 0;
            pairs.retain(|(k, _)| {
                i += 1;
                i - 1 == at || k != key
            });
        }
        None => pairs.push((key.to_owned(), value.to_owned())),
    }
}

fn parse_form(bytes: &[u8]) -> Pairs {
    form_urlencoded::parse(bytes).into_owned().collect()
}

fn serialize_form<'a>(pairs: impl IntoIterator<Item = &'a (String, String)>) -> String {
    form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish()
}

impl Signer {
    /// The HMAC and PLAINTEXT key; the token secret counts only beside a token.
    fn key(&self) -> String {
        let secret = if self.token.is_empty() {
            ""
        } else {
            &self.token_secret
        };
        signing_key(&self.consumer_secret, secret)
    }

    /// Each secret as it can appear on the wire once encoded, plus the PLAINTEXT signature in
    /// its header and form encodings, so redaction by substring finds them.
    pub(crate) fn sensitive_forms(&self) -> Vec<String> {
        let form = |s: &str| form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
        let mut forms = Vec::new();
        for secret in [&self.consumer_secret, &self.token_secret] {
            let encoded = encode(secret);
            forms.extend([form(&encoded), encode(&encoded), encoded]);
        }
        // With no secret in it the signature is a bare `&`, which would only redact punctuation.
        let signature = self.key();
        if self.method == SignatureMethod::Plaintext && signature != "&" {
            forms.extend([form(&signature), encode(&signature), signature]);
        }
        forms
    }

    /// Signs `hop` and places the parameters; `now` (Unix seconds) and `nonce` stand in for a
    /// blank timestamp or nonce. Fails only when an RSA method has no key or signing fails.
    pub(crate) fn sign(&self, hop: &Hop<'_>, now: u64, nonce: &str) -> Result<Signed, String> {
        let (mut params, extra) = self.parameters(hop, now, nonce);
        let base = base_string(hop, &params, &extra);
        let key = self.key();
        let sig = signature(self.method, self.key.as_ref(), &key, &base)?;
        set(&mut params, "oauth_signature", &sig);

        let mut signed = Signed {
            signature: sig.clone(),
            url: hop.url.clone(),
            body: hop.body.to_vec(),
            authorization: None,
            content_type: None,
        };
        let is_form = media_type(hop.content_type) == FORM;
        match self.placement {
            Placement::Header => signed.authorization = Some(self.header(&params)),
            Placement::Query => {
                let mut query = parse_form(hop.url.query().unwrap_or_default().as_bytes());
                for (k, v) in params.iter().filter(|(_, v)| !v.is_empty()) {
                    set(&mut query, k, v);
                }
                let text = serialize_form(&query);
                signed
                    .url
                    .set_query((!text.is_empty()).then_some(text.as_str()));
            }
            Placement::Body => {
                // A non-form body is replaced by the parameters alone, as a form.
                let mut form = if is_form {
                    parse_form(hop.body)
                } else {
                    Vec::new()
                };
                for (k, v) in &params {
                    set(&mut form, k, v);
                }
                signed.body = serialize_form(&form).into_bytes();
                signed.content_type = (!is_form).then_some(FORM);
            }
        }
        Ok(signed)
    }

    /// The signature base string for `hop`, before any signature exists.
    #[cfg(test)]
    fn base_string(&self, hop: &Hop<'_>, now: u64, nonce: &str) -> String {
        let (params, extra) = self.parameters(hop, now, nonce);
        base_string(hop, &params, &extra)
    }

    /// The protocol parameters in the reference's insertion order, and the non-`oauth_` form
    /// body pairs that also enter the base string.
    fn parameters(&self, hop: &Hop<'_>, now: u64, nonce: &str) -> (Pairs, Pairs) {
        let or = |field: &str, fallback: String| {
            if field.is_empty() {
                fallback
            } else {
                field.to_owned()
            }
        };
        let mut params: Pairs = vec![
            ("oauth_consumer_key".into(), self.consumer_key.clone()),
            ("oauth_nonce".into(), or(&self.nonce, nonce.to_owned())),
            ("oauth_signature_method".into(), self.method.name().into()),
            (
                "oauth_timestamp".into(),
                or(&self.timestamp, now.to_string()),
            ),
            ("oauth_version".into(), or(&self.version, "1.0".into())),
        ];
        for (key, value) in [
            ("oauth_token", &self.token),
            ("oauth_callback", &self.callback),
            ("oauth_verifier", &self.verifier),
        ] {
            if !value.is_empty() {
                set(&mut params, key, value);
            }
        }

        // Form pairs sign unless a GET or HEAD would not send them; their `oauth_*` pairs join
        // the protocol parameters. Form and multipart bodies get no body hash.
        let media = media_type(hop.content_type);
        let is_form = media == FORM;
        let bodyless =
            hop.method.eq_ignore_ascii_case("GET") || hop.method.eq_ignore_ascii_case("HEAD");
        let mut extra = Vec::new();
        if is_form && !hop.body.is_empty() && (!bodyless || self.placement == Placement::Body) {
            for (k, v) in parse_form(hop.body) {
                if k.starts_with("oauth_") {
                    set(&mut params, &k, &v);
                } else {
                    extra.push((k, v));
                }
            }
        }
        if self.include_body_hash && !is_form && !media.starts_with("multipart/") {
            set(
                &mut params,
                "oauth_body_hash",
                &body_hash(hop.body, self.method),
            );
        }
        (params, extra)
    }

    /// `OAuth realm="..", k="v", ..`: the realm escapes only `\` and `"`, keys are sorted.
    fn header(&self, params: &Pairs) -> String {
        let mut header = String::from("OAuth ");
        if !self.realm.is_empty() {
            let realm = self.realm.replace('\\', "\\\\").replace('"', "\\\"");
            header.push_str(&format!("realm=\"{realm}\", "));
        }
        let mut sorted: Vec<_> = params
            .iter()
            .filter(|(k, _)| k.starts_with("oauth_"))
            .collect();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        let parts: Vec<_> = sorted
            .iter()
            .map(|(k, v)| format!("{}=\"{}\"", encode(k), encode(v)))
            .collect();
        header.push_str(&parts.join(", "));
        header
    }
}

/// `METHOD&base-url&params`: protocol, query (`+` literal) and body pairs encoded and sorted.
fn base_string(hop: &Hop<'_>, params: &Pairs, extra: &Pairs) -> String {
    let query = hop.url.query().unwrap_or_default().replace('+', "%2B");
    let mut all: Vec<(String, String)> = params
        .iter()
        .cloned()
        .chain(parse_form(query.as_bytes()))
        .chain(extra.iter().cloned())
        .map(|(k, v)| (encode(&k), encode(&v)))
        .collect();
    all.sort();
    let joined: Vec<_> = all.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!(
        "{}&{}&{}",
        hop.method.to_ascii_uppercase(),
        encode(&base_url(hop.url)),
        encode(&joined.join("&"))
    )
}

/// Drops `oauth_*` pairs from the query and, for a form body, from the body; untouched when none.
pub(crate) fn strip(url: &mut Url, content_type: Option<&str>, body: &mut Vec<u8>) {
    // Raw segments are kept byte for byte so that only the dropped pairs change.
    let is_oauth = |segment: &[u8]| {
        form_urlencoded::parse(segment)
            .next()
            .is_some_and(|(k, _)| k.starts_with("oauth_"))
    };
    if let Some(query) = url.query() {
        let segments: Vec<&str> = query.split('&').collect();
        if segments.iter().any(|s| is_oauth(s.as_bytes())) {
            let kept: Vec<&str> = segments
                .into_iter()
                .filter(|s| !is_oauth(s.as_bytes()))
                .collect();
            let kept = kept.join("&");
            url.set_query((!kept.is_empty()).then_some(kept.as_str()));
        }
    }
    if media_type(content_type) == FORM {
        let segments: Vec<&[u8]> = body.split(|b| *b == b'&').collect();
        if segments.iter().any(|s| is_oauth(s)) {
            *body = segments
                .into_iter()
                .filter(|s| !is_oauth(s))
                .collect::<Vec<_>>()
                .join(&b'&');
        }
    }
}
