//! Golden vectors: the reference signer's output, recorded in the test vectors.

use super::*;
use SignatureMethod::*;
use rsa::pkcs8::DecodePrivateKey;
use std::path::Path;

const PKCS8: &str = include_str!("../../tests/fixtures/oauth1/rsa-pkcs8.pem");
const TWITTER_SECRETS: (&str, &str) = (
    "kAcSOqF21Fu85e7zjz7ZN2U4ZRhfV3WpwPAoE3Z7kBw",
    "LswwdoUaIvS8ltyTt5jkRh4J50vUPVVHtR2YPi5kE",
);

/// The lines under `# <title>...` in the reference output.
fn section(title: &str) -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reference/2026-10-13-oauth1-vectors/vectors.out");
    let text = std::fs::read_to_string(path).unwrap();
    let at = text
        .find(&format!("# {title}"))
        .unwrap_or_else(|| panic!("no section {title}"));
    text[at..]
        .lines()
        .skip(1)
        .take_while(|l| !l.starts_with("# "))
        .map(str::to_owned)
        .collect()
}

/// The rest of the line that starts with `key` and a tab.
fn lookup(title: &str, key: &str) -> String {
    let prefix = format!("{key}\t");
    section(title)
        .iter()
        .find_map(|l| l.strip_prefix(&prefix).map(str::to_owned))
        .unwrap_or_else(|| panic!("no {key} under {title}"))
}

fn url(s: &str) -> Url {
    Url::parse(s).unwrap()
}

fn key() -> RsaPrivateKey {
    RsaPrivateKey::from_pkcs8_pem(PKCS8).unwrap()
}

#[test]
fn the_twitter_base_string_signs_like_the_reference_with_every_method() {
    let twitter = lookup("base strings", "twitter");
    let signing = signing_key(TWITTER_SECRETS.0, TWITTER_SECRETS.1);
    assert_eq!(
        lookup("authorize()", "HMAC-SHA1"),
        "hCtSmYh+iHYCEqBWrE7C7hYmtUk="
    );
    for method in [
        HmacSha1, HmacSha256, HmacSha512, Plaintext, RsaSha1, RsaSha256, RsaSha512,
    ] {
        // Only the method's name differs between the methods' base strings.
        let base = twitter.replace("method%3DHMAC-SHA1", &format!("method%3D{}", method.name()));
        let k = method.is_rsa().then(key);
        let got = signature(method, k.as_ref(), &signing, &base).unwrap();
        assert_eq!(
            got,
            lookup("authorize()", method.name()),
            "{}",
            method.name()
        );
    }
    assert!(signature(RsaSha1, None, &signing, &twitter).is_err());
}

#[test]
fn encoding_base_url_and_key_follow_the_reference() {
    for (raw, want) in [
        ("abcXYZ019-._~", "abcXYZ019-._~"),
        ("hello world", "hello%20world"),
        ("a+b", "a%2Bb"),
        ("bang!*'()", "bang%21%2A%27%28%29"),
        ("a/b@c&d=e", "a%2Fb%40c%26d%3De"),
        ("café", "caf%C3%A9"),
        ("", ""),
    ] {
        assert_eq!(encode(raw), want, "{raw}");
    }
    for (raw, want) in [
        (
            "HTTPS://EXAMPLE.COM:443/path?q=1#f",
            "https://example.com/path",
        ),
        ("http://example.com:80/p", "http://example.com/p"),
        ("https://example.com:8443/p", "https://example.com:8443/p"),
        ("https://example.com:80/p", "https://example.com:80/p"),
        ("https://example.com", "https://example.com/"),
    ] {
        assert_eq!(base_url(&url(raw)), want, "{raw}");
    }
    assert_eq!(
        signing_key("secret&with=special", "ts"),
        "secret%26with%3Dspecial&ts"
    );
    assert_eq!(
        signing_key("a b", "token/secret!"),
        "a%20b&token%2Fsecret%21"
    );
}

#[test]
fn body_hash_uses_the_methods_digest() {
    for body in ["", "Hello World", r#"{"a":1}"#] {
        for (method, as_name) in [
            (HmacSha1, "HMAC-SHA1"),
            (RsaSha1, "HMAC-SHA1"),
            (Plaintext, "HMAC-SHA1"),
            (HmacSha256, "HMAC-SHA256"),
            (RsaSha256, "HMAC-SHA256"),
            (HmacSha512, "HMAC-SHA512"),
            (RsaSha512, "HMAC-SHA512"),
        ] {
            let row = format!("{}\t{as_name}", serde_json::to_string(body).unwrap());
            assert_eq!(
                body_hash(body.as_bytes(), method),
                lookup("computeBodyHash", &row),
                "{body:?} {}",
                method.name()
            );
        }
    }
}

#[test]
fn nonces_are_32_random_alphanumerics() {
    let n = nonce().unwrap();
    assert!(
        n.len() == 32 && n.chars().all(|c| c.is_ascii_alphanumeric()),
        "{n}"
    );
    assert_ne!(n, nonce().unwrap());
}

