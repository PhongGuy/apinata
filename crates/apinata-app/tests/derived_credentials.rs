//! Derived credentials never reach the history files, the index or a saved example.

use apinata_app::last_response::ResponseStore;
use apinata_core::{ItemPath, Outcome, RequestResult, Response};
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

const SIGNATURE: &str = "c2lnbmF0dXJlK3gvPQ==";
const HEADER: &str = "Digest username=\"bob\", response=\"9f3a77c1d2e44b60\", nonce=\"n0nce-77\"";

fn echoing() -> Response {
    Response::new(200)
        .with_url(format!(
            "http://127.0.0.1:1/r?oauth_signature={}&x=1",
            "c2lnbmF0dXJlK3gvPQ%3D%3D"
        ))
        .with_header("x-echo", HEADER)
        .with_text_body(format!("{{\"auth\":\"{HEADER}\",\"sig\":\"{SIGNATURE}\"}}"))
        .with_derived_credentials(vec![HEADER.to_owned(), SIGNATURE.to_owned()])
}

/// Every byte under `dir`, files concatenated.
fn stored(dir: &Path) -> String {
    let mut all = String::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        all.push_str(&String::from_utf8_lossy(
            &std::fs::read(entry.unwrap().path()).unwrap(),
        ));
    }
    all
}

#[test]
fn history_files_and_index_hold_no_derived_value() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("responses");
    let store = ResponseStore::new(dir.clone());
    let item = ItemPath::new("a.yml").unwrap();
    let result = RequestResult::from_parts(
        item.clone(),
        "A",
        Outcome::Passed,
        Some(echoing()),
        Vec::new(),
    );
    store
        .save(
            Path::new("/c"),
            &item,
            &result,
            UNIX_EPOCH + Duration::from_millis(5),
        )
        .unwrap();
    let files = stored(&dir);
    assert!(files.contains("x=1"), "the rest of the url is kept");
    for leaked in [
        HEADER,
        SIGNATURE,
        "c2lnbmF0dXJlK3gvPQ%3D%3D",
        "9f3a77c1d2e44b60",
    ] {
        assert!(!files.contains(leaked), "{leaked} in {files}");
    }
    let loaded = store.load(Path::new("/c"), &item).expect("kept");
    let kept = loaded.result.response.expect("response");
    assert!(kept.derived_credentials().is_empty());
}

use apinata_app::save_example::{example_response, secrets_in};
use apinata_core::derived_secrets;

/// (mode, derived values, where the first value is echoed in the url)
const MODES: [(&str, &[&str], bool); 9] = [
    (
        "digest",
        &["Digest username=\"bob\", response=\"aa11bb22cc33dd44\""],
        false,
    ),
    ("ntlm", &["NTLM TlRMTVNTUAABAAAAB4IIogAAAAA="], false),
    (
        "edgegrid",
        &["EG1-HMAC-SHA256 client_token=akab;signature=QUJDRA=="],
        false,
    ),
    ("oauth1 header", &["Yy9zaWduYXR1cmUrYWJjZA=="], false),
    ("oauth1 query", &["Yy9zaWduYXR1cmUrYWJjZA=="], true),
    (
        "sigv4",
        &[
            "AWS4-HMAC-SHA256 Credential=AKID/20261019/r/s/aws4_request, Signature=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        ],
        false,
    ),
    (
        "wsse",
        &["UsernameToken Username=\"bob\", PasswordDigest=\"ZGlnZXN0\", Nonce=\"bm9uY2U=\""],
        false,
    ),
    ("oauth2 header", &["tok-4f1c-SECRET"], false),
    ("oauth2 query", &["tok-4f1c-SECRET"], true),
];

#[test]
fn each_mode_is_absent_from_history_and_a_saved_example() {
    for (mode, derived, in_url) in MODES {
        let first = derived[0];
        let url = if in_url {
            let q: String = url::form_urlencoded::byte_serialize(first.as_bytes()).collect();
            format!("http://127.0.0.1:1/r?k={q}&x=1")
        } else {
            "http://127.0.0.1:1/r?x=1".to_owned()
        };
        let body = format!("{{\"echo\":\"{}\"}}", derived.join("\",\"echo2\":\""));
        let response = Response::new(200)
            .with_url(url)
            .with_header("x-echo", first)
            .with_text_body(body)
            .with_derived_credentials(derived.iter().map(|s| s.to_string()).collect());

        let data = tempfile::tempdir().unwrap();
        let dir = data.path().join("responses");
        let store = ResponseStore::new(dir.clone());
        let item = ItemPath::new("a.yml").unwrap();
        let result = RequestResult::from_parts(
            item.clone(),
            "A",
            Outcome::Passed,
            Some(response.clone()),
            Vec::new(),
        );
        store
            .save(
                Path::new("/c"),
                &item,
                &result,
                UNIX_EPOCH + Duration::from_millis(5),
            )
            .unwrap();
        let files = stored(&dir);
        for v in derived {
            // The store may escape quotes, so look for the escaped form too.
            let escaped = serde_json::to_string(v).unwrap();
            let escaped = &escaped[1..escaped.len() - 1];
            assert!(!files.contains(v), "{mode}: {v} in history");
            assert!(!files.contains(escaped), "{mode}: {v} escaped in history");
        }

        let secrets = derived_secrets(response.derived_credentials());
        let mut example = example_response(&response);
        assert_eq!(
            secrets_in(&example, &secrets),
            vec!["derived credential".to_owned()],
            "{mode}"
        );
        example.scrub(Some(&secrets));
        let mut saved: Vec<&str> = example.headers.iter().map(|(_, v)| v.as_str()).collect();
        saved.extend(example.body.as_ref().map(|b| b.data.as_str()));
        assert!(saved.len() >= 2, "{mode}: header and body are checked");
        for v in derived {
            for text in &saved {
                assert!(!text.contains(v), "{mode}: {v} in example");
            }
        }
    }
}
