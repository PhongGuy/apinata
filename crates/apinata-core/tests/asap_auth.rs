//! Atlassian ASAP auth: the file block, the signed token and how it is sent and hidden.

use crate::common;

use apinata_core::{AsapSpec, AuthSpec, RequestFile, ResolveError, ResolvedRequest};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use common::{at, executor, resolved_one, setup};
use rsa::RsaPublicKey;
use rsa::pkcs1v15::{Signature, VerifyingKey};
use rsa::pkcs8::DecodePublicKey as _;
use rsa::sha2::{Sha256, Sha384};
use rsa::signature::Verifier as _;
use serde_json::Value;
use std::fs;
use tempfile::TempDir;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const RSA: &str = include_str!("fixtures/jwt/rsa.pem");
const RSA_PKCS1: &str = include_str!("fixtures/jwt/rsa-pkcs1.pem");
const RSA_PUB: &str = include_str!("fixtures/jwt/rsa.pub.pem");
const EC256_SEC1: &str = include_str!("fixtures/jwt/ec256-sec1.pem");
const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";
const HEAD: &str =
    "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: https://x.test/\n";

const FULL: &str = "  auth:
    type: asap
    algorithm: RS384
    issuer: svc/a
    audience: svc/b
    keyId: svc/a/key1
    subject: user-1
    expiresIn: \"600\"
    additionalClaims: '{\"scope\":\"read\"}'
    privateKey: \"{{asap_key}}\"
";

fn asap(auth: AuthSpec) -> AsapSpec {
    match auth {
        AuthSpec::Asap(spec) => spec,
        other => panic!("not asap: {other:?}"),
    }
}

fn file(auth: &str) -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("r.yml");
    fs::write(&path, format!("{HEAD}{auth}")).unwrap();
    (dir, path)
}

/// A one-line YAML scalar holding `pem` with its line breaks as literal `\n`.
fn inline(pem: &str) -> String {
    format!("\"{}\"", pem.trim().replace('\n', "\\\\n"))
}

/// An ASAP block with the given extra lines, signing with the PKCS#8 RSA key.
fn block(extra: &str) -> String {
    format!(
        "  auth:\n    type: asap\n    issuer: svc/a\n    audience: svc/b\n    keyId: svc/a/key1\n    privateKey: {}\n{extra}",
        inline(RSA)
    )
}

fn resolved(auth: &str) -> ResolvedRequest {
    resolved_one("GET", "http://127.0.0.1:9/a", auth, "")
}

fn refusal(auth: &str) -> String {
    let r = format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: http://127.0.0.1:9/a\n{auth}"
    );
    let (_d, c, ctx) = setup(&[("opencollection.yml", ROOT), ("r.yml", &r)]);
    match at(&c, &ctx, "r.yml") {
        Err(e @ ResolveError::Asap { .. }) => e.to_string(),
        other => panic!("expected an ASAP error: {other:?}"),
    }
}

fn token(r: &ResolvedRequest) -> String {
    let value = r
        .headers()
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("authorization"))
        .map(|(_, v)| v.clone())
        .expect("an Authorization header");
    value
        .strip_prefix("Bearer ")
        .expect("a bearer token")
        .to_owned()
}

/// The token's header and claims, after its RS256 or RS384 signature verifies with the
/// fixture's public key through the `rsa` crate.
fn verified(token: &str, bits: u16) -> (Value, Value) {
    let (signed, signature) = token.rsplit_once('.').expect("three parts");
    let key = RsaPublicKey::from_public_key_pem(RSA_PUB).unwrap();
    let signature = Signature::try_from(URL_SAFE_NO_PAD.decode(signature).unwrap().as_slice())
        .expect("a signature");
    let ok = match bits {
        256 => VerifyingKey::<Sha256>::new(key).verify(signed.as_bytes(), &signature),
        _ => VerifyingKey::<Sha384>::new(key).verify(signed.as_bytes(), &signature),
    };
    ok.expect("the signature verifies");
    let mut parts = signed.split('.');
    let mut part = || -> Value {
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts.next().unwrap()).unwrap()).unwrap()
    };
    let header = part();
    (header, part())
}

#[test]
fn every_key_reads_into_the_spec() {
    let (_d, path) = file(FULL);
    let spec = asap(RequestFile::load(&path).unwrap().auth());
    assert_eq!(spec.algorithm, "RS384");
    assert_eq!(spec.issuer, "svc/a");
    assert_eq!(spec.audience, "svc/b");
    assert_eq!(spec.key_id, "svc/a/key1");
    assert_eq!(spec.subject, "user-1");
    assert_eq!(spec.expires_in, "600");
    assert_eq!(spec.additional_claims, "{\"scope\":\"read\"}");
    assert_eq!(spec.private_key, "{{asap_key}}");
}

