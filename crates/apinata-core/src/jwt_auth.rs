//! Signs the token of a JWT auth from its interpolated fields.
//!
//! Every error names the field or the algorithm, never the payload, key or secret text.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use percent_encoding::percent_decode_str;
use serde_json::{Map, Value};

use crate::edit::JwtAlgorithm;
use crate::script::jwt::{KeyInput, check_auth_key, sign};

/// What signs the token: the secret's bytes for HS, the PEM text for the others.
pub(crate) enum Key {
    Secret(Vec<u8>),
    Pem(String),
}

/// The token's parts after variables are filled in.
pub(crate) struct Claims<'a> {
    pub(crate) algorithm: &'a str,
    pub(crate) headers: &'a str,
    pub(crate) payload: &'a str,
    pub(crate) expires_in: Option<u64>,
}

/// A JSON object from `text`; blank text is the empty object.
fn object(text: &str, what: &str) -> Result<Map<String, Value>, String> {
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str(text) {
        Ok(Value::Object(map)) => Ok(map),
        _ => Err(format!("the {what} is not a JSON object")),
    }
}

/// The signed token. The header carries `alg` first, then the other header keys; an expiry
/// adds `iat` (`now`) and `exp` only to a payload that has neither.
pub(crate) fn token(claims: &Claims<'_>, key: &Key, now: u64) -> Result<String, String> {
    let alg = claims.algorithm;
    if matches!(JwtAlgorithm::parse(alg), JwtAlgorithm::Other(_)) {
        // The name may come from a variable, so the error names the field, never its value.
        return Err("the algorithm is not one of HS, RS, PS or ES with 256, 384 or 512".to_owned());
    }
    let mut header = Map::new();
    header.insert("alg".to_owned(), Value::from(alg));
    for (name, value) in object(claims.headers, "headers")? {
        if name != "alg" {
            header.insert(name, value);
        }
    }
    let mut payload = object(claims.payload, "payload")?;
    if let Some(seconds) = claims.expires_in
        && !payload.contains_key("iat")
        && !payload.contains_key("exp")
    {
        payload.insert("iat".to_owned(), Value::from(now));
        payload.insert("exp".to_owned(), Value::from(now.saturating_add(seconds)));
    }
    let input = match key {
        Key::Secret(bytes) if bytes.is_empty() => return Err(format!("{alg} needs a secret")),
        Key::Pem(pem) if pem.trim().is_empty() => {
            return Err(format!("{alg} needs a private key"));
        }
        Key::Secret(bytes) => KeyInput::Bytes(bytes.clone()),
        Key::Pem(pem) => KeyInput::Text(pem.clone()),
    };
    check_auth_key(alg, &input)?;
    let header = Value::Object(header).to_string();
    let payload = Value::Object(payload).to_string();
    sign(alg, &header, &payload, &input).map_err(|_| format!("the key cannot be used with {alg}"))
}

/// The fields of an ASAP token after variables are filled in.
pub(crate) struct Asap<'a> {
    pub(crate) algorithm: &'a str,
    pub(crate) issuer: &'a str,
    pub(crate) audience: &'a str,
    pub(crate) key_id: &'a str,
    pub(crate) subject: &'a str,
    pub(crate) expires_in: &'a str,
    pub(crate) additional_claims: &'a str,
    pub(crate) private_key: &'a str,
}

/// A signed ASAP token and its key as the PEM text it was signed with.
pub(crate) struct AsapSigned {
    pub(crate) token: String,
    pub(crate) pem: String,
}

