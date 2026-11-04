//! WSSE and Akamai EdgeGrid in the HTML, OpenAPI and Postman exports.

use crate::export_support;

use apinata_core::AuthSpec;
use apinata_core::Collection;
use apinata_core::export::postman;
use apinata_core::import::parse_postman;
use export_support::*;
use serde_json::json;
use tempfile::TempDir;

const WSSE: &str = "  auth:\n    type: wsse\n    username: u\n    password: wsse-pw-secret\n";
const EDGEGRID: &str = "  auth:\n    type: akamai-edgegrid\n    accessToken: at\n    clientToken: ct\n    clientSecret: eg-secret\n    nonce: \"\"\n    timestamp: \"\"\n    baseURL: \"\"\n    headersToSign: \"\"\n    maxBodySize: 4096\n";

fn both() -> (TempDir, Collection) {
    open(&[
        ("opencollection.yml", ROOT),
        ("w.yml", &http_at("w", 1, "GET", "https://x.test/w", WSSE)),
        (
            "e.yml",
            &http_at("e", 2, "GET", "https://x.test/e", EDGEGRID),
        ),
    ])
}

#[test]
fn html_labels_name_both_modes_and_hide_their_secrets() {
    let (_d, c) = both();
    let page = html_page(&c);
    assert!(page.contains("<p class=\"auth\">Auth: WSSE</p>"), "{page}");
    assert!(
        page.contains("<p class=\"auth\">Auth: Akamai EdgeGrid</p>"),
        "{page}"
    );
    assert!(!page.contains("wsse-pw-secret") && !page.contains("eg-secret"));
}

#[test]
fn openapi_takes_the_unmappable_path_for_both() {
    let (_d, c) = both();
    let mut got = openapi_notes(&c);
    got.sort();
    assert_eq!(
        got,
        pairs(&[
            (
                "e / auth",
                "the akamai-edgegrid authentication cannot be exported"
            ),
            ("w / auth", "the wsse authentication cannot be exported"),
        ])
    );
}

#[test]
fn postman_carries_edgegrid_and_notes_wsse() {
    let (_d, c) = both();
    let v = exported(&c);
    let item = |name: &str| {
        v["item"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["name"] == name)
            .unwrap()
            .clone()
    };
    let s = |k: &str, val: &str| json!({"key": k, "value": val, "type": "string"});
    assert_eq!(
        item("e")["request"]["auth"],
        json!({"type": "edgegrid", "edgegrid": [s("accessToken", "at"), s("clientToken", "ct"), s("clientSecret", "eg-secret"),
            s("baseURL", ""), s("nonce", ""), s("timestamp", ""), s("headersToSign", ""),
            {"key": "maxBodySize", "value": 4096, "type": "string"}]})
    );
    assert_eq!(item("w")["request"]["auth"], json!({"type": "noauth"}));
    assert_eq!(
        notes(&c),
        pairs(&[("w / auth", "the wsse authentication cannot be exported")])
    );
}

#[test]
fn edgegrid_survives_the_postman_round_trip() {
    for max in ["", "    maxBodySize: 1.5\n", "    maxBodySize: 0\n"] {
        let auth = format!(
            "  auth:\n    type: akamai-edgegrid\n    accessToken: at\n    clientToken: ct\n    clientSecret: cs\n    nonce: n\n    timestamp: t\n    baseURL: https://b.test\n    headersToSign: X-A\n{max}"
        );
        let (_d, c) = open(&[
            ("opencollection.yml", ROOT),
            ("e.yml", &http_at("e", 1, "GET", "https://x.test/e", &auth)),
        ]);
        let back = parse_postman(postman(&c).json.as_bytes()).expect("parses");
        let theirs = imported_requests(&back.items, "");
        let ours = source_requests(c.tree(), "");
        assert_eq!(theirs[0].1.request.auth, ours[0].1.auth(), "{max:?}");
        assert!(matches!(theirs[0].1.request.auth, AuthSpec::EdgeGrid(_)));
    }
}

#[test]
fn a_huge_max_body_size_exports_with_js_number_spelling() {
    let auth = EDGEGRID.replace("maxBodySize: 4096", "maxBodySize: 1.0e+21");
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        ("e.yml", &http_at("e", 1, "GET", "https://x.test/e", &auth)),
    ]);
    let json = postman(&c).json;
    assert!(json.contains("\"value\": 1e+21"), "{json}");
}
