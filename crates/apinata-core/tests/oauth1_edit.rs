//! OAuth 1.0 auth read and written as the reference does, from the
//! OAuth 1.0 parity corpus.

use apinata_core::{
    AuthSpec, Collection, ItemPath, OAuth1KeyKind, OAuth1Placement, OAuth1PrivateKey,
    OAuth1SignatureMethod, OAuth1Spec, RequestFile,
};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const CORPUS: &str = "2026-10-13-oauth1-parity";
const PEM: &str = "-----BEGIN PRIVATE KEY-----\nMIIBVQIBADANBgkqhkiG9w0BAQEFAASCAT8wggE7AgEAAkEA\n-----END PRIVATE KEY-----";

fn research() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/reference")
}

#[derive(serde::Deserialize)]
struct Case {
    id: String,
    kind: String,
    input: String,
    files: Vec<String>,
}

fn cases(kind: &str) -> Vec<Case> {
    let text = fs::read_to_string(research().join(CORPUS).join("cases.json")).unwrap();
    let all: Vec<Case> = serde_json::from_str(&text).unwrap();
    let picked: Vec<Case> = all.into_iter().filter(|c| c.kind == kind).collect();
    assert!(!picked.is_empty(), "no {kind} cases");
    picked
}

fn expected(id: &str, file: &str) -> String {
    fs::read_to_string(research().join(CORPUS).join("expected").join(id).join(file)).unwrap()
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn oauth1(auth: AuthSpec) -> OAuth1Spec {
    match auth {
        AuthSpec::OAuth1(o) => o,
        other => panic!("not oauth1: {other:?}"),
    }
}

/// The Apinata edit for request case `id`, given the auth the file reads as.
fn request_edit(id: &str, read: AuthSpec) -> AuthSpec {
    let s = |v: &str| Some(v.to_owned());
    let edit = |f: &dyn Fn(&mut OAuth1Spec)| {
        let mut o = oauth1(read.clone());
        f(&mut o);
        AuthSpec::OAuth1(o)
    };
    match id {
        "oauth1-new" => AuthSpec::OAuth1(OAuth1Spec::default()),
        "oauth1-new-filled" => {
            let mut o = OAuth1Spec::default();
            o.consumer_key = s("ck");
            o.consumer_secret = s("{{consumerSecret}}");
            o.access_token = s("tk");
            o.access_token_secret = s("ts");
            AuthSpec::OAuth1(o)
        }
        "oauth1-edit" => edit(&|o| o.consumer_key = s("ck2")),
        "oauth1-edit-blank" => edit(&|o| o.verifier = s("")),
        "oauth1-sparse" => edit(&|o| o.access_token = s("tk")),
        "oauth1-key-file" => edit(&|o| {
            o.signature_method = OAuth1SignatureMethod::RsaSha256;
            o.private_key = Some(OAuth1PrivateKey::new(
                OAuth1KeyKind::File,
                "keys/private.pem",
            ));
        }),
        "oauth1-key-text" => edit(&|o| {
            o.signature_method = OAuth1SignatureMethod::RsaSha1;
            o.private_key = Some(OAuth1PrivateKey::new(OAuth1KeyKind::Text, PEM));
        }),
        "oauth1-plain-key" => edit(&|o| o.verifier = s("v")),
        "oauth1-other" => edit(&|o| o.nonce = s("n")),
        "oauth1-to-digest" => AuthSpec::Digest {
            username: "u".into(),
            password: "p".into(),
        },
        other => panic!("no Apinata edit for {other}"),
    }
}

#[test]
fn request_edits_match_the_reference() {
    let mut failures = Vec::new();
    let picked = cases("request");
    for case in &picked {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(&case.files[0]);
        fs::copy(research().join(&case.input), &path).unwrap();
        let mut r = RequestFile::load(&path).unwrap();
        r.set_auth(request_edit(&case.id, r.auth()));
        r.save().unwrap();
        let (got, want) = (
            fs::read_to_string(&path).unwrap(),
            expected(&case.id, &case.files[0]),
        );
        if got != want {
            failures.push(format!(
                "{}\n  expected {want:?}\n  actual   {got:?}",
                case.id
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} differ:\n{}",
        failures.len(),
        picked.len(),
        failures.join("\n")
    );
}

#[test]
fn config_edits_match_the_reference() {
    let mut failures = Vec::new();
    for case in cases("config") {
        let dir = TempDir::new().unwrap();
        copy_tree(&research().join(&case.input), dir.path());
        let mut c = Collection::open(dir.path()).unwrap();
        let mut o = OAuth1Spec::default();
        match case.id.as_str() {
            "folder-oauth1" => {
                o.consumer_key = Some("fk".into());
                o.placement = OAuth1Placement::Body;
                let f = c
                    .folder_file_mut(&ItemPath::new("folder_1").unwrap())
                    .unwrap();
                f.set_auth(AuthSpec::OAuth1(o));
                f.save().unwrap();
            }
            "collection-oauth1" => {
                o.consumer_key = Some("ck".into());
                o.signature_method = OAuth1SignatureMethod::Plaintext;
                o.realm = Some("r".into());
                let f = c.file_mut();
                f.set_auth(AuthSpec::OAuth1(o));
                f.save().unwrap();
            }
            other => panic!("no Apinata edit for {other}"),
        }
        let got = fs::read_to_string(dir.path().join(&case.files[0])).unwrap();
        let want = expected(&case.id, &case.files[0]);
        if got != want {
            failures.push(format!(
                "{}\n  expected {want:?}\n  actual   {got:?}",
                case.id
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn blanks_read_as_absent_and_defaults_fill_in() {
    let path = research().join(CORPUS).join("sparse/oauth1-blank.yml");
    let o = oauth1(RequestFile::load(&path).unwrap().auth());
    let mut want = OAuth1Spec::default();
    want.consumer_key = Some("ck".into());
    want.access_token_secret = Some("  ".into());
    for field in [
        &mut want.consumer_secret,
        &mut want.access_token,
        &mut want.callback_url,
        &mut want.verifier,
        &mut want.timestamp,
        &mut want.nonce,
        &mut want.realm,
    ] {
        *field = None;
    }
    want.private_key = None;
    assert_eq!(o, want);
    let plain = research().join(CORPUS).join("sparse/oauth1-plain-key.yml");
    let o = oauth1(RequestFile::load(&plain).unwrap().auth());
    assert_eq!(
        o.private_key,
        Some(OAuth1PrivateKey::new(OAuth1KeyKind::Text, "{{privateKey}}"))
    );
    let other = research().join(CORPUS).join("sparse/oauth1-other.yml");
    let o = oauth1(RequestFile::load(&other).unwrap().auth());
    assert_eq!(
        o.signature_method,
        OAuth1SignatureMethod::Other("HMAC-MD5".into())
    );
    assert_eq!(o.placement, OAuth1Placement::Other("cookie".into()));
    assert_eq!(
        (o.signature_method.as_str(), o.placement.as_str()),
        ("HMAC-MD5", "cookie")
    );
}

#[test]
fn enum_names_round_trip() {
    use OAuth1SignatureMethod::*;
    for (m, name) in [
        (HmacSha1, "HMAC-SHA1"),
        (HmacSha256, "HMAC-SHA256"),
        (HmacSha512, "HMAC-SHA512"),
        (RsaSha1, "RSA-SHA1"),
        (RsaSha256, "RSA-SHA256"),
        (RsaSha512, "RSA-SHA512"),
        (Plaintext, "PLAINTEXT"),
    ] {
        assert_eq!(
            (m.as_str(), OAuth1SignatureMethod::parse(name)),
            (name, m.clone())
        );
        assert_eq!(m.is_rsa(), name.starts_with("RSA-"));
    }
    for (p, name) in [
        (OAuth1Placement::Header, "header"),
        (OAuth1Placement::Query, "query"),
        (OAuth1Placement::Body, "body"),
    ] {
        assert_eq!(
            (p.as_str(), OAuth1Placement::parse(name)),
            (name, p.clone())
        );
    }
}

#[test]
fn debug_hides_credentials() {
    let mut o = OAuth1Spec::default();
    o.consumer_key = Some("visible-key".into());
    o.consumer_secret = Some("cs-123".into());
    o.access_token = Some("tok-456".into());
    o.access_token_secret = Some("ts-789".into());
    o.private_key = Some(OAuth1PrivateKey::new(OAuth1KeyKind::Text, "pem-000"));
    let text = format!("{:?}", AuthSpec::OAuth1(o));
    for secret in ["cs-123", "tok-456", "ts-789", "pem-000"] {
        assert!(!text.contains(secret), "{text}");
    }
    assert!(text.contains("visible-key"), "{text}");
}