/// Signs an ASAP token: the header is `alg` then `kid`, the claims `iss`, `sub` (the issuer
/// when blank), `aud`, `iat` (`now`), `exp` (an hour unless set) and `jti`, and any
/// additional claim then replaces or extends them. Errors name the field, never a value.
pub(crate) fn asap_token(f: &Asap<'_>, now: u64, jti: &str) -> Result<AsapSigned, String> {
    let alg = f.algorithm;
    let algorithm = JwtAlgorithm::parse(alg);
    if matches!(algorithm, JwtAlgorithm::Other(_)) || algorithm.uses_secret() {
        return Err("the algorithm is not one of RS, PS or ES with 256, 384 or 512".to_owned());
    }
    for (name, value) in [("issuer", f.issuer), ("audience", f.audience)] {
        if value.trim().is_empty() {
            return Err(format!("{name} is required"));
        }
    }
    if f.private_key.trim().is_empty() {
        return Err("the private key is blank".to_owned());
    }
    let (pem, uri_kid) = asap_pem(f.private_key)?;
    let kid = match (f.key_id.trim(), uri_kid) {
        ("", Some(kid)) => kid,
        ("", None) => return Err("keyId is required".to_owned()),
        (given, Some(kid)) if given != kid => {
            return Err("keyId does not match the key id of the data URI key".to_owned());
        }
        (given, _) => given.to_owned(),
    };
    let seconds = match f.expires_in.trim() {
        "" => 3600,
        n => n
            .parse::<u64>()
            .map_err(|_| "expiresIn is not a whole number of seconds".to_owned())?,
    };
    let subject = if f.subject.trim().is_empty() {
        f.issuer
    } else {
        f.subject
    };
    let mut payload = Map::new();
    payload.insert("iss".to_owned(), Value::from(f.issuer));
    payload.insert("sub".to_owned(), Value::from(subject));
    payload.insert("aud".to_owned(), Value::from(f.audience));
    payload.insert("iat".to_owned(), Value::from(now));
    payload.insert("exp".to_owned(), Value::from(now.saturating_add(seconds)));
    payload.insert("jti".to_owned(), Value::from(jti));
    let extra = object(f.additional_claims, "additionalClaims")
        .map_err(|_| "additionalClaims is not a JSON object".to_owned())?;
    payload.extend(extra);
    let mut header = Map::new();
    header.insert("kid".to_owned(), Value::from(kid));
    let claims = Claims {
        algorithm: alg,
        headers: &Value::Object(header).to_string(),
        payload: &Value::Object(payload).to_string(),
        expires_in: None,
    };
    let token = token(&claims, &Key::Pem(pem.clone()), now)?;
    Ok(AsapSigned { token, pem })
}

