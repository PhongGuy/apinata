//! The manifest signature: cargo packager's format, key lists and refusals.
#![cfg(feature = "update")]

use crate::update_support;

use apinata_core::update::{UpdateError, verify_manifest};
use update_support::TestKey;

fn fixture(name: &str) -> Vec<u8> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/update");
    std::fs::read(format!("{dir}/{name}")).unwrap()
}

fn fixture_key() -> String {
    String::from_utf8(fixture("test-only-key.pub")).unwrap()
}

#[test]
fn a_manifest_signed_by_cargo_packager_verifies() {
    verify_manifest(
        &fixture("latest.json"),
        &fixture("latest.json.sig"),
        &[fixture_key()],
    )
    .unwrap();
}

#[test]
fn the_fixture_fails_once_edited() {
    let mut m = fixture("latest.json");
    let i = m.iter().position(|&b| b == b'1').unwrap();
    m[i] = b'9';
    let got = verify_manifest(&m, &fixture("latest.json.sig"), &[fixture_key()]);
    assert!(
        matches!(got, Err(UpdateError::ManifestSignature(_))),
        "{got:?}"
    );
}

#[test]
fn any_listed_key_may_sign() {
    let (old, new) = (TestKey::generate(), TestKey::generate());
    let m = b"{\"version\":\"1.0.0\"}";
    let keys = [old.key_line(), new.key_line()];
    verify_manifest(m, new.sign(m).as_bytes(), &keys).unwrap();
    verify_manifest(m, old.sign(m).as_bytes(), &keys).unwrap();
}

#[test]
fn whitespace_around_signature_and_key_is_ignored() {
    let k = TestKey::generate();
    let m = b"{}";
    let sig = format!("{}\n", k.sign(m));
    verify_manifest(m, sig.as_bytes(), &[format!("  {}\r\n", k.key_line())]).unwrap();
}

#[test]
fn a_signature_from_an_unlisted_key_is_unknown() {
    let (listed, other) = (TestKey::generate(), TestKey::generate());
    let m = b"{}";
    assert_eq!(
        verify_manifest(m, other.sign(m).as_bytes(), &[listed.key_line()]),
        Err(UpdateError::UnknownKey)
    );
}

#[test]
fn malformed_or_mismatched_signatures_are_refused() {
    let k = TestKey::generate();
    let m = b"{\"version\":\"1.0.0\"}";
    let keys = [k.key_line()];
    // Empty, not base64, and base64 of text that is no signature box.
    for sig in ["", "not base64!", "aGVsbG8="] {
        let got = verify_manifest(m, sig.as_bytes(), &keys);
        assert!(
            matches!(got, Err(UpdateError::ManifestSignature(_))),
            "{sig:?}: {got:?}"
        );
    }
    let got = verify_manifest(b"{\"version\":\"9.0.0\"}", k.sign(m).as_bytes(), &keys);
    assert!(
        matches!(got, Err(UpdateError::ManifestSignature(_))),
        "{got:?}"
    );
}

#[test]
fn an_undecodable_key_is_skipped() {
    let k = TestKey::generate();
    let m = b"{}";
    let sig = k.sign(m);
    verify_manifest(m, sig.as_bytes(), &["garbage".to_string(), k.key_line()]).unwrap();
    assert_eq!(
        verify_manifest(m, sig.as_bytes(), &["garbage".to_string()]),
        Err(UpdateError::UnknownKey)
    );
}
