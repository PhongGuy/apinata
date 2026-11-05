//! JSON Web Token signing, verification and decoding for the script `jsonwebtoken` module.
//!
//! The JS shim owns options, claims and reading keys through `crypto`; this module owns base64url
//! and hands asymmetric signatures to `crypto.rs`, so both modules hold one set of RSA and EC rules.
//! Error strings name the algorithm or the problem, never the key or token text.

use aws_lc_rs::signature::{
    ECDSA_P256_SHA256_FIXED_SIGNING, ECDSA_P384_SHA384_FIXED_SIGNING,
    ECDSA_P521_SHA512_FIXED_SIGNING, EcdsaKeyPair, EcdsaSigningAlgorithm,
};
use base64::Engine;
use base64::alphabet::URL_SAFE;
use base64::engine::DecodePaddingMode;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig, URL_SAFE_NO_PAD};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, crypto};
use rquickjs::Function;
use serde_json::{Value, json};

use super::crypto::{Curve, Public, public_key, rsa_public_parts};
use super::crypto_pss::Scheme;
use super::engine::Installer;

const JWT_JS: &str = include_str!("jwt.js");

/// Installs `__jwt` and the script that wraps it into `require("jsonwebtoken")` and `jwt`.
pub(crate) fn installer() -> Installer {
    Box::new(|ctx| {
        let host = |op: String, args: String| dispatch(&op, &args).to_string();
        ctx.globals()
            .set("__jwt", Function::new(ctx.clone(), host)?)?;
        ctx.eval::<(), _>(JWT_JS)
    })
}

/// Runs `op` over the JSON array `args`; the answer is `{"v": value}` or `{"e": message}`.
fn dispatch(op: &str, args: &str) -> Value {
    let args: Vec<Value> = match serde_json::from_str(args) {
        Ok(args) => args,
        Err(e) => return json!({ "e": format!("invalid arguments for {op}: {e}") }),
    };
    let text = |i: usize| args.get(i).and_then(Value::as_str).unwrap_or_default();
    let key = |i: usize| match args.get(i) {
        Some(Value::Array(bytes)) => KeyInput::Bytes(
            bytes
                .iter()
                .map(|b| b.as_u64().and_then(|b| u8::try_from(b).ok()).unwrap_or(0))
                .collect(),
        ),
        _ => KeyInput::Text(text(i).to_owned()),
    };
    let answer = match op {
        "details" => details(&key(0)),
        "sign" => sign(text(0), text(1), text(2), &key(3)).map(Value::from),
        "verify" => verify(text(0), text(1), &key(2)).map(Value::from),
        "decode" => Ok(decode(text(0)).map_or(Value::Null, |(h, p, s)| json!([h, p, s]))),
        _ => Err(format!("unknown jwt operation {op}")),
    };
    match answer {
        Ok(v) => json!({ "v": v }),
        Err(e) => json!({ "e": e }),
    }
}

/// A key as the script passed it: a string or the bytes of a `Buffer`.
pub(crate) enum KeyInput {
    Text(String),
    Bytes(Vec<u8>),
}

/// Decodes token parts as Node's `Buffer.from(part, 'base64')` would for base64url text.
const LENIENT: GeneralPurpose = GeneralPurpose::new(
    &URL_SAFE,
    GeneralPurposeConfig::new()
        .with_decode_padding_mode(DecodePaddingMode::Indifferent)
        .with_decode_allow_trailing_bits(true),
);

impl KeyInput {
    pub(crate) fn bytes(&self) -> &[u8] {
        match self {
            KeyInput::Text(s) => s.as_bytes(),
            KeyInput::Bytes(b) => b,
        }
    }
}

/// The curve of an EC private key DER (SEC1 or PKCS#8), tried against each supported curve.
pub(crate) fn ec_private_curve(der: &[u8], sec1: bool) -> Option<&'static str> {
    let curves: [(&'static EcdsaSigningAlgorithm, &str); 3] = [
        (&ECDSA_P256_SHA256_FIXED_SIGNING, "prime256v1"),
        (&ECDSA_P384_SHA384_FIXED_SIGNING, "secp384r1"),
        (&ECDSA_P521_SHA512_FIXED_SIGNING, "secp521r1"),
    ];
    curves.into_iter().find_map(|(alg, name)| {
        let parsed = if sec1 {
            EcdsaKeyPair::from_private_key_der(alg, der).is_ok()
        } else {
            EcdsaKeyPair::from_pkcs8(alg, der).is_ok()
        };
        parsed.then_some(name)
    })
}

