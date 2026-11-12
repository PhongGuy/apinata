//! URL and params kept in sync as Bruno's request reducers keep them.

use crate::parity;

use apinata_core::{Param, ParamKind, RequestFile};
use parity::{check_requests, file_with};
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

fn path_param(name: &str, value: &str) -> Param {
    Param::new(name, value, ParamKind::Path)
}

fn off(mut p: Param) -> Param {
    p.enabled = false;
    p
}

#[test]
fn url_edits_match_bruno() {
    check_requests("url-", |id, r| match id {
        "url-adds-query" => r.set_url("https://api.example.com/one?a=1&b"),
        "url-adds-path" => r.set_url("https://api.example.com/users/:id/posts/:postId"),
        "url-drops-query" => r.set_url("{{localhost}}/api/wait-for/body"),
        "url-template-host-path" => r.set_url("{{host}}/users/:id"),
        "url-template-query" => r.set_url("{{host}}/search?q={{term}}&page=2"),
        "url-fragment" => r.set_url("https://x.test/a?q=1#frag"),
        "url-keeps-disabled-query" => {
            r.set_params(vec![off(query("time", "1000"))]);
            r.set_url("{{localhost}}/api/wait-for/body?x=2");
        }
        "url-keeps-path-value" => {
            r.set_url("https://x.test/users/:id");
            r.set_params(vec![path_param("id", "42")]);
            r.set_url("https://x.test/users/:id/posts");
        }
        "url-readds-disabled-path" => {
            r.set_url("https://x.test/u/:id");
            r.set_params(vec![off(path_param("id", "7"))]);
            r.set_url("https://x.test/u/:id?q=1");
        }
        other => panic!("no Apinata edit for {other}"),
    });
}

#[test]
fn params_edits_match_bruno() {
    check_requests("params-", |id, r| match id {
        "params-set-query" => r.set_params(vec![query("time", "2000"), query("extra", "x")]),
        "params-disable-all" => r.set_params(vec![off(query("time", "1000"))]),
        other => panic!("no Apinata edit for {other}"),
    });
}

#[test]
fn params_read_kind_and_state() {
    let params = "  params:\n    - name: q\n      value: 1\n    - name: id\n      value: x\n      type: path\n      disabled: true\n";
    let (_dir, path) = request("u/:id?q=1", params);
    let r = RequestFile::load(&path).expect("loads");
    assert_eq!(
        r.params(),
        vec![query("q", "1"), off(path_param("id", "x"))]
    );
}

#[test]
fn a_url_edit_keeps_disabled_query_params_after_the_urls_own() {
    let params = "  params:\n    - name: a\n      value: \"1\"\n      type: query\n    - name: old\n      value: o\n      type: query\n      disabled: true\n";
    let (_dir, path) = request("https://x.test/?a=1", params);
    let mut r = RequestFile::load(&path).expect("loads");
    r.set_url("https://x.test/?b=2&a=3");
    assert_eq!(
        r.params(),
        vec![query("b", "2"), query("a", "3"), off(query("old", "o"))]
    );
    assert_eq!(r.url(), "https://x.test/?b=2&a=3");
}

#[test]
fn a_url_edit_drops_path_params_the_url_no_longer_uses() {
    let params = "  params:\n    - name: id\n      value: \"9\"\n      type: path\n    - name: gone\n      value: g\n      type: path\n";
    let (_dir, path) = request("https://x.test/:id/:gone", params);
    let mut r = RequestFile::load(&path).expect("loads");
    r.set_url("https://x.test/:id/:a/:b");
    assert_eq!(
        r.params(),
        vec![
            path_param("a", ""),
            path_param("b", ""),
            path_param("id", "9")
        ]
    );
}

#[test]
fn renaming_the_only_path_param_keeps_its_value_and_undoing_it_is_clean() {
    let params = "  params:\n    - name: id\n      value: \"{{postId}}\"\n      type: path\n";
    let (_dir, path) = request("{{baseUrl}}/posts/:id", params);
    let mut r = RequestFile::load(&path).expect("loads");
    r.set_url("{{baseUrl}}/posts/:idx");
    assert_eq!(r.params(), vec![path_param("idx", "{{postId}}")]);
    r.set_url("{{baseUrl}}/posts/:id");
    assert_eq!(r.params(), vec![path_param("id", "{{postId}}")]);
    assert!(!r.is_dirty());
}

