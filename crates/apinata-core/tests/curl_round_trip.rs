//! `to_curl` output imports back into the same request.

use crate::common;

use apinata_core::import::parse_curl;
use apinata_core::{BodySpec, KeyValue, MultipartField, ResolvedRequest, to_curl};
use common::resolved_one;

fn round(r: &ResolvedRequest) -> apinata_core::import::ImportedRequest {
    let command = to_curl(r);
    parse_curl(&command)
        .unwrap_or_else(|e| panic!("{command}: {e}"))
        .request
}
fn headers(r: &ResolvedRequest) -> Vec<(String, String)> {
    r.headers()
        .iter()
        .filter(|(n, v)| {
            !(n.eq_ignore_ascii_case("content-type") && v.starts_with("multipart/form-data"))
        })
        .cloned()
        .collect()
}
fn imported_headers(i: &apinata_core::import::ImportedRequest) -> Vec<(String, String)> {
    i.headers
        .iter()
        .map(|h| (h.name.clone(), h.value.clone()))
        .collect()
}

#[test]
fn method_url_headers_and_auth_come_back() {
    let under = "  headers:\n    - name: X-Q\n      value: \"it's $HOME `x` \\\\ \\\"q\\\"\"\n    - name: X-Empty\n      value: \"\"\n  auth:\n    type: apikey\n    key: X-Key\n    value: k1\n    placement: header\n";
    for method in ["GET", "DELETE", "PATCH"] {
        let r = resolved_one(method, "http://x.test/a?b=1&c=%20d", under, "");
        let i = round(&r);
        assert_eq!(i.method, method);
        assert_eq!(i.url, r.url());
        assert_eq!(imported_headers(&i), headers(&r));
    }
}

#[test]
fn text_json_and_form_bodies_come_back() {
    let json = "  body:\n    type: json\n    data: \"{\\n  \\\"a\\\": 1\\n}\"\n";
    let i = round(&resolved_one("POST", "http://x.test/", json, ""));
    assert_eq!(i.body, BodySpec::Json("{\n  \"a\": 1\n}".into()));
    let text = "  body:\n    type: text\n    data: \"line'1\\nline 2\"\n";
    let i = round(&resolved_one("POST", "http://x.test/", text, ""));
    assert_eq!(i.body, BodySpec::Text("line'1\nline 2".into()));
    let form = "  body:\n    type: form-urlencoded\n    data:\n      - name: a\n        value: \"1\"\n      - name: b\n        value: x y&z=é\n";
    let i = round(&resolved_one("POST", "http://x.test/", form, ""));
    assert_eq!(
        i.body,
        BodySpec::FormUrlEncoded(vec![KeyValue::new("a", "1"), KeyValue::new("b", "x y&z=é")])
    );
}

#[test]
fn multipart_and_file_bodies_come_back() {
    let mp = "  body:\n    type: multipart-form\n    data:\n      - name: a\n        type: text\n        value: \"1\"\n      - name: s\n        type: text\n        value: \"x;y \\\"q\\\"\"\n      - name: at\n        type: text\n        value: \"@notafile\"\n      - name: up\n        type: file\n        value: [/tmp/up.bin]\n";
    let r = resolved_one("POST", "http://x.test/", mp, "");
    let i = round(&r);
    assert_eq!(imported_headers(&i), headers(&r));
    assert_eq!(
        i.body,
        BodySpec::Multipart(vec![
            MultipartField::text("a", "1"),
            MultipartField::text("s", "x;y \"q\""),
            MultipartField::text("at", "@notafile"),
            MultipartField::files("up", vec!["/tmp/up.bin".to_owned()]),
        ])
    );
    let file = "  body:\n    type: file\n    data:\n      - filePath: /tmp/b.bin\n        contentType: application/octet-stream\n        selected: true\n";
    let i = round(&resolved_one("POST", "http://x.test/", file, ""));
    let BodySpec::File(entries) = &i.body else {
        panic!("{:?}", i.body)
    };
    assert_eq!(entries[0].path, "/tmp/b.bin");
    assert_eq!(entries[0].content_type, "application/octet-stream");
}

#[test]
fn form_values_with_edge_whitespace_come_back() {
    let mp = "  body:\n    type: multipart-form\n    data:\n      - name: l\n        type: text\n        value: \" lead\"\n      - name: t\n        type: text\n        value: \"trail  \"\n";
    let i = round(&resolved_one("POST", "http://x.test/", mp, ""));
    assert_eq!(
        i.body,
        BodySpec::Multipart(vec![
            MultipartField::text("l", " lead"),
            MultipartField::text("t", "trail  ")
        ])
    );
}