/// What the shim's key checks read of an opened key: an EC key's curve in Node's spelling, an RSA
/// key's modulus size.
fn details(key: &KeyInput) -> Result<Value, String> {
    Ok(match public_key(key)? {
        Public::Ec(curve, _) => json!({ "curve": curve_name(curve) }),
        Public::Rsa(der) | Public::RsaPss { der, .. } => {
            json!({ "bits": rsa_public_parts(&der).map(|(n, _)| n.bits()) })
        }
        Public::Ed(_) => json!({}),
    })
}

fn curve_name(curve: Curve) -> &'static str {
    match curve {
        Curve::P256 => "prime256v1",
        Curve::P384 => "secp384r1",
        Curve::P521 => "secp521r1",
    }
}

/// The crate algorithm for a supported name.
fn algorithm(alg: &str) -> Result<Algorithm, String> {
    let algorithm = match alg {
        "HS256" => Algorithm::HS256,
        "HS384" => Algorithm::HS384,
        "HS512" => Algorithm::HS512,
        "RS256" => Algorithm::RS256,
        "RS384" => Algorithm::RS384,
        "RS512" => Algorithm::RS512,
        "PS256" => Algorithm::PS256,
        "PS384" => Algorithm::PS384,
        "PS512" => Algorithm::PS512,
        "ES256" => Algorithm::ES256,
        "ES384" => Algorithm::ES384,
        // The crate has no ES512; the value only matters for HMAC, ECDSA signs through crypto.rs.
        "ES512" => Algorithm::ES384,
        _ => return Err(format!("{alg} is not supported by Apinata")),
    };
    Ok(algorithm)
}

fn unusable(alg: &str) -> String {
    format!("the key cannot be used with {alg}")
}

/// Node's answer when the type check is off and an Ed25519 key meets a digest it does not take.
const NO_OPERATION: &str = "Unsupported crypto operation";
/// What an RSA signature fails with where a JOSE ECDSA signature is formatted.
const NOT_ECDSA: &str = "Could not find expected \"seq\"";

/// How `crypto.rs` signs a family: its digest, ECDSA encoding and RSA options. RS names no
/// padding, so an RSA-PSS key the type check let past signs PSS, as in Node.
fn scheme(alg: &str) -> (&'static str, &'static str, Scheme) {
    let digest = match &alg[2..] {
        "256" => "sha256",
        "384" => "sha384",
        _ => "sha512",
    };
    match &alg[..2] {
        // The library signs with the salt as long as the digest.
        "PS" => (
            digest,
            "der",
            Scheme {
                padding: Some(6),
                salt: Some(-1),
            },
        ),
        "ES" => (digest, "ieee-p1363", Scheme::default()),
        _ => (digest, "der", Scheme::default()),
    }
}

/// Holds a key the option let past the type check to what the JOSE encoding can carry. As in
/// Node, an EC key still signs RS and PS; the fixed-size ES forms need their own curve here.
fn check_signing_key(alg: &str, key: &KeyInput) -> Result<(), String> {
    match (&alg[..2], public_key(key)?) {
        (_, Public::Ed(_)) => Err(NO_OPERATION.into()),
        ("ES", Public::Rsa(_) | Public::RsaPss { .. }) => Err(NOT_ECDSA.into()),
        ("ES", Public::Ec(curve, _)) => {
            let wanted = match alg {
                "ES256" => Curve::P256,
                "ES384" => Curve::P384,
                _ => Curve::P521,
            };
            if curve == wanted {
                Ok(())
            } else {
                Err(unusable(alg))
            }
        }
        _ => Ok(()),
    }
}

/// base64url(header) "." base64url(payload) "." signature; both texts are encoded as given.
pub(crate) fn sign(
    alg: &str,
    header: &str,
    payload: &str,
    key: &KeyInput,
) -> Result<String, String> {
    let algorithm = algorithm(alg)?;
    let input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header),
        URL_SAFE_NO_PAD.encode(payload)
    );
    if alg.starts_with("HS") {
        let key = EncodingKey::from_secret(key.bytes());
        let signature =
            crypto::sign(input.as_bytes(), &key, algorithm).map_err(|_| unusable(alg))?;
        return Ok(format!("{input}.{signature}"));
    }
    check_signing_key(alg, key)?;
    let (digest, dsa, padding) = scheme(alg);
    let signature = super::crypto::sign(digest, input.as_bytes(), key, dsa, padding)?;
    Ok(format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature)))
}

