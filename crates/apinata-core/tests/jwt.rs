//! The JWT decoder: claims are read, nothing is verified.

use crate::common::*;
use apinata_core::{ContextOptions, Expiry, JwtError, VarContext, decode_jwt};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

fn part(json: &str) -> String {
    URL_SAFE_NO_PAD.encode(json)
}

fn token(header: &str, payload: &str) -> String {
    format!("{}.{}.c2ln", part(header), part(payload))
}

#[test]
fn a_valid_token_shows_header_and_payload_pretty_in_written_order() {
    let t = token(
        r#"{"typ":"JWT","alg":"HS256"}"#,
        r#"{"sub":"u1","exp":2000}"#,
    );
    let d = decode_jwt(&t).unwrap();
    assert_eq!(d.header, "{\n  \"typ\": \"JWT\",\n  \"alg\": \"HS256\"\n}");
    assert_eq!(d.payload, "{\n  \"sub\": \"u1\",\n  \"exp\": 2000\n}");
    assert_eq!(d.expires_at, Some(2000));
}

#[test]
fn padding_and_surrounding_space_are_tolerated() {
    let padded = format!("{}=.{}==.sig", part(r#"{"a":1}"#), part(r#"{"b":2}"#));
    assert!(decode_jwt(&format!("  {padded}\n")).is_ok());
}

#[test]
fn malformed_tokens_are_errors_and_never_panic() {
    let ok = part(r#"{"a":1}"#);
    for bad in [
        "".to_owned(),
        "abc".to_owned(),
        "a.b".to_owned(),
        "a.b.c.d".to_owned(),
        format!("{ok}.{ok}"),
        format!("!!!.{ok}.s"),
        format!("{}.{ok}.s", part("not json")),
        format!("{}.{ok}.s", part("[1]")),
        format!("{ok}.{}.s", part("\"str\"")),
        format!("{ok}.\u{e9}\u{e9}.s"),
    ] {
        assert!(decode_jwt(&bad).is_err(), "{bad:?}");
    }
    assert_eq!(decode_jwt("a.b"), Err(JwtError::NotThreeParts));
}

#[test]
fn expiry_is_judged_against_the_clock_passed_in() {
    let with_exp = decode_jwt(&token("{}", r#"{"exp":1000}"#)).unwrap();
    assert_eq!(with_exp.expiry(999), Expiry::Valid { expires_at: 1000 });
    assert_eq!(with_exp.expiry(1000), Expiry::Expired { expires_at: 1000 });
    assert_eq!(with_exp.expiry(5000), Expiry::Expired { expires_at: 1000 });
}

#[test]
fn a_missing_or_non_numeric_exp_means_no_expiry() {
    for payload in [r#"{"sub":"x"}"#, r#"{"exp":"soon"}"#, r#"{"exp":null}"#] {
        let d = decode_jwt(&token("{}", payload)).unwrap();
        assert_eq!(d.expires_at, None, "{payload}");
        assert_eq!(d.expiry(1), Expiry::Absent);
    }
}

#[test]
fn a_fractional_exp_rounds_down_to_seconds() {
    let d = decode_jwt(&token("{}", r#"{"exp":1000.9}"#)).unwrap();
    assert_eq!(d.expires_at, Some(1000));
}

#[test]
fn a_variable_reference_is_decoded_even_when_the_value_is_secret() {
    let t = token(r#"{"alg":"none"}"#, r#"{"sub":"hidden"}"#);
    let (_dir, c, _) = setup(&[
        (
            "opencollection.yml",
            "opencollection: 1.0.0\ninfo:\n  name: c\n",
        ),
        (
            "a.yml",
            "info:\n  name: a\n  type: http\n  seq: 1\n\nhttp:\n  method: GET\n  url: http://x\n",
        ),
    ]);
    let options = ContextOptions::new()
        .sensitive_env_var("tok", &t)
        .process_env(Vec::<(String, String)>::new());
    let ctx = VarContext::new(&c, options).unwrap();
    let set = ctx.variables(request(&c, "a.yml"));
    let d = set.decode_jwt("{{tok}}").unwrap();
    assert!(d.payload.contains("hidden"));
    assert_eq!(set.decode_jwt("{{missing}}"), Err(JwtError::Unresolved));
}
