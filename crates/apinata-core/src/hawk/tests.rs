//! Vectors from the Hawk protocol's own README, plus app, SHA-1, escaping and IPv6 cases
//! computed independently from the protocol's normalized-string definition.

use super::*;
use crate::edit::HawkSpec;
use url::Url;

const ID: &str = "dh37fgj492je";
const KEY: &str = "werxhqb98rpaxn39848xrunpaw3489ruxnpa98w4rxn";
const TS: u64 = 1_353_832_234;
const NONCE: &str = "j4h3g2";
const RESOURCE: &str = "http://example.com:8000/resource/1?b=1&a=2";

fn spec(ext: &str) -> HawkSpec {
    HawkSpec {
        auth_id: ID.to_owned(),
        auth_key: KEY.to_owned(),
        ext: ext.to_owned(),
        ..HawkSpec::default()
    }
}

/// One bad setting: the error it must name and how it breaks a valid spec.
type BadCase = (&'static str, fn(&mut HawkSpec));

fn sign(spec: &HawkSpec, method: &str, url: &str, ct: Option<&str>, body: &[u8]) -> Signed {
    let signer = Signer::new(spec).expect("valid");
    let url = Url::parse(url).expect("url");
    signer
        .sign_at(
            &Outgoing {
                method,
                url: &url,
                content_type: ct,
                body,
            },
            TS,
            NONCE,
        )
        .expect("signs")
}

#[test]
fn readme_get_vector() {
    let signed = sign(&spec("some-app-ext-data"), "GET", RESOURCE, None, b"");
    assert_eq!(signed.mac, "6R4rV5iE+NPoym+WwjeHzjAGXUtLNIxmo1vpMofpLAE=");
    assert_eq!(
        signed.header,
        "Hawk id=\"dh37fgj492je\", ts=\"1353832234\", nonce=\"j4h3g2\", ext=\"some-app-ext-data\", mac=\"6R4rV5iE+NPoym+WwjeHzjAGXUtLNIxmo1vpMofpLAE=\""
    );
}

#[test]
fn readme_payload_vector() {
    let mut s = spec("some-app-ext-data");
    s.include_payload_hash = true;
    let signed = sign(
        &s,
        "POST",
        RESOURCE,
        Some("Text/Plain; charset=utf-8"),
        b"Thank you for flying Hawk",
    );
    assert_eq!(
        signed.header,
        "Hawk id=\"dh37fgj492je\", ts=\"1353832234\", nonce=\"j4h3g2\", hash=\"Yi9LfIIFRtBEPt74PVmbTF/xVAwPn7ub15ePICfgnuY=\", ext=\"some-app-ext-data\", mac=\"aSe1DERmZuRl3pI36/9BdZmnErTw3sNzOOAUlfeKjVw=\""
    );
}

#[test]
fn payload_hash_off_leaves_hash_out() {
    let signed = sign(&spec(""), "POST", RESOURCE, Some("text/plain"), b"body");
    assert!(!signed.header.contains("hash="), "{}", signed.header);
    assert!(!signed.header.contains("ext="), "{}", signed.header);
}

#[test]
fn app_and_dlg_are_signed_and_trail_the_mac() {
    let mut s = spec("some-app-ext-data");
    s.app = "my-app".to_owned();
    s.dlg = "their-app".to_owned();
    let signed = sign(&s, "GET", RESOURCE, None, b"");
    assert_eq!(signed.mac, "l8NjY8T4mgYSljAJrgye7TaCQOx36yBOoroBSLRQwsU=");
    assert!(
        signed.header.ends_with(
            "mac=\"l8NjY8T4mgYSljAJrgye7TaCQOx36yBOoroBSLRQwsU=\", app=\"my-app\", dlg=\"their-app\""
        ),
        "{}",
        signed.header
    );
}

#[test]
fn sha1_signs_with_sha1() {
    let mut s = spec("some-app-ext-data");
    s.algorithm = "sha1".to_owned();
    let signed = sign(&s, "GET", RESOURCE, None, b"");
    assert_eq!(signed.mac, "KqOejc9yo2NAQlM29iSeYQEzwmE=");
}

#[test]
fn ext_is_escaped_in_both_the_mac_and_the_header() {
    let signed = sign(&spec("a\\b\"c"), "GET", RESOURCE, None, b"");
    assert_eq!(signed.mac, "6XpzdsI4iKvl8CqYSIvrLdbLRj0QXIUWDPwZYlXTnj8=");
    assert!(
        signed.header.contains("ext=\"a\\\\b\\\"c\""),
        "{}",
        signed.header
    );
}

#[test]
fn ipv6_host_loses_brackets_and_https_defaults_to_443() {
    let signed = sign(&spec(""), "get", "https://[::1]/x?", None, b"");
    assert_eq!(signed.mac, "SEYwk9YyeF8sCpHmgDXbJ/ZYEYQ8b4TSEEKBaVfksfw=");
}

#[test]
fn fixed_nonce_and_timestamp_override_the_fresh_ones() {
    let mut s = spec("");
    s.nonce = NONCE.to_owned();
    s.timestamp = TS.to_string();
    let signer = Signer::new(&s).expect("valid");
    let url = Url::parse(RESOURCE).expect("url");
    let out = Outgoing {
        method: "GET",
        url: &url,
        content_type: None,
        body: b"",
    };
    let signed = signer.sign(&out).expect("signs");
    assert!(
        signed
            .header
            .starts_with("Hawk id=\"dh37fgj492je\", ts=\"1353832234\", nonce=\"j4h3g2\""),
        "{}",
        signed.header
    );
}

#[test]
fn fresh_nonces_differ_per_signature() {
    let signer = Signer::new(&spec("")).expect("valid");
    let url = Url::parse(RESOURCE).expect("url");
    let out = Outgoing {
        method: "GET",
        url: &url,
        content_type: None,
        body: b"",
    };
    let a = signer.sign(&out).expect("signs");
    let b = signer.sign(&out).expect("signs");
    assert_ne!(a.header, b.header);
}

#[test]
fn bad_settings_are_refused_without_quoting_the_key() {
    let cases: [BadCase; 5] = [
        ("authId is required", |s| s.auth_id.clear()),
        ("authKey is required", |s| s.auth_key.clear()),
        ("unknown algorithm", |s| s.algorithm = "md5".to_owned()),
        ("timestamp is not a whole number", |s| {
            s.timestamp = "soon".to_owned()
        }),
        ("ext holds a character", |s| s.ext = "caf\u{e9}".to_owned()),
    ];
    for (expected, change) in cases {
        let mut s = spec("");
        change(&mut s);
        let err = Signer::new(&s).err().expect(expected);
        assert!(err.contains(expected), "{expected}: {err}");
        assert!(!err.contains(KEY), "{err}");
    }
}