#[test]
fn a_renamed_path_param_keeps_its_row_among_the_others() {
    let params = "  params:\n    - name: q\n      value: \"1\"\n      type: query\n    - name: a\n      value: x\n      type: path\n    - name: b\n      value: y\n      type: path\n      description: why\n";
    let (_dir, path) = request("https://x.test/:a/:b?q=1", params);
    let mut r = RequestFile::load(&path).expect("loads");
    r.set_url("https://x.test/:a/:bz?q=1");
    let mut renamed = path_param("bz", "y");
    renamed.description = Some("why".to_owned());
    assert_eq!(
        r.params(),
        vec![query("q", "1"), path_param("a", "x"), renamed]
    );
    r.set_url("https://x.test/:a/:b?q=1");
    assert!(!r.is_dirty());
}

#[test]
fn replacing_more_than_one_path_param_starts_the_new_ones_empty() {
    let params = "  params:\n    - name: a\n      value: x\n      type: path\n    - name: b\n      value: y\n      type: path\n";
    let (_dir, path) = request("https://x.test/:a/:b", params);
    let mut r = RequestFile::load(&path).expect("loads");
    r.set_url("https://x.test/:c/:d");
    assert_eq!(r.params(), vec![path_param("c", ""), path_param("d", "")]);
}

#[test]
fn setting_the_same_url_does_not_touch_params() {
    // The params are out of sync with the URL, and stay that way until the URL changes.
    let params = "  params:\n    - name: stale\n      value: s\n      type: query\n";
    let (_dir, path) = request("https://x.test/?a=1", params);
    let mut r = RequestFile::load(&path).expect("loads");
    r.set_url("https://x.test/?a=1");
    assert!(!r.is_dirty());
}

#[test]
fn set_params_rebuilds_the_query_and_keeps_the_rest_of_the_url() {
    let (_dir, path) = request("{{host}}/a?{{q}}=1", "");
    let mut r = RequestFile::load(&path).expect("loads");
    r.set_params(vec![
        query("x", "1"),
        path_param("id", "2"),
        off(query("y", "3")),
        query(" ", "blank"),
    ]);
    assert_eq!(r.url(), "{{host}}/a?x=1");
    r.set_params(Vec::new());
    assert_eq!(r.url(), "{{host}}/a");
    assert!(r.params().is_empty());
}

#[test]
fn a_query_param_keeps_its_unknown_keys_through_set_params() {
    let params =
        "  params:\n    - name: a\n      value: \"1\"\n      type: query\n      description: why\n";
    let (_dir, path) = request("https://x.test/?a=1", params);
    let mut r = RequestFile::load(&path).expect("loads");
    let mut list = r.params();
    list[0].value = "2".to_owned();
    r.set_params(list);
    r.save().expect("saves");
    let after: yaml_serde::Value =
        yaml_serde::from_str(&fs::read_to_string(&path).expect("read")).expect("parses");
    assert_eq!(
        after["http"]["params"][0]["description"],
        yaml_serde::Value::from("why")
    );
    assert_eq!(
        after["http"]["url"],
        yaml_serde::Value::from("https://x.test/?a=2")
    );
}

#[test]
fn debug_shows_param_names_and_sizes_only() {
    let shown = format!("{:?}", query("api_key", "hunter2"));
    assert!(
        shown.contains("api_key") && shown.contains("7 bytes"),
        "{shown}"
    );
    assert!(!shown.contains("hunter2"), "{shown}");
}

#[test]
fn a_file_without_params_shows_its_url_query_and_path_as_params() {
    let (_dir, path) = request("https://x.test/u/:id?a=1&b", "");
    let before = fs::read(&path).unwrap();
    let r = RequestFile::load(&path).expect("loads");
    assert_eq!(r.params(), vec![]);
    assert_eq!(
        r.shown_params(),
        vec![query("a", "1"), query("b", ""), path_param("id", "")]
    );
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn a_params_list_wins_over_the_url_even_when_empty() {
    let (_dir, path) = request("https://x.test/?a=1", "  params: []\n");
    let r = RequestFile::load(&path).expect("loads");
    assert_eq!(r.shown_params(), vec![]);
}

#[test]
fn saving_untouched_after_showing_derived_params_keeps_the_bytes() {
    let (_dir, path) = request("https://x.test/u/:id?a=1&b", "");
    let before = fs::read(&path).unwrap();
    let mut r = RequestFile::load(&path).expect("loads");
    assert_eq!(r.shown_params().len(), 3);
    r.save().expect("saves");
    assert_eq!(fs::read(&path).unwrap(), before);
}
