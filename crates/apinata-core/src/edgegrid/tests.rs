//! Golden vectors: the reference signer's output, recorded in the test vectors.
#![allow(
    clippy::field_reassign_with_default,
    reason = "specs are built field by field from vectors"
)]

use super::*;
use crate::edit::EdgeGridSpec;
use base64::engine::general_purpose::STANDARD;
use serde_json::Value;
use std::path::{Path, PathBuf};
use url::Url;

const NONCE: &str = "nonce-xx-xxxx-xxxx-xxxx-xxxxxxxxxxxx";
const TIMESTAMP: &str = "20140321T19:34:21+0000";

const HOST: &str = "https://akaa-baseurl-xxxxxxxxxxx-xxxxxxxxxxxxx.luna.akamaiapis.net";
/// What every pipeline case signs (vectors.mjs `item`).
const PIPELINE_HEADERS: &str =
    "content-type,x-custom,content-length,user-agent,accept,authorization";
/// Vectors not compared byte for byte, each with its reason; each is asserted below instead.
const EXCLUDED: &[(&str, &str, &str)] = &[
    (
        "signer",
        "post-max-16-string",
        "unrepresentable: maxBodySize is a number in the model",
    ),
    (
        "signer",
        "post-max-16abc-parses-16",
        "unrepresentable: as post-max-16-string",
    ),
    (
        "signer",
        "post-max-nan-no-truncation",
        "unrepresentable: as post-max-16-string",
    ),
    (
        "signer",
        "nonce-timestamp-blank-generated",
        "generated values; the shape is asserted",
    ),
    (
        "pipeline",
        "multipart",
        "deviation 3: the bytes sent are hashed",
    ),
    ("pipeline", "file", "deviation 3: the bytes sent are hashed"),
    (
        "pipeline",
        "none",
        "deviation 4: a listed header that is not sent is skipped",
    ),
];
/// The numeric twin each unrepresentable vector signs like.
const TWINS: &[(&str, &str)] = &[
    ("post-max-16-string", "post-max-16-number"),
    ("post-max-16abc-parses-16", "post-max-16-number"),
    ("post-max-nan-no-truncation", "post-max-zero-is-default"),
];

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reference/2026-10-14-m10d-auth-vectors")
}

fn group(name: &str) -> Vec<Value> {
    let text = std::fs::read_to_string(dir().join("vectors.json")).unwrap();
    let all: Value = serde_json::from_str(&text).unwrap();
    all["edgegrid"][name].as_array().unwrap().clone()
}

/// vectors.mjs `CRED` with the case's `config` over it.
fn spec(id: &str, config: &Value) -> EdgeGridSpec {
    let mut s = EdgeGridSpec::default();
    s.client_token = "akab-client-token-xxx-xxxxxxxxxxxxxxxx".into();
    s.access_token = "akab-access-token-xxx-xxxxxxxxxxxxxxxx".into();
    s.client_secret = "SOMESECRET".into();
    s.timestamp = TIMESTAMP.into();
    s.nonce = NONCE.into();
    let set = |field: &mut String, key: &str| {
        if let Some(v) = config[key].as_str() {
            *field = v.to_owned();
        }
    };
    set(&mut s.access_token, "accessToken");
    set(&mut s.client_token, "clientToken");
    set(&mut s.client_secret, "clientSecret");
    set(&mut s.nonce, "nonce");
    set(&mut s.timestamp, "timestamp");
    set(&mut s.base_url, "baseURL");
    set(&mut s.headers_to_sign, "headersToSign");
    s.max_body_size = config["maxBodySize"].as_f64();
    // `missing-all` sets the three to undefined, which JSON drops from its config.
    if id == "missing-all" {
        s.access_token.clear();
        s.client_token.clear();
        s.client_secret.clear();
    }
    s
}

fn find(group_name: &str, id: &str) -> Value {
    group(group_name)
        .into_iter()
        .find(|c| c["id"] == id)
        .unwrap_or_else(|| panic!("no {id}"))
}

fn excluded(group_name: &str, id: &str) -> bool {
    EXCLUDED
        .iter()
        .any(|(g, i, _)| *g == group_name && *i == id)
}

/// Request headers in order; a non-string value is a header the client never sends.
fn headers(v: &Value) -> Vec<(String, String)> {
    v.as_object()
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                .collect()
        })
        .unwrap_or_default()
}

/// The body text the reference hashed: elided long strings rebuilt, objects stringified.
fn text(data: &Value) -> String {
    match data {
        Value::String(s) if s.starts_with("<131072 chars") => "a".repeat(131_072),
        Value::String(s) if s.starts_with("<131073 chars") => format!("{}b", "a".repeat(131_072)),
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => crate::js::stringify(other),
    }
}

fn content_hash(data_to_sign: &str) -> String {
    let fields: Vec<&str> = data_to_sign.split('\t').collect();
    fields[fields.len() - 2].to_owned()
}

