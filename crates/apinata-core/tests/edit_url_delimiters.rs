//! Param text that holds a query delimiter never splits the URL's query, and a URL edit reads
//! it back as the URL text.

use crate::parity;

use apinata_core::{Param, ParamKind, RequestFile};
use parity::{file_with, saved};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

fn request(url: &str, params: &str) -> (TempDir, PathBuf) {
    file_with(&format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\n{params}"
    ))
}

fn query(name: &str, value: &str) -> Param {
    Param::new(name, value, ParamKind::Query)
}

fn off(mut p: Param) -> Param {
    p.enabled = false;
    p
}

fn loaded(url: &str) -> (TempDir, RequestFile) {
    let (dir, path) = request(url, "");
    let r = RequestFile::load(&path).expect("loads");
    (dir, r)
}

// Reference: a URL edit derives the table with decode=false (slices/collections/index.js:1165,
// bruno-common url/index.ts:122), so the escape the table wrote now shows as URL text.
#[test]
fn an_ampersand_in_a_value_is_escaped_and_one_more_keystroke_keeps_one_param() {
    let (_dir, mut r) = loaded("https://x.test/echo");
    r.set_params(vec![query("b", "three & four")]);
    assert_eq!(r.url(), "https://x.test/echo?b=three %26 four");
    assert_eq!(r.params(), vec![query("b", "three & four")]);
    r.set_url("https://x.test/echo?b=three %26 fourx");
    assert_eq!(r.params(), vec![query("b", "three %26 fourx")]);
    r.set_params(r.params());
    assert_eq!(r.url(), "https://x.test/echo?b=three %26 fourx");
}

#[test]
fn each_delimiter_is_escaped_only_where_it_would_split_the_query() {
    let (_dir, mut r) = loaded("https://x.test/");
    let list = vec![
        query("amp", "a&b"),
        query("eq", "a=b"),
        query("hash", "x#y"),
        query("space", "two words"),
        query("pct", "100%"),
        query("uni", "\u{fc}ber"),
        query("empty", ""),
        query("k&1", "v"),
        query("k=2", "v"),
        query("k#3", "v"),
    ];
    r.set_params(list.clone());
    assert_eq!(
        r.url(),
        "https://x.test/?amp=a%26b&eq=a=b&hash=x%23y&space=two words&pct=100%&uni=\u{fc}ber&empty=&k%261=v&k%3D2=v&k%233=v"
    );
    assert_eq!(r.params(), list);
    // Reference: the URL edit re-derives raw URL text (slices/collections/index.js:1165,
    // bruno-common url/index.ts:122), so the written escapes stay escapes.
    let url = format!("{}&z=1", r.url());
    r.set_url(url);
    let expected = vec![
        query("amp", "a%26b"),
        query("eq", "a=b"),
        query("hash", "x%23y"),
        query("space", "two words"),
        query("pct", "100%"),
        query("uni", "\u{fc}ber"),
        query("empty", ""),
        query("k%261", "v"),
        query("k%3D2", "v"),
        query("k%233", "v"),
        query("z", "1"),
    ];
    assert_eq!(r.params(), expected);
}

#[test]
fn templates_are_never_escaped() {
    let (_dir, mut r) = loaded("{{host}}/a");
    r.set_params(vec![query("{{k#1}}", "{{a&b}}&{{c#d}}")]);
    assert_eq!(r.url(), "{{host}}/a?{{k#1}}={{a&b}}%26{{c#d}}");
}

// Reference: parseQueryParams defaults to decode=false (bruno-common url/index.ts:104,122,125).
#[test]
fn no_escape_is_decoded_from_a_typed_url() {
    let (_dir, mut r) = loaded("https://x.test/");
    r.set_url("https://x.test/?q=a%26b%2526%20c%3D%23d&n%3Dx=1");
    assert_eq!(
        r.params(),
        vec![query("q", "a%26b%2526%20c%3D%23d"), query("n%3Dx", "1")]
    );
    let before = r.url().to_owned();
    r.set_params(r.params());
    assert_eq!(r.url(), before);
}

// Reference: the URL edit keeps raw URL text (slices/collections/index.js:1165).
#[test]
fn duplicate_keys_stay_separate_rows_in_order() {
    let (_dir, mut r) = loaded("https://x.test/");
    r.set_params(vec![query("a", "1&2"), query("a", "3")]);
    assert_eq!(r.url(), "https://x.test/?a=1%262&a=3");
    r.set_url("https://x.test/?a=1%262&a=34");
    assert_eq!(r.params(), vec![query("a", "1%262"), query("a", "34")]);
}

