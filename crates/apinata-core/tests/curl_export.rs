//! `to_curl` command text.

use crate::common;

use apinata_core::to_curl;
use common::resolved_one;

const NL: &str = " \\\n  ";

fn lines(parts: &[&str]) -> String {
    parts.join(NL)
}

#[test]
fn a_get_has_method_and_url_only() {
    let r = resolved_one("GET", "http://x.test/a?b=1", "", "");
    assert_eq!(
        to_curl(&r),
        lines(&["curl --request 'GET'", "--url 'http://x.test/a?b=1'"])
    );
}

#[test]
fn head_uses_the_head_flag_and_brackets_turn_globbing_off() {
    let r = resolved_one("HEAD", "http://x.test/a?f[0]=1", "", "");
    assert_eq!(
        to_curl(&r),
        lines(&["curl --head", "--url 'http://x.test/a?f[0]=1'", "--globoff"])
    );
}

#[test]
fn quotes_are_escaped_posix_style() {
    let under = "  headers:\n    - name: X-Q\n      value: \"it's\"\n    - name: X-Empty\n      value: \"\"\n  body:\n    type: text\n    data: \"a'b\\nc\"\n";
    let r = resolved_one("POST", "http://x.test/", under, "");
    let out = to_curl(&r);
    assert!(
        out.contains(&format!("{NL}--header 'X-Q: it'\\''s'")),
        "{out}"
    );
    assert!(out.contains(&format!("{NL}--header 'X-Empty;'")), "{out}");
    assert!(
        out.ends_with(&format!("{NL}--data-raw 'a'\\''b\nc'")),
        "{out}"
    );
}

#[test]
fn auth_appears_as_the_resolved_header() {
    let r = resolved_one(
        "GET",
        "http://x.test/",
        "  auth:\n    type: bearer\n    token: t0k\n",
        "",
    );
    assert!(
        to_curl(&r).contains("--header 'Authorization: Bearer t0k'"),
        "{}",
        to_curl(&r)
    );
    let r = resolved_one(
        "GET",
        "http://x.test/",
        "  auth:\n    type: basic\n    username: u\n    password: p\n",
        "",
    );
    assert!(
        to_curl(&r).contains("--header 'Authorization: Basic dTpw'"),
        "{}",
        to_curl(&r)
    );
}

#[test]
fn a_form_body_is_encoded_like_the_executor() {
    let under = "  body:\n    type: form-urlencoded\n    data:\n      - name: a\n        value: \"1\"\n      - name: b\n        value: x y&z\n";
    let r = resolved_one("POST", "http://x.test/", under, "");
    let out = to_curl(&r);
    assert!(out.ends_with("--data-raw 'a=1&b=x+y%26z'"), "{out}");
    assert!(
        out.to_lowercase()
            .contains("--header 'content-type: application/x-www-form-urlencoded'"),
        "{out}"
    );
}

#[test]
fn multipart_parts_become_form_flags_without_the_bare_content_type() {
    let under = "  body:\n    type: multipart-form\n    data:\n      - name: a\n        type: text\n        value: \"1\"\n      - name: s\n        type: text\n        value: \"x;y\"\n      - name: up\n        type: file\n        value: [/tmp/up.bin]\n        contentType: text/plain\n";
    let r = resolved_one("POST", "http://x.test/", under, "");
    let out = to_curl(&r);
    assert!(!out.to_lowercase().contains("multipart/form-data"), "{out}");
    assert!(out.contains(&format!("{NL}--form 'a=1'")), "{out}");
    assert!(out.contains(&format!("{NL}--form 's=\"x;y\"'")), "{out}");
    assert!(
        out.ends_with(&format!("{NL}--form 'up=@/tmp/up.bin;type=text/plain'")),
        "{out}"
    );
}

#[test]
fn a_file_body_is_sent_from_the_file() {
    let under = "  body:\n    type: file\n    data:\n      - filePath: /tmp/b.json\n        contentType: application/json\n        selected: true\n";
    let r = resolved_one("POST", "http://x.test/", under, "");
    let out = to_curl(&r);
    assert!(
        out.ends_with(&format!("{NL}--data-binary '@/tmp/b.json'")),
        "{out}"
    );
    assert!(
        out.to_lowercase()
            .contains("content-type: application/json"),
        "{out}"
    );
}

#[test]
fn form_values_with_edge_whitespace_are_quoted() {
    let under = "  body:\n    type: multipart-form\n    data:\n      - name: l\n        type: text\n        value: \" lead\"\n      - name: t\n        type: text\n        value: \"trail  \"\n";
    let r = resolved_one("POST", "http://x.test/", under, "");
    let out = to_curl(&r);
    assert!(out.contains(&format!("{NL}--form 'l=\" lead\"'")), "{out}");
    assert!(
        out.contains(&format!("{NL}--form 't=\"trail  \"'")),
        "{out}"
    );
}

#[test]
fn a_head_with_a_body_names_the_method_instead_of_the_head_flag() {
    let under = "  body:\n    type: text\n    data: x\n";
    let r = resolved_one("HEAD", "http://x.test/", under, "");
    let out = to_curl(&r);
    assert!(out.starts_with("curl --request 'HEAD'"), "{out}");
    assert!(!out.contains("--head "), "{out}");
}