#[test]
fn an_unchanged_spec_leaves_the_file_byte_for_byte() {
    let (_d, path) = file(FULL);
    let mut r = RequestFile::load(&path).unwrap();
    r.set_auth(r.auth());
    r.save().unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), format!("{HEAD}{FULL}"));
}

#[test]
fn a_new_spec_writes_every_key_in_order() {
    let (_d, path) = file("");
    let mut r = RequestFile::load(&path).unwrap();
    let mut spec = AsapSpec::default();
    spec.issuer = "svc/a".into();
    spec.private_key = "{{asap_key}}".into();
    r.set_auth(AuthSpec::Asap(spec.clone()));
    r.save().unwrap();
    let text = fs::read_to_string(&path).unwrap();
    let want = "  auth:
    type: asap
    algorithm: RS256
    issuer: svc/a
    audience: \"\"
    keyId: \"\"
    subject: \"\"
    expiresIn: \"\"
    additionalClaims: \"\"
    privateKey: \"{{asap_key}}\"
";
    assert!(
        text.ends_with(&format!("url: https://x.test/\n{want}")),
        "{text}"
    );
    assert_eq!(asap(RequestFile::load(&path).unwrap().auth()), spec);
}

#[test]
fn a_sparse_block_reads_rs256_and_a_numeric_expiry_reads_as_text() {
    let spec = asap(
        RequestFile::load(&file("  auth:\n    type: asap\n    expiresIn: 120\n").1)
            .unwrap()
            .auth(),
    );
    assert_eq!(spec.algorithm, "RS256");
    assert_eq!(spec.expires_in, "120");
    assert_eq!(spec.private_key, "");
}

#[test]
fn debug_hides_the_private_key() {
    let mut spec = AsapSpec::default();
    spec.private_key = "hunter2-asap-key".into();
    let shown = format!("{:?}", AuthSpec::Asap(spec));
    assert!(!shown.contains("hunter2-asap-key"), "{shown}");
}

#[test]
fn the_token_verifies_and_carries_the_standard_claims() {
    let r = resolved(&block(""));
    let (header, claims) = verified(&token(&r), 256);
    assert_eq!(header.to_string(), r#"{"alg":"RS256","kid":"svc/a/key1"}"#);
    let keys: Vec<_> = claims.as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys, ["iss", "sub", "aud", "iat", "exp", "jti"]);
    assert_eq!(claims["iss"], "svc/a");
    // A blank subject is the issuer.
    assert_eq!(claims["sub"], "svc/a");
    assert_eq!(claims["aud"], "svc/b");
    let (iat, exp) = (
        claims["iat"].as_u64().unwrap(),
        claims["exp"].as_u64().unwrap(),
    );
    assert_eq!(exp - iat, 3600);
    let jti = claims["jti"].as_str().unwrap();
    assert!(uuid::Uuid::parse_str(jti).is_ok(), "{jti}");
    let (_, again) = verified(&token(&resolved(&block(""))), 256);
    assert_ne!(again["jti"], claims["jti"]);
}

#[test]
fn subject_expiry_algorithm_and_pkcs1_keys_are_honoured() {
    let auth = format!(
        "  auth:\n    type: asap\n    algorithm: RS384\n    issuer: svc/a\n    audience: svc/b\n    keyId: k\n    subject: user-1\n    expiresIn: \"60\"\n    privateKey: {}\n",
        inline(RSA_PKCS1)
    );
    let (header, claims) = verified(&token(&resolved(&auth)), 384);
    assert_eq!(header["alg"], "RS384");
    assert_eq!(claims["sub"], "user-1");
    let span = claims["exp"].as_u64().unwrap() - claims["iat"].as_u64().unwrap();
    assert_eq!(span, 60);
}

#[test]
fn additional_claims_add_to_and_override_the_standard_ones() {
    let auth = block("    additionalClaims: '{\"scope\":\"read\",\"iss\":\"other\",\"exp\":5}'\n");
    let (_, claims) = verified(&token(&resolved(&auth)), 256);
    assert_eq!(claims["scope"], "read");
    assert_eq!(claims["iss"], "other");
    assert_eq!(claims["exp"], 5);
}

#[test]
fn an_ec_key_signs_with_es256() {
    let auth = format!(
        "  auth:\n    type: asap\n    algorithm: ES256\n    issuer: i\n    audience: a\n    keyId: k\n    privateKey: {}\n",
        inline(EC256_SEC1)
    );
    let t = token(&resolved(&auth));
    let head = t.split('.').next().unwrap();
    let header: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(head).unwrap()).unwrap();
    assert_eq!(header["alg"], "ES256");
}