use serde_json::Value;

const TS: &str = "1318622958";
const FORM: Option<&str> = Some("application/x-www-form-urlencoded");
const TWITTER_URL: &str = "https://api.twitter.com/1.1/statuses/update.json?include_entities=true";
const TWITTER_BODY: &[u8] = b"status=Hello+Ladies+%2B+Gentlemen%2C+a+signed+OAuth+request%21";

/// The `applyOAuth1ToRequest` cases' credentials.
fn signer(method: SignatureMethod, placement: Placement) -> Signer {
    Signer {
        consumer_key: "consumer_key_1".into(),
        consumer_secret: "consumer_secret_1".into(),
        token: "access_token_1".into(),
        token_secret: "token_secret_1".into(),
        callback: String::new(),
        verifier: String::new(),
        method,
        key: None,
        timestamp: TS.into(),
        nonce: "n0nce".into(),
        version: String::new(),
        realm: String::new(),
        placement,
        include_body_hash: false,
    }
}

/// One request as `applyOAuth1ToRequest` left it.
fn reference(id: &str) -> Value {
    section("applyOAuth1ToRequest")
        .iter()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find(|v| v["id"] == id)
        .unwrap_or_else(|| panic!("no {id}"))
}

fn twitter() -> Signer {
    Signer {
        consumer_key: "xvz1evFS4wEEPTGEFPHBog".into(),
        consumer_secret: TWITTER_SECRETS.0.into(),
        token: "370773112-GmHxMAgYyLbNEtIKZeRNFsMKPR9EyMZeS9weJAEb".into(),
        token_secret: TWITTER_SECRETS.1.into(),
        nonce: "kYjzVBB8Y0ZFabxSWbWovY3uYSQ2pTgmZeNu2VS4cg".into(),
        version: "1.0".into(),
        ..signer(HmacSha1, Placement::Header)
    }
}

fn sign(s: &Signer, method: &str, u: &str, content_type: Option<&str>, body: &[u8]) -> Signed {
    let u = url(u);
    s.sign(
        &Hop {
            method,
            url: &u,
            content_type,
            body,
        },
        0,
        "unused",
    )
    .unwrap()
}

/// The decoded value of `key` in an `OAuth ...` header.
fn param(auth: &str, key: &str) -> String {
    let at = auth
        .find(&format!("{key}=\""))
        .unwrap_or_else(|| panic!("no {key} in {auth}"));
    let raw = auth[at + key.len() + 2..].split('"').next().unwrap();
    percent_encoding::percent_decode_str(raw)
        .decode_utf8()
        .unwrap()
        .into_owned()
}

#[test]
fn base_strings_match_the_reference_and_rfc_5849() {
    let u = url(TWITTER_URL);
    let hop = Hop {
        method: "POST",
        url: &u,
        content_type: FORM,
        body: TWITTER_BODY,
    };
    assert_eq!(
        twitter().base_string(&hop, 0, "unused"),
        lookup("base strings", "twitter")
    );
    let auth = sign(&twitter(), "POST", TWITTER_URL, FORM, TWITTER_BODY)
        .authorization
        .unwrap();
    assert_eq!(
        param(&auth, "oauth_signature"),
        "hCtSmYh+iHYCEqBWrE7C7hYmtUk="
    );
    let rfc = Signer {
        consumer_key: "9djdj82h48djs9d2".into(),
        consumer_secret: "j49sk3j29djd".into(),
        token: "kkk9d7dh3k39sjv7".into(),
        token_secret: "dh893hdasih9".into(),
        timestamp: "137131201".into(),
        nonce: "7d8f3e4a".into(),
        realm: "Example".into(),
        ..signer(HmacSha1, Placement::Header)
    };
    let u = url("http://example.com/request?b5=%3D%253D&a3=a&c%40=&a2=r%20b");
    let hop = Hop {
        method: "POST",
        url: &u,
        content_type: FORM,
        body: b"c2&a3=2+q",
    };
    assert_eq!(
        rfc.base_string(&hop, 0, "unused"),
        lookup("base strings", "rfc5849")
    );
}

