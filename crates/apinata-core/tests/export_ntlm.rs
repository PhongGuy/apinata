//! NTLM in the OpenAPI and HTML exports: no scheme, a note, a label and no credential.

use crate::export_support;

use export_support::*;
use serde_json::json;

const NTLM: &str = "  auth:\n    type: ntlm\n    username: u\n    password: ntlm-pw-secret\n    domain: ntlm-dom-secret\n";

#[test]
fn openapi_declares_no_scheme_for_ntlm_and_notes_it() {
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        ("n.yml", &http_at("n", 1, "GET", "https://x.test/n", NTLM)),
    ]);
    let doc = openapi_doc(&c);
    assert!(
        doc["components"].get("securitySchemes").is_none(),
        "{}",
        doc["components"]
    );
    assert_eq!(
        openapi_notes(&c),
        pairs(&[("n / auth", "the NTLM authentication cannot be exported")])
    );
    let text = doc.to_string();
    assert!(!text.contains("ntlm-pw-secret") && !text.contains("ntlm-dom-secret"));
    assert_ne!(doc["openapi"], json!(null));
}

#[test]
fn html_labels_ntlm_and_hides_the_credentials() {
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        ("n.yml", &http_at("n", 1, "GET", "https://x.test/n", NTLM)),
    ]);
    let page = html_page(&c);
    assert!(page.contains("<p class=\"auth\">Auth: NTLM</p>"), "{page}");
    assert!(!page.contains("ntlm-pw-secret") && !page.contains("ntlm-dom-secret"));
}