fn pipeline_signer() -> (Url, Signer) {
    let url = Url::parse(&format!("{HOST}/m")).unwrap();
    let mut s = spec("pipeline", &Value::Null);
    s.headers_to_sign = PIPELINE_HEADERS.into();
    let signer = Signer::new(&s, &url).unwrap();
    (url, signer)
}

#[test]
fn missing_credentials_fail_in_the_reference_order() {
    let errors: Vec<Value> = group("signer")
        .into_iter()
        .filter(|c| c.get("error").is_some())
        .collect();
    assert_eq!(errors.len(), 4);
    for case in errors {
        let id = case["id"].as_str().unwrap();
        let url = Url::parse(case["request"]["url"].as_str().unwrap()).unwrap();
        let err = Signer::new(&spec(id, &case["config"]), &url)
            .err()
            .unwrap_or_else(|| panic!("{id} signed"));
        assert_eq!(
            format!("EdgeGrid: {err}"),
            case["error"].as_str().unwrap(),
            "{id}"
        );
    }
}

#[test]
fn a_base_url_that_does_not_parse_is_refused() {
    let url = Url::parse("https://h.example/p").unwrap();
    let mut s = spec("bad", &Value::Null);
    s.base_url = "https://[::1".into();
    assert_eq!(
        Signer::new(&s, &url).err().as_deref(),
        Some("baseURL is not a valid URL")
    );
}

#[test]
fn presence_is_the_reference_is_str_present() {
    for (s, want) in [
        ("", false),
        ("  ", false),
        ("undefined", false),
        (" undefined ", false),
        ("x", true),
        (" x ", true),
        ("Undefined", true),
    ] {
        assert_eq!(is_present(s), want, "{s:?}");
    }
}

#[test]
fn the_limit_follows_javascript_parse_int() {
    for (max, want) in [
        (None, Some(131_072)),
        (Some(0.0), Some(131_072)),
        (Some(f64::NAN), Some(131_072)),
        (Some(16.0), Some(16)),
        (Some(1.5), Some(1)),
        (Some(1e21), Some(1)),
        (Some(5e-7), Some(5)),
        (Some(-3.0), Some(0)),
        (Some(f64::INFINITY), None),
    ] {
        assert_eq!(body_limit(max), want, "{max:?}");
    }
}

#[test]
fn every_signer_vector_signs_like_the_reference() {
    let mut checked = 0;
    for case in group("signer") {
        let id = case["id"].as_str().unwrap();
        if excluded("signer", id) || case.get("error").is_some() {
            continue;
        }
        let req = &case["request"];
        let url = Url::parse(req["url"].as_str().unwrap()).unwrap();
        let signer =
            Signer::new(&spec(id, &case["config"]), &url).unwrap_or_else(|e| panic!("{id}: {e}"));
        let (hdrs, body) = (headers(&req["headers"]), text(&req["data"]));
        let hop = Outgoing {
            method: req["method"].as_str().unwrap(),
            url: &url,
            headers: &hdrs,
            payload: Payload::Text(&body),
        };
        assert_eq!(
            signer.data_to_sign(&hop, NONCE, TIMESTAMP).unwrap(),
            case["dataToSign"].as_str().unwrap(),
            "{id}"
        );
        assert_eq!(
            signer.sign(&hop).unwrap(),
            case["authorization"].as_str().unwrap(),
            "{id}"
        );
        checked += 1;
    }
    assert_eq!(checked, 28);
}

#[test]
fn unrepresentable_vectors_sign_like_their_numeric_twins() {
    for (g, id, _) in EXCLUDED {
        find(g, id);
    }
    for (id, twin) in TWINS {
        assert_eq!(
            find("signer", id)["authorization"],
            find("signer", twin)["authorization"],
            "{id}"
        );
    }
}

#[test]
fn a_blank_nonce_and_timestamp_are_generated_per_signature() {
    let case = find("signer", "nonce-timestamp-blank-generated");
    let url = Url::parse(case["request"]["url"].as_str().unwrap()).unwrap();
    let signer = Signer::new(
        &spec("nonce-timestamp-blank-generated", &case["config"]),
        &url,
    )
    .unwrap();
    let hop = Outgoing {
        method: "GET",
        url: &url,
        headers: &[],
        payload: Payload::Text(""),
    };
    let (a, b) = (signer.sign(&hop).unwrap(), signer.sign(&hop).unwrap());
    let field = |auth: &str, key: &str| {
        auth.split(';')
            .find_map(|p| p.strip_prefix(key))
            .unwrap()
            .to_owned()
    };
    assert!(a.starts_with("EG1-HMAC-SHA256 client_token=akab-client-token-xxx-xxxxxxxxxxxxxxxx;access_token=akab-access-token-xxx-xxxxxxxxxxxxxxxx;timestamp="), "{a}");
    let ts = field(&a, "timestamp=");
    assert!(
        chrono::NaiveDateTime::parse_from_str(&ts, "%Y%m%dT%H:%M:%S+0000").is_ok(),
        "{ts}"
    );
    let nonce = field(&a, "nonce=");
    assert_eq!(
        uuid::Uuid::parse_str(&nonce).unwrap().get_version_num(),
        4,
        "{nonce}"
    );
    assert_eq!(nonce, nonce.to_lowercase());
    assert_ne!(nonce, field(&b, "nonce="));
}