/// The fixture's PKCS#8 DER as base64, the body of its PEM.
fn der_base64() -> String {
    RSA.lines().filter(|l| !l.starts_with("-----")).collect()
}

#[test]
fn a_data_uri_key_signs_and_fills_a_blank_key_id() {
    let uri = format!(
        "data:application/pkcs8;kid=svc%2Fkey9;base64,{}",
        der_base64()
    );
    let auth = format!(
        "  auth:\n    type: asap\n    issuer: i\n    audience: a\n    privateKey: \"{uri}\"\n"
    );
    let (header, _) = verified(&token(&resolved(&auth)), 256);
    assert_eq!(header["kid"], "svc/key9");
    let clash = format!(
        "  auth:\n    type: asap\n    issuer: i\n    audience: a\n    keyId: other\n    privateKey: \"{uri}\"\n"
    );
    let err = refusal(&clash);
    assert!(err.contains("keyId"), "{err}");
    assert!(!err.contains(&der_base64()[..40]), "{err}");
}

#[test]
fn a_bare_base64_der_key_signs() {
    let auth = format!(
        "  auth:\n    type: asap\n    issuer: i\n    audience: a\n    keyId: k\n    privateKey: {}\n",
        der_base64()
    );
    verified(&token(&resolved(&auth)), 256);
    // The decoded DER is the same key, so the base64 here never fails to decode.
    assert!(STANDARD.decode(der_base64()).is_ok());
}

#[test]
fn refusals_name_the_field_and_never_quote_the_key() {
    let with =
        |lines: &str, key: &str| format!("  auth:\n    type: asap\n{lines}    privateKey: {key}\n");
    let full = "    issuer: i\n    audience: a\n    keyId: k\n";
    let key = inline(RSA);
    let cases = [
        (with("    audience: a\n    keyId: k\n", &key), "issuer"),
        (with("    issuer: i\n    keyId: k\n", &key), "audience"),
        (with("    issuer: i\n    audience: a\n", &key), "keyId"),
        (with(full, "\"\""), "private key"),
        (
            with(&format!("{full}    algorithm: HS256\n"), &key),
            "algorithm",
        ),
        (
            with(&format!("{full}    algorithm: XS1\n"), &key),
            "algorithm",
        ),
        (
            with(&format!("{full}    expiresIn: soon\n"), &key),
            "expiresIn",
        ),
        (
            with(&format!("{full}    additionalClaims: '[\"leak\"]'\n"), &key),
            "additionalClaims",
        ),
        (with(full, "leak-not-a-key!!"), "private key"),
        (
            with(&format!("{full}    algorithm: ES256\n"), &key),
            "does not match",
        ),
    ];
    for (auth, want) in cases {
        let err = refusal(&auth);
        assert!(err.contains(want), "{want}: {err}");
        for leak in ["leak", "BEGIN", "MII", "XS1"] {
            assert!(!err.contains(leak), "{want}: {err}");
        }
    }
}

#[test]
fn a_missing_variable_wins_over_a_field_check() {
    let r = "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: http://127.0.0.1:9/a\n  auth:\n    type: asap\n    privateKey: \"{{nope}}\"\n";
    let (_d, c, ctx) = setup(&[("opencollection.yml", ROOT), ("r.yml", r)]);
    let err = at(&c, &ctx, "r.yml").expect_err("refused");
    assert!(
        matches!(err, ResolveError::UndefinedVariable { .. }),
        "{err:?}"
    );
}

#[test]
fn the_key_is_sensitive_and_the_token_is_derived() {
    let r = resolved(&block(""));
    let pem = RSA.trim().to_owned();
    assert!(
        r.sensitive_values().iter().any(|v| v.contains(&pem)),
        "the key is hidden"
    );
    let t = token(&r);
    assert!(r.sensitive_values().contains(&t));
}

#[tokio::test]
async fn a_same_origin_redirect_keeps_the_token_and_another_origin_drops_it() {
    let first = MockServer::start().await;
    let second = MockServer::start().await;
    Mock::given(path("/start"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/next"))
        .mount(&first)
        .await;
    let away = format!("{}/end", second.uri());
    Mock::given(path("/next"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", away.as_str()))
        .mount(&first)
        .await;
    Mock::given(path("/end"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&second)
        .await;
    let req = resolved_one(
        "GET",
        &format!("{}/start", first.uri()),
        &block(""),
        "settings:\n  forwardAuthorizationHeader: false\n",
    );
    let res = executor().send(&req).await.expect("sent");
    let hops = res.timeline();
    assert_eq!(hops.len(), 3);
    let auth = |i: usize| {
        hops[i]
            .request_headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case("authorization"))
            .map(|(_, v)| v.clone())
    };
    assert!(auth(0).is_some());
    assert_eq!(auth(1), auth(0));
    assert_eq!(auth(2), None);
}