#[test]
fn whole_requests_match_the_reference() {
    let mut realm = signer(HmacSha1, Placement::Header);
    (realm.realm, realm.callback, realm.verifier) =
        (r#"Photo "R"\"#.into(), "oob".into(), "v 1".into());
    let mut no_token = signer(HmacSha1, Placement::Header);
    (no_token.token, no_token.token_secret) = (String::new(), "ignored".into());
    let mut query = signer(HmacSha256, Placement::Query);
    query.include_body_hash = true;
    let mut form = signer(HmacSha1, Placement::Body);
    form.include_body_hash = true;
    let mut rsa = signer(RsaSha256, Placement::Header);
    (rsa.consumer_secret, rsa.key, rsa.include_body_hash) = (String::new(), Some(key()), true);
    let mut plain = signer(Plaintext, Placement::Header);
    plain.include_body_hash = true;
    let json = Some("application/json");
    type Case = (
        &'static str,
        Signer,
        &'static str,
        &'static str,
        Option<&'static str>,
        &'static [u8],
    );
    let cases: [Case; 8] = [
        (
            "header-get-realm",
            realm,
            "GET",
            "https://Example.COM:443/a%20b/c?b=2&a=1+x&oauth_x=keep",
            None,
            b"",
        ),
        (
            "header-no-token",
            no_token,
            "GET",
            "http://localhost:8080/p",
            None,
            b"",
        ),
        (
            "query-json-bodyhash",
            query,
            "POST",
            "https://api.test/items?q=a%20b&oauth_nonce=old",
            json,
            br#"{"a":1}"#,
        ),
        (
            "body-form",
            form,
            "POST",
            "https://api.test/form",
            Some("application/x-www-form-urlencoded; charset=utf-8"),
            b"b=x+y&a=1&oauth_extra=e",
        ),
        (
            "body-json-replaced",
            signer(HmacSha512, Placement::Body),
            "PUT",
            "https://api.test/json",
            json,
            br#"{"a":1}"#,
        ),
        (
            "get-form-header",
            signer(HmacSha1, Placement::Header),
            "GET",
            "https://api.test/g",
            FORM,
            b"a=1",
        ),
        (
            "rsa-sha256-header",
            rsa,
            "POST",
            "https://api.test/rsa",
            None,
            b"raw body",
        ),
        (
            "plaintext-empty-body-hash",
            plain,
            "DELETE",
            "https://api.test/p",
            None,
            b"",
        ),
    ];
    for (id, s, method, u, content_type, body) in cases {
        let want = reference(id);
        let got = sign(&s, method, u, content_type, body);
        match s.placement {
            Placement::Header => assert_eq!(
                got.authorization.as_deref(),
                want["headers"]["Authorization"].as_str(),
                "{id}"
            ),
            Placement::Query => assert_eq!(Some(got.url.as_str()), want["url"].as_str(), "{id}"),
            Placement::Body => {
                assert_eq!(
                    std::str::from_utf8(&got.body).ok(),
                    want["data"].as_str(),
                    "{id}"
                );
                let replaced =
                    want["headers"]["Content-Type"] == "application/x-www-form-urlencoded";
                assert_eq!(got.content_type.is_some(), replaced, "{id}");
            }
        }
    }
}

#[test]
fn multipart_and_form_bodies_get_no_body_hash() {
    let mut s = signer(HmacSha1, Placement::Header);
    s.include_body_hash = true;
    for ct in [
        "multipart/form-data; boundary=x",
        "Application/X-WWW-Form-Urlencoded",
    ] {
        let auth = sign(&s, "POST", "https://api.test/m", Some(ct), b"a=1")
            .authorization
            .unwrap();
        assert!(!auth.contains("oauth_body_hash"), "{ct}: {auth}");
    }
}

#[test]
fn blank_timestamp_and_nonce_take_the_given_ones() {
    let mut s = signer(HmacSha1, Placement::Header);
    (s.timestamp, s.nonce) = (String::new(), String::new());
    let u = url("https://api.test/x");
    let hop = Hop {
        method: "GET",
        url: &u,
        content_type: None,
        body: b"",
    };
    let auth = s.sign(&hop, 42, "abc").unwrap().authorization.unwrap();
    assert_eq!(
        (param(&auth, "oauth_timestamp"), param(&auth, "oauth_nonce")),
        ("42".into(), "abc".into())
    );
}

#[test]
fn strip_removes_only_oauth_pairs() {
    let mut u = url("https://api.test/x?a=1&oauth_signature=s&b=2&oauth_nonce=n");
    let mut body = b"c=3&oauth_token=t".to_vec();
    strip(&mut u, FORM, &mut body);
    assert_eq!(
        (u.as_str(), body.as_slice()),
        ("https://api.test/x?a=1&b=2", &b"c=3"[..])
    );
    let mut plain = url("https://api.test/y?q=a%20b");
    let mut json = br#"{"oauth_x":1}"#.to_vec();
    strip(&mut plain, Some("application/json"), &mut json);
    assert_eq!(
        (plain.as_str(), json.as_slice()),
        ("https://api.test/y?q=a%20b", &br#"{"oauth_x":1}"#[..])
    );
}

#[test]
fn strip_drops_an_emptied_query_and_encoded_oauth_keys() {
    let mut u = url("https://api.test/z?oauth_nonce=n&oauth%5Fsignature=s");
    let mut body = Vec::new();
    strip(&mut u, None, &mut body);
    assert_eq!(u.as_str(), "https://api.test/z");
}

#[test]
fn debug_shows_no_credentials() {
    let mut s = twitter();
    s.key = Some(key());
    let text = format!("{s:?}");
    for secret in ["kAcSOqF21", "LswwdoUa", "370773112", "xvz1evFS"] {
        assert!(!text.contains(secret), "{text}");
    }
}