#[test]
fn every_pipeline_vector_signs_like_the_reference() {
    let (url, signer) = pipeline_signer();
    let mut checked = 0;
    for case in group("pipeline") {
        let id = case["id"].as_str().unwrap();
        if excluded("pipeline", id) {
            continue;
        }
        let seen = &case["seen"];
        let hdrs = headers(&seen["headers"]);
        let hop = Outgoing {
            method: seen["method"].as_str().unwrap(),
            url: &url,
            headers: &hdrs,
            payload: Payload::Text(seen["bodyTextHashed"].as_str().unwrap()),
        };
        let sent = &case["sentHeaders"];
        let want = sent["Authorization"]
            .as_str()
            .or(sent["authorization"].as_str())
            .unwrap();
        assert_eq!(signer.sign(&hop).unwrap(), want, "{id}");
        checked += 1;
    }
    assert_eq!(checked, 6);
}

#[test]
fn multipart_and_file_bodies_hash_the_bytes_sent() {
    let (url, signer) = pipeline_signer();
    let multipart = find("pipeline", "multipart")["sentBytes"]
        .as_str()
        .unwrap()
        .as_bytes()
        .to_vec();
    let file = std::fs::read(dir().join("fixture.txt")).unwrap();
    for (id, bytes) in [("multipart", multipart), ("file", file)] {
        let case = find("pipeline", id);
        let hdrs = headers(&case["seen"]["headers"]);
        let hop = Outgoing {
            method: "POST",
            url: &url,
            headers: &hdrs,
            payload: Payload::Bytes(&bytes),
        };
        let hash = content_hash(&signer.data_to_sign(&hop, NONCE, TIMESTAMP).unwrap());
        assert_eq!(hash, STANDARD.encode(Sha256::digest(&bytes)), "{id}");
        assert_ne!(hash, case["contentHash"].as_str().unwrap(), "{id}");
    }
    // A byte body is cut in bytes: 32 ASCII bytes at a limit of 16 hash as the text vector does.
    let twin = find("signer", "post-max-16-number");
    let url = Url::parse(twin["request"]["url"].as_str().unwrap()).unwrap();
    let cut = Signer::new(&spec("post-max-16-number", &twin["config"]), &url).unwrap();
    let body = twin["request"]["data"].as_str().unwrap().as_bytes();
    let hop = Outgoing {
        method: "POST",
        url: &url,
        headers: &[],
        payload: Payload::Bytes(body),
    };
    assert_eq!(
        cut.sign(&hop).unwrap(),
        twin["authorization"].as_str().unwrap()
    );
}

#[test]
fn a_listed_header_that_is_not_sent_is_skipped() {
    let (url, signer) = pipeline_signer();
    let case = find("pipeline", "none");
    let hdrs = headers(&case["seen"]["headers"]);
    let hop = Outgoing {
        method: "POST",
        url: &url,
        headers: &hdrs,
        payload: Payload::Text(""),
    };
    let data = signer.data_to_sign(&hop, NONCE, TIMESTAMP).unwrap();
    let fields: Vec<&str> = data.split('\t').collect();
    let want = case["canonicalHeaders"]
        .as_str()
        .unwrap()
        .replace("content-type:false\t", "");
    assert_eq!(fields[4..fields.len() - 2].join("\t"), want);
    assert_eq!(content_hash(&data), case["contentHash"].as_str().unwrap());
}

#[test]
fn an_empty_query_signs_no_question_mark() {
    let url = Url::parse("https://h.example/p?").unwrap();
    let signer = Signer::new(&spec("q", &Value::Null), &url).unwrap();
    let hop = Outgoing {
        method: "GET",
        url: &url,
        headers: &[],
        payload: Payload::Text(""),
    };
    assert_eq!(
        signer
            .data_to_sign(&hop, NONCE, TIMESTAMP)
            .unwrap()
            .split('\t')
            .nth(3),
        Some("/p")
    );
}

#[test]
fn a_base_url_that_fails_to_parse_on_a_later_hop_is_an_error() {
    // Signer::new validates against hop 1 only; force the state a later scheme could reach.
    let first = Url::parse("https://h.example/p").unwrap();
    let mut s = spec("later", &Value::Null);
    s.base_url = "akab.example".into();
    let mut signer = Signer::new(&s, &first).unwrap();
    signer.base_url = Some("exa mple".into());
    let later = Url::parse("x-y://h.example/p").unwrap();
    let hop = Outgoing {
        method: "GET",
        url: &later,
        headers: &[],
        payload: Payload::Text(""),
    };
    assert!(signer.data_to_sign(&hop, NONCE, TIMESTAMP).is_err());
    assert!(signer.sign(&hop).is_err());
}