// Reference: URL params, then disabled query params (slices/collections/index.js:1165,1216).
#[test]
fn disabled_params_stay_out_of_the_url_and_follow_the_urls_own_after_a_url_edit() {
    let (_dir, mut r) = loaded("https://x.test/");
    let list = vec![
        off(query("a", "1")),
        query("b", "x&y"),
        off(query("c", "#")),
    ];
    r.set_params(list.clone());
    assert_eq!(r.url(), "https://x.test/?b=x%26y");
    assert_eq!(r.params(), list);
    r.set_url("https://x.test/?b=x%26yz");
    assert_eq!(
        r.params(),
        vec![
            query("b", "x%26yz"),
            off(query("a", "1")),
            off(query("c", "#"))
        ]
    );
}

#[test]
fn loading_and_saving_without_an_edit_keeps_every_byte() {
    for (url, params) in [
        (
            "https://x.test/?b=three & four",
            "  params:\n    - name: b\n      value: three & four\n      type: query\n",
        ),
        (
            "https://x.test/?b=three %26 four&c=%2541",
            "  params:\n    - name: b\n      value: three & four\n      type: query\n    - name: c\n      value: \"%2541\"\n      type: query\n",
        ),
        ("https://x.test/?a=1%26b#frag", ""),
    ] {
        let (_dir, path) = request(url, params);
        let before = fs::read(&path).unwrap();
        let mut r = RequestFile::load(&path).expect("loads");
        let _ = r.shown_params();
        r.set_url(url);
        assert!(!r.is_dirty(), "{url}");
        r.save().expect("saves");
        assert_eq!(fs::read(&path).unwrap(), before, "{url}");
    }
}

// Reference: a URL edit derives the table with decode=false (bruno-common url/index.ts:122).
#[test]
fn an_escape_typed_into_a_value_reads_back_as_typed_after_a_url_edit() {
    let (_dir, mut r) = loaded("https://x.test/");
    r.set_params(vec![query("a", "%26")]);
    assert_eq!(r.url(), "https://x.test/?a=%26");
    r.set_url("https://x.test/?a=%26&z=1");
    assert_eq!(r.params(), vec![query("a", "%26"), query("z", "1")]);
    r.set_params(r.params());
    assert_eq!(r.url(), "https://x.test/?a=%26&z=1");
}

/// Each typed query value and the table value the reference derives for it, unchanged.
const RAW: &[(&str, &str)] = &[
    ("v=a%26b", "a%26b"),
    ("v=a%23b", "a%23b"),
    ("v=a%3Db", "a%3Db"),
    ("v=a%20b", "a%20b"),
    ("v=a+b", "a+b"),
];

// Reference: requestUrlChanged stores the URL verbatim and parses with decode=false
// (slices/collections/index.js:1160,1165, bruno-common url/index.ts:122); the save copies both
// as held (utils/collections/index.js:748,776-783).
#[test]
fn a_typed_escape_shows_saves_and_reloads_as_url_text() {
    for (typed, value) in RAW {
        let (_dir, path) = request("https://x.test/", "");
        let mut r = RequestFile::load(&path).expect("loads");
        let url = format!("https://x.test/?{typed}");
        r.set_url(url.as_str());
        assert_eq!(r.params(), vec![query("v", value)], "{typed}");
        r.save().expect("saves");
        let file = saved(&path);
        assert_eq!(file["http"]["url"].as_str(), Some(url.as_str()), "{typed}");
        assert_eq!(
            file["http"]["params"][0]["value"].as_str(),
            Some(*value),
            "{typed}"
        );
        let r = RequestFile::load(&path).expect("reloads");
        assert_eq!(r.url(), url, "{typed}");
        assert_eq!(r.params(), vec![query("v", value)], "{typed}");
    }
}

// Reference: Keep URL is the same derivation as a URL edit (bruno-common url/index.ts:122).
#[test]
fn keeping_a_mismatched_url_takes_its_query_as_url_text() {
    for (typed, value) in RAW {
        let params = "  params:\n    - name: v\n      value: other\n      type: query\n";
        let (_dir, path) = request(&format!("https://x.test/?{typed}"), params);
        let mut r = RequestFile::load(&path).expect("loads");
        assert!(r.query_mismatch().is_some(), "{typed}");
        r.use_url_query();
        assert_eq!(r.params(), vec![query("v", value)], "{typed}");
    }
}

// Reference: names are parsed with decode=false too (bruno-common url/index.ts:125).
#[test]
fn an_escaped_equals_in_a_name_stays_url_text() {
    let (_dir, mut r) = loaded("https://x.test/");
    r.set_url("https://x.test/?n%3Dx=1");
    assert_eq!(r.params(), vec![query("n%3Dx", "1")]);
}
