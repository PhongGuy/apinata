//! A request file whose URL query and params block disagree, and the two ways to settle it.

use crate::parity;

use apinata_core::{Param, ParamKind, RequestFile};
use parity::file_with;
use std::fs;

fn load(url: &str, params: &str) -> (tempfile::TempDir, std::path::PathBuf, RequestFile) {
    let (dir, path) = file_with(&format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\n{params}"
    ));
    let r = RequestFile::load(&path).expect("loads");
    (dir, path, r)
}

fn names(r: &RequestFile) -> Option<Vec<String>> {
    r.query_mismatch().map(|m| m.names)
}

fn param(name: &str, value: &str, kind: &str) -> String {
    format!("    - name: {name}\n      value: \"{value}\"\n      type: {kind}\n")
}

fn block(items: &[String]) -> String {
    format!("  params:\n{}", items.concat())
}

#[test]
fn a_disagreeing_value_names_its_param() {
    let (_d, _p, r) = load("https://x.test/?z=6", &block(&[param("z", "3", "query")]));
    assert_eq!(names(&r), Some(vec!["z".to_owned()]));
}

#[test]
fn a_file_without_params_never_disagrees() {
    let (_d, _p, r) = load("https://x.test/?z=6&a", "");
    assert_eq!(names(&r), None);
}

#[test]
fn the_same_pairs_in_another_order_agree() {
    let (_d, _p, r) = load(
        "https://x.test/:id?b={{x}}&a=1",
        &block(&[
            param("a", "1", "query"),
            param("id", "7", "path"),
            param("b", "{{x}}", "query"),
            format!("{}      disabled: true\n", param("off", "1", "query")),
        ]),
    );
    assert_eq!(names(&r), None);
}

#[test]
fn names_differ_as_a_multiset_url_first_without_repeats() {
    let (_d, _p, r) = load(
        "https://x.test/?b=1&a=1&a=1&k=1",
        &block(&[
            param("a", "1", "query"),
            param("k", "1", "query"),
            param("c", "1", "query"),
            param("b", "2", "query"),
        ]),
    );
    assert_eq!(
        names(&r),
        Some(vec!["b".to_owned(), "a".to_owned(), "c".to_owned()])
    );
}

#[test]
fn escaped_delimiters_written_by_the_table_still_agree() {
    let (_d, path, mut r) = load("https://x.test/", "");
    r.set_params(vec![Param::new("a&b", "c=d#e", ParamKind::Query)]);
    r.save().expect("saves");
    let r = RequestFile::load(&path).expect("reloads");
    assert_eq!(names(&r), None);
}

#[test]
fn blank_named_rows_in_the_table_agree_with_the_url() {
    let (_d, _p, mut r) = load("https://x.test/?a=1", "");
    r.set_params(vec![
        Param::new("a", "1", ParamKind::Query),
        Param::new("", "", ParamKind::Query),
    ]);
    assert_eq!(names(&r), None);
}

#[test]
fn keeping_the_url_rebuilds_the_table_from_it() {
    let (_d, _p, mut r) = load(
        "https://x.test/u/:id?z=6",
        &block(&[
            param("z", "3", "query"),
            format!("{}      disabled: true\n", param("y", "1", "query")),
            param("id", "7", "path"),
        ]),
    );
    r.use_url_query();
    let mut off = Param::new("y", "1", ParamKind::Query);
    off.enabled = false;
    assert_eq!(r.url(), "https://x.test/u/:id?z=6");
    assert_eq!(
        r.params(),
        vec![
            Param::new("z", "6", ParamKind::Query),
            off,
            Param::new("id", "7", ParamKind::Path),
        ]
    );
    assert_eq!(names(&r), None);
    assert!(r.is_dirty());
}

#[test]
fn keeping_the_table_rebuilds_the_url_query_from_it() {
    let (_d, _p, mut r) = load(
        "https://x.test/u/:id?z=6",
        &block(&[param("z", "3", "query"), param("id", "7", "path")]),
    );
    r.use_params_query();
    assert_eq!(r.url(), "https://x.test/u/:id?z=3");
    assert_eq!(names(&r), None);
    assert!(r.is_dirty());
}

#[test]
fn settling_an_agreeing_request_changes_nothing() {
    let (_d, path, mut r) = load(
        "https://x.test/?b=2&a={{x}}",
        &block(&[param("a", "{{x}}", "query"), param("b", "2", "query")]),
    );
    let before = fs::read(&path).unwrap();
    r.use_url_query();
    r.use_params_query();
    assert!(!r.is_dirty());
    r.save().expect("saves");
    assert_eq!(fs::read(&path).unwrap(), before);
}

// The table writes `&`, `#` and a name's `=` as escapes, so text already looking like one of
// those escapes is the user's literal text and must not read back as the delimiter.
#[test]
fn literal_escape_text_typed_in_the_table_still_agrees() {
    for (name, value) in [("v", "a%26b"), ("v", "a%23b"), ("n%3Dx", "1")] {
        let (_d, _p, mut r) = load("https://x.test/", "");
        r.set_params(vec![Param::new(name, value, ParamKind::Query)]);
        assert_eq!(names(&r), None, "{name}={value} gave {}", r.url());
    }
}

// Reference: Keep URL derives with decode=false, so escapes stay as written (bruno-common
// url/index.ts:122,125).
#[test]
fn keeping_the_url_keeps_its_escapes_as_written() {
    let (_d, _p, mut r) = load(
        "https://x.test/?a%3Db=c%26d",
        &block(&[param("a=b", "x", "query")]),
    );
    assert_eq!(names(&r), Some(vec!["a%3Db".to_owned()]));
    r.use_url_query();
    assert_eq!(
        r.params(),
        vec![Param::new("a%3Db", "c%26d", ParamKind::Query)]
    );
    assert_eq!(names(&r), None);
}

// Reference: the URL's names are its raw text (bruno-common url/index.ts:125), so a mismatch
// names them that way too.
#[test]
fn a_mismatch_names_a_param_as_the_url_writes_it() {
    let (_d, _p, r) = load(
        "https://x.test/?k%26=1",
        &block(&[param("k&", "2", "query")]),
    );
    assert_eq!(names(&r), Some(vec!["k%26".to_owned()]));
}