/// Ok(false) for a wrong signature, Err for a key that cannot be used with `alg`.
pub(crate) fn verify(token: &str, alg: &str, key: &KeyInput) -> Result<bool, String> {
    let algorithm = algorithm(alg)?;
    let hmac = alg.starts_with("HS");
    if !hmac && matches!(public_key(key)?, Public::Ed(_)) {
        return Err(NO_OPERATION.into());
    }
    let Some((input, signature)) = token.rsplit_once('.') else {
        return Ok(false);
    };
    // A signature that is not base64url cannot match; the crate would call it an error.
    if hmac {
        if URL_SAFE_NO_PAD.decode(signature).is_err() {
            return Ok(false);
        }
        let key = DecodingKey::from_secret(key.bytes());
        return crypto::verify(signature, input.as_bytes(), &key, algorithm)
            .map_err(|_| unusable(alg));
    }
    // Spare trailing bits are ignored as Node ignores them, where HMAC compares the text. Other
    // spellings Node's decoder forgives fail here, which only refuses.
    let Ok(raw) = LENIENT.decode(signature) else {
        return Ok(false);
    };
    let (digest, dsa, padding) = scheme(alg);
    super::crypto::verify(digest, input.as_bytes(), key, &raw, dsa, padding)
}

/// The decoded header text, payload text and raw signature part; None unless the token matches
/// `^[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]*$` and both parts decode.
pub(crate) fn decode(token: &str) -> Option<(String, String, String)> {
    let mut parts = token.split('.');
    let (header, payload, signature) = (parts.next()?, parts.next()?, parts.next()?);
    let base64url = |s: &str| {
        s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    };
    let shaped = parts.next().is_none()
        && !header.is_empty()
        && !payload.is_empty()
        && [header, payload, signature].into_iter().all(base64url);
    if !shaped {
        return None;
    }
    let text = |s: &str| {
        LENIENT
            .decode(s)
            .ok()
            .map(|b| String::from_utf8_lossy(&b).into_owned())
    };
    Some((text(header)?, text(payload)?, signature.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RSA: &str = include_str!("../../tests/fixtures/jwt/rsa.pem");
    const RSA_PKCS1: &str = include_str!("../../tests/fixtures/jwt/rsa-pkcs1.pem");
    const RSA_PUB: &str = include_str!("../../tests/fixtures/jwt/rsa.pub.pem");
    const RSA_CRT: &str = include_str!("../../tests/fixtures/jwt/rsa.crt");
    const EC256_SEC1: &str = include_str!("../../tests/fixtures/jwt/ec256-sec1.pem");
    const EC256_PUB: &str = include_str!("../../tests/fixtures/jwt/ec256.pub.pem");
    const EC384: &str = include_str!("../../tests/fixtures/jwt/ec384.pem");
    const EC384_PUB: &str = include_str!("../../tests/fixtures/jwt/ec384.pub.pem");
    const VECTORS: &str = include_str!("../../tests/fixtures/jwt/vectors.json");
    const PAYLOAD: &str = "{\"sub\":\"1234567890\",\"name\":\"J\u{f6}rg \u{2713}\",\"admin\":true,\"iat\":1700000000}";

    fn vector(name: &str) -> String {
        let v: serde_json::Value = serde_json::from_str(VECTORS).expect("vectors");
        v[name].as_str().expect(name).to_owned()
    }
    fn text(s: &str) -> KeyInput {
        KeyInput::Text(s.to_owned())
    }
    fn header(alg: &str) -> String {
        format!("{{\"alg\":\"{alg}\",\"typ\":\"JWT\"}}")
    }

    #[test]
    fn hs_and_rs_tokens_match_node_byte_for_byte() {
        for alg in ["HS256", "HS384", "HS512"] {
            let token = sign(alg, &header(alg), PAYLOAD, &text("s3cr3t-key"));
            assert_eq!(token, Ok(vector(alg)), "{alg}");
        }
        let bytes = KeyInput::Bytes((0u8..32).collect());
        assert_eq!(
            sign("HS256", &header("HS256"), PAYLOAD, &bytes),
            Ok(vector("HS256_buffer"))
        );
        for alg in ["RS256", "RS384", "RS512"] {
            assert_eq!(
                sign(alg, &header(alg), PAYLOAD, &text(RSA)),
                Ok(vector(alg)),
                "{alg}"
            );
        }
        let pkcs1 = sign("RS256", &header("RS256"), PAYLOAD, &text(RSA_PKCS1));
        assert_eq!(pkcs1, Ok(vector("RS256_pkcs1")));
    }

    #[test]
    fn node_tokens_verify_with_public_keys_and_certificates() {
        let cases = [
            ("HS256", "s3cr3t-key"),
            ("RS256", RSA_PUB),
            ("RS384", RSA_CRT),
            ("RS512", RSA_PUB),
            ("PS256", RSA_CRT),
            ("PS384", RSA_PUB),
            ("PS512", RSA_PUB),
            ("ES256", EC256_PUB),
            ("ES384", EC384_PUB),
        ];
        for (alg, key) in cases {
            assert_eq!(verify(&vector(alg), alg, &text(key)), Ok(true), "{alg}");
        }
        assert_eq!(verify(&vector("HS256"), "HS256", &text("wrong")), Ok(false));
        assert_ne!(
            verify(&vector("RS256"), "RS256", &text(EC256_PUB)),
            Ok(true)
        );
    }

    #[test]
    fn ps_and_es_round_trip_including_a_sec1_key() {
        for alg in ["PS256", "PS384", "PS512"] {
            let token = sign(alg, &header(alg), PAYLOAD, &text(RSA)).expect(alg);
            assert_eq!(verify(&token, alg, &text(RSA_PUB)), Ok(true), "{alg}");
        }
        let es256 = sign("ES256", &header("ES256"), PAYLOAD, &text(EC256_SEC1)).expect("es256");
        assert_eq!(verify(&es256, "ES256", &text(EC256_PUB)), Ok(true));
        let es384 = sign("ES384", &header("ES384"), PAYLOAD, &text(EC384)).expect("es384");
        assert_eq!(verify(&es384, "ES384", &text(EC384_PUB)), Ok(true));
        // ECDSA signatures are randomised; the signing input is still Node's.
        let input = |t: &str| t.rsplit_once('.').map(|(i, _)| i.to_owned());
        assert_eq!(input(&es256), input(&vector("ES256")));
    }

    #[test]
    fn errors_never_echo_the_key_and_unsupported_algorithms_are_named() {
        let secret =
            "-----BEGIN PRIVATE KEY-----\nVE9QU0VDUkVUS0VZREFUQQ==\n-----END PRIVATE KEY-----\n";
        let err = sign("RS256", &header("RS256"), PAYLOAD, &text(secret)).expect_err("bad key");
        assert!(!err.contains("VE9QU0VD"), "{err}");
        if let Err(e) = verify(&vector("ES256"), "ES256", &text(secret)) {
            assert!(!e.contains("VE9QU0VD"), "{e}");
        }
        for alg in ["none", "EdDSA"] {
            let err = sign(alg, &header(alg), PAYLOAD, &text("k")).expect_err(alg);
            assert_eq!(err, format!("{alg} is not supported by Apinata"));
        }
    }

    #[test]
    fn a_malformed_pem_signs_as_a_secret_and_never_panics() {
        let odd = text("-----BEGIN -----END -----");
        assert!(sign("HS256", &header("HS256"), PAYLOAD, &odd).is_ok());
        assert!(sign("RS256", &header("RS256"), PAYLOAD, &odd).is_err());
    }

    #[test]
    fn decode_splits_valid_tokens_and_rejects_malformed_ones() {
        let token = vector("HS256");
        let (h, p, s) = decode(&token).expect("decodes");
        assert_eq!(
            (h.as_str(), p.as_str()),
            (header("HS256").as_str(), PAYLOAD)
        );
        assert_eq!(Some(s.as_str()), token.rsplit('.').next());
        for bad in ["abc", "a.b", "a!.b.c", ".eyJiIjoyfQ.x"] {
            assert_eq!(decode(bad), None, "{bad}");
        }
        assert!(decode("eyJhIjoxfQ.eyJiIjoyfQ.").is_some());
    }
}