/// The key as PEM, with the key id a data URI names. PEM text is kept (a literal `\n` read as
/// a line break); a `data:application/pkcs8;kid=..;base64,..` URI or bare base64 is PKCS#8 DER.
fn asap_pem(key: &str) -> Result<(String, Option<String>), String> {
    let bad =
        || "the private key is not PEM, a data:application/pkcs8 URI or base64 DER".to_owned();
    let key = key.replace("\\n", "\n");
    let key = key.trim();
    if key.contains("-----BEGIN") {
        return Ok((key.to_owned(), None));
    }
    let (kid, data) = match key.strip_prefix("data:") {
        Some(rest) => {
            let (meta, data) = rest.split_once(',').ok_or_else(bad)?;
            let mut parts = meta.split(';');
            if parts.next() != Some("application/pkcs8") {
                return Err(bad());
            }
            let (mut kid, mut base64) = (None, false);
            for part in parts {
                match part.strip_prefix("kid=") {
                    Some(id) => {
                        let id = percent_decode_str(id).decode_utf8().map_err(|_| bad())?;
                        kid = Some(id.into_owned());
                    }
                    None => base64 |= part == "base64",
                }
            }
            if !base64 {
                return Err(bad());
            }
            (kid, data)
        }
        None => (None, key),
    };
    let compact: String = data.chars().filter(|c| !c.is_whitespace()).collect();
    let der = STANDARD.decode(compact).map_err(|_| bad())?;
    let body = STANDARD.encode(der);
    let lines: Vec<&str> = body
        .as_bytes()
        .chunks(64)
        .filter_map(|c| std::str::from_utf8(c).ok())
        .collect();
    let pem = format!(
        "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
        lines.join("\n")
    );
    Ok((pem, kid))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::jwt::{decode, verify};

    const RSA: &str = include_str!("../tests/fixtures/jwt/rsa.pem");
    const RSA_PKCS1: &str = include_str!("../tests/fixtures/jwt/rsa-pkcs1.pem");
    const RSA_PUB: &str = include_str!("../tests/fixtures/jwt/rsa.pub.pem");
    const EC256_SEC1: &str = include_str!("../tests/fixtures/jwt/ec256-sec1.pem");
    const EC256_PUB: &str = include_str!("../tests/fixtures/jwt/ec256.pub.pem");
    const EC521: &str = include_str!("../tests/fixtures/jwt/ec521.pem");
    const EC521_PUB: &str = include_str!("../tests/fixtures/jwt/ec521.pub.pem");

    fn claims<'a>(algorithm: &'a str, headers: &'a str, payload: &'a str) -> Claims<'a> {
        Claims {
            algorithm,
            headers,
            payload,
            expires_in: None,
        }
    }

    fn parts(token: &str) -> (Value, Value) {
        let (h, p, _) = decode(token).expect("a token");
        (
            serde_json::from_str(&h).unwrap(),
            serde_json::from_str(&p).unwrap(),
        )
    }

    fn pem(text: &str) -> Key {
        Key::Pem(text.to_owned())
    }

    #[test]
    fn hs256_verifies_with_its_secret() {
        let key = Key::Secret(b"s3cr3t".to_vec());
        let t = token(&claims("HS256", "", "{\"sub\":\"1\"}"), &key, 0).unwrap();
        let secret = KeyInput::Text("s3cr3t".into());
        assert_eq!(verify(&t, "HS256", &secret), Ok(true));
        let (header, payload) = parts(&t);
        assert_eq!(header.to_string(), "{\"alg\":\"HS256\"}");
        assert_eq!(payload.to_string(), "{\"sub\":\"1\"}");
    }

    #[test]
    fn rs256_verifies_with_the_public_key_for_pkcs8_and_pkcs1() {
        for key in [RSA, RSA_PKCS1] {
            let t = token(&claims("RS256", "", "{}"), &pem(key), 0).unwrap();
            assert_eq!(
                verify(&t, "RS256", &KeyInput::Text(RSA_PUB.into())),
                Ok(true)
            );
        }
    }

    #[test]
    fn es256_from_sec1_and_es512_verify() {
        let t = token(&claims("ES256", "", "{}"), &pem(EC256_SEC1), 0).unwrap();
        assert_eq!(
            verify(&t, "ES256", &KeyInput::Text(EC256_PUB.into())),
            Ok(true)
        );
        let t = token(&claims("ES512", "", "{}"), &pem(EC521), 0).unwrap();
        assert_eq!(
            verify(&t, "ES512", &KeyInput::Text(EC521_PUB.into())),
            Ok(true)
        );
    }

    #[test]
    fn alg_comes_first_and_from_the_algorithm_over_a_header_override() {
        let key = Key::Secret(b"k".to_vec());
        let t = token(
            &claims(
                "HS384",
                "{\"kid\":\"k1\",\"alg\":\"none\",\"typ\":\"JWT\"}",
                "{}",
            ),
            &key,
            0,
        )
        .unwrap();
        let (header, _) = parts(&t);
        assert_eq!(
            header.to_string(),
            "{\"alg\":\"HS384\",\"kid\":\"k1\",\"typ\":\"JWT\"}"
        );
    }

    #[test]
    fn expiry_adds_iat_and_exp_only_when_both_are_absent() {
        let key = Key::Secret(b"k".to_vec());
        let mut c = claims("HS256", "", "{\"sub\":\"1\"}");
        c.expires_in = Some(300);
        let (_, p) = parts(&token(&c, &key, 1_000).unwrap());
        assert_eq!(p.to_string(), "{\"sub\":\"1\",\"iat\":1000,\"exp\":1300}");
        c.payload = "{\"exp\":5}";
        let (_, p) = parts(&token(&c, &key, 1_000).unwrap());
        assert_eq!(p.to_string(), "{\"exp\":5}");
        c.payload = "{\"iat\":7}";
        let (_, p) = parts(&token(&c, &key, 1_000).unwrap());
        assert_eq!(p.to_string(), "{\"iat\":7}");
    }

    #[test]
    fn refusals_never_echo_the_payload_key_or_secret() {
        let secret = Key::Secret(b"topsecret".to_vec());
        let cases = [
            (
                claims("HS256", "", "[\"leak-payload\"]"),
                &secret,
                "payload",
            ),
            (claims("HS256", "\"leak-head\"", "{}"), &secret, "headers"),
            (claims("XS1", "", "{}"), &secret, "algorithm is not one of"),
            (
                claims("HS256", "", "{}"),
                &Key::Secret(Vec::new()),
                "needs a secret",
            ),
            (claims("RS256", "", "{}"), &pem(" "), "needs a private key"),
            (
                claims("RS256", "", "{}"),
                &pem(EC256_SEC1),
                "does not match",
            ),
            (claims("ES256", "", "{}"), &pem(RSA), "does not match"),
            (
                claims("ES384", "", "{}"),
                &pem(EC256_SEC1),
                "does not match",
            ),
            (
                claims("PS256", "", "{}"),
                &pem("leak-not-a-key"),
                "not a PEM private key",
            ),
            (
                claims("RS256", "", "{}"),
                &pem(RSA_PUB),
                "not a PEM private key",
            ),
        ];
        for (c, key, want) in cases {
            let err = token(&c, key, 0).expect_err(want);
            assert!(err.contains(want), "{want}: {err}");
            for leak in ["leak", "topsecret", "BEGIN", "XS1"] {
                assert!(!err.contains(leak), "{err}");
            }
        }
    }
}
