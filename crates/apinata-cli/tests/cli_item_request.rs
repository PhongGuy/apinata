//! `apinata item request`: each edit leaves the collection exactly as MCP `update_request` does
//! with the equivalent sections. One fixture is copied twice; the CLI edits one copy,
//! `apinata mcp` the other, and the trees must match byte for byte.

use crate::cli_item::{Copy, stdout};
use crate::common;

use std::collections::BTreeMap;
use std::fs;
use std::time::Duration;

use common::{apinata, write_tree};
use predicates::str::contains;
use serde_json::{Value, json};
use tempfile::TempDir;

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: shop\n";

const GET: &str = "info:\n  name: get\n  type: http\n  seq: 1\n  tags:\n    - smoke\nhttp:\n  method: GET\n  url: http://localhost/users/:id?page=1\n  params:\n    - name: page\n      value: \"1\"\n      type: query\n    - name: id\n      value: \"7\"\n      type: path\n  headers:\n    - name: X-A\n      value: \"1\"\n    - name: X-Old\n      value: o\n  body:\n    type: json\n    data: '{\"a\":1}'\n  auth:\n    type: bearer\n    token: t\nruntime:\n  variables:\n    - name: host\n      value: localhost\n    - name: n\n      value:\n        type: number\n        data: \"1\"\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\ndocs: notes\n";

const FORM: &str = "info:\n  name: form\n  type: http\n  seq: 2\nhttp:\n  method: POST\n  url: http://localhost/form\n  body:\n    type: form-urlencoded\n    data:\n      - name: a\n        value: \"1\"\n      - name: old\n        value: o\n";

const MULTI: &str = "info:\n  name: multi\n  type: http\n  seq: 3\nhttp:\n  method: POST\n  url: http://localhost/up\n  body:\n    type: multipart-form\n    data:\n      - name: t\n        type: text\n        value: x\n";

const GQL: &str = "info:\n  name: gql\n  type: graphql\n  seq: 4\ngraphql:\n  method: POST\n  url: http://localhost/graphql\n  body:\n    query: \"{ a }\"\n";

const SKEW: &str = "info:\n  name: skew\n  type: http\n  seq: 5\nhttp:\n  method: GET\n  url: http://localhost/s?a=1\n  params:\n    - name: a\n      value: \"2\"\n      type: query\n";

fn fixture() -> TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/get.yml", GET),
        ("form.yml", FORM),
        ("multi.yml", MULTI),
        ("gql.yml", GQL),
        ("skew.yml", SKEW),
    ])
}

fn copy() -> Copy {
    Copy::of(&fixture())
}

/// Run `item request <path> <cli>` on one copy and `update_request` with `path` plus `args`
/// on another; the trees must match and differ from the fixture. Returns the CLI's stdout and
/// the tree.
async fn parity(path: &str, cli: &[&str], args: Value) -> (String, BTreeMap<String, String>) {
    let a = copy();
    let b = copy();
    let mut all = vec!["request", path];
    all.extend_from_slice(cli);
    let out = stdout(&a.item(&all).code(0));
    let Value::Object(mut args) = args else {
        panic!("sections must be an object")
    };
    args.insert("path".into(), json!(path));
    b.mcp("update_request", Value::Object(args))
        .await
        .expect("the tool succeeds");
    let tree = a.tree();
    assert_eq!(tree, b.tree(), "cli {cli:?} and update_request differ");
    assert_ne!(tree, copy().tree(), "the edit changed something");
    (out, tree)
}

#[tokio::test]
async fn name_method_and_url_match_update_request() {
    let (out, tree) = parity(
        "users/get.yml",
        &[
            "--name",
            "Get one",
            "--method",
            "post",
            "--url",
            "{{base}}/users/:id?page=2",
        ],
        json!({"name": "Get one", "method": "post", "url": "{{base}}/users/:id?page=2"}),
    )
    .await;
    assert_eq!(out, "updated users/get.yml\n");
    let file = &tree["users/get.yml"];
    assert!(file.contains("method: POST"), "{file}");
    assert!(
        file.contains("url: \"{{base}}/users/:id?page=2\""),
        "{file}"
    );
}

#[tokio::test]
async fn query_and_path_params_match_update_request() {
    parity(
        "users/get.yml",
        &[
            "--query",
            "page=3",
            "--query",
            "q={{term}}",
            "--disable-query",
            "q",
            "--path-param",
            "id=9",
        ],
        json!({"params": [
            {"name": "page", "value": "3", "kind": "query", "enabled": true},
            {"name": "id", "value": "9", "kind": "path", "enabled": true},
            {"name": "q", "value": "{{term}}", "kind": "query", "enabled": false},
        ]}),
    )
    .await;
    parity(
        "users/get.yml",
        &["--unset-query", "page"],
        json!({"params": [{"name": "id", "value": "7", "kind": "path", "enabled": true}]}),
    )
    .await;
}

#[tokio::test]
async fn headers_match_update_request() {
    parity(
        "users/get.yml",
        &[
            "--header",
            "X-A=2=3",
            "--header",
            "X-New={{v}}",
            "--unset-header",
            "X-Old",
            "--disable-header",
            "X-New",
        ],
        json!({"headers": [
            {"name": "X-A", "value": "2=3", "enabled": true},
            {"name": "X-New", "value": "{{v}}", "enabled": false},
        ]}),
    )
    .await;
}

#[tokio::test]
async fn vars_keep_their_type_and_match_update_request() {
    parity(
        "users/get.yml",
        &["--var", "n=5", "--var", "extra=e", "--unset-var", "host"],
        json!({"vars": [
            {"name": "n", "value": "5", "type": "number", "enabled": true},
            {"name": "extra", "value": "e", "enabled": true},
        ]}),
    )
    .await;
}

#[tokio::test]
async fn text_bodies_match_update_request() {
    parity(
        "users/get.yml",
        &["--body", "{\"b\":\"{{x}}\"}"],
        json!({"body": {"type": "json", "text": "{\"b\":\"{{x}}\"}"}}),
    )
    .await;
    parity(
        "users/get.yml",
        &["--body-mode", "xml", "--body", "<a/>"],
        json!({"body": {"type": "xml", "text": "<a/>"}}),
    )
    .await;
    parity(
        "users/get.yml",
        &["--body-mode", "text"],
        json!({"body": {"type": "text", "text": "{\"a\":1}"}}),
    )
    .await;
    parity(
        "users/get.yml",
        &["--body-mode", "none"],
        json!({"body": {"type": "none"}}),
    )
    .await;
}

#[tokio::test]
async fn body_file_and_stdin_match_body() {
    let a = copy();
    let b = copy();
    let file = a.root().join("../body.json");
    fs::write(&file, "{\"from\":\"file\"}\n").expect("write");
    a.item(&[
        "request",
        "users/get.yml",
        "--body-file",
        file.to_str().expect("utf-8"),
    ])
    .code(0);
    b.item(&[
        "request",
        "users/get.yml",
        "--body",
        "{\"from\":\"file\"}\n",
    ])
    .code(0);
    assert_eq!(a.tree(), b.tree());

    let c = copy();
    let mut cmd = apinata();
    for (k, v) in c.envs() {
        cmd.env(k, v);
    }
    cmd.args(["item", "--collection"])
        .arg(c.root())
        .args(["request", "users/get.yml", "--body-file", "-"])
        .write_stdin("{\"from\":\"file\"}\n")
        .timeout(Duration::from_secs(30))
        .assert()
        .code(0);
    assert_eq!(c.tree(), b.tree());
}

#[tokio::test]
async fn form_fields_match_update_request() {
    parity(
        "form.yml",
        &["--form", "a=2", "--form", "b={{x}}", "--unset-form", "old"],
        json!({"body": {"type": "form_urlencoded", "fields": [
            {"name": "a", "value": "2", "enabled": true},
            {"name": "b", "value": "{{x}}", "enabled": true},
        ]}}),
    )
    .await;
    parity(
        "users/get.yml",
        &["--body-mode", "form-urlencoded", "--form", "k=v"],
        json!({"body": {"type": "form_urlencoded", "fields": [
            {"name": "k", "value": "v", "enabled": true},
        ]}}),
    )
    .await;
}

#[tokio::test]
async fn multipart_fields_match_update_request() {
    parity(
        "multi.yml",
        &[
            "--form",
            "t=y",
            "--form-file",
            "up=/tmp/a.bin",
            "--form-file",
            "up=/tmp/b.bin",
            "--form-type",
            "up=text/plain",
            "--disable-form",
            "t",
        ],
        json!({"body": {"type": "multipart", "fields": [
            {"name": "t", "value": {"type": "text", "text": "y"}, "content_type": "", "enabled": false},
            {"name": "up", "value": {"type": "files", "paths": ["/tmp/a.bin", "/tmp/b.bin"]},
             "content_type": "text/plain", "enabled": true},
        ]}}),
    )
    .await;
}

#[tokio::test]
async fn a_file_body_matches_update_request() {
    parity(
        "users/get.yml",
        &[
            "--body-mode",
            "file",
            "--select-file",
            "/tmp/a.bin",
            "--file-type",
            "/tmp/a.bin=application/octet-stream",
        ],
        json!({"body": {"type": "file", "files": [
            {"path": "/tmp/a.bin", "content_type": "application/octet-stream", "selected": true},
        ]}}),
    )
    .await;
}

#[tokio::test]
async fn a_graphql_body_matches_update_request() {
    parity(
        "gql.yml",
        &[
            "--body",
            "query { b }",
            "--graphql-variables",
            "{\"id\": \"{{id}}\"}",
        ],
        json!({"body": {"type": "graphql", "query": "query { b }", "variables": "{\"id\": \"{{id}}\"}"}}),
    )
    .await;
}

#[tokio::test]
async fn each_auth_mode_matches_update_request() {
    let cases: [(&[&str], Value); 5] = [
        (&["--auth", "inherit"], json!({"type": "inherit"})),
        (&["--auth", "none"], json!({"type": "none"})),
        (
            &["--basic", "ann:p:w"],
            json!({"type": "basic", "username": "ann", "password": "p:w"}),
        ),
        (
            &["--api-key", "X-Key={{k}}", "--api-key-in", "query"],
            json!({"type": "apikey", "key": "X-Key", "value": "{{k}}", "placement": "query"}),
        ),
        (
            &[
                "--auth-json",
                r#"{"type": "digest", "username": "d", "password": "{{pw}}"}"#,
            ],
            json!({"type": "digest", "username": "d", "password": "{{pw}}"}),
        ),
    ];
    for (flags, auth) in cases {
        let (out, _) = parity("users/get.yml", flags, json!({"auth": auth})).await;
        assert_eq!(out, "updated users/get.yml\n", "{flags:?}");
    }
}

#[tokio::test]
async fn assertions_match_update_request() {
    parity(
        "users/get.yml",
        &[
            "--assert",
            "res.status=neq 500",
            "--assert",
            "res.body.ok=isTruthy",
            "--disable-assert",
            "res.status",
        ],
        json!({"assertions": [
            {"expression": "res.status", "operator": "neq", "value": "500", "enabled": false},
            {"expression": "res.body.ok", "operator": "isTruthy", "enabled": true},
        ]}),
    )
    .await;
    parity(
        "users/get.yml",
        &["--unset-assert", "res.status"],
        json!({"assertions": []}),
    )
    .await;
}

#[tokio::test]
async fn settings_match_update_request() {
    parity(
        "users/get.yml",
        &[
            "--timeout",
            "500",
            "--encode-url",
            "true",
            "--follow-redirects",
            "false",
            "--max-redirects",
            "3",
            "--forward-auth",
            "false",
        ],
        json!({"settings": {"encode_url": true, "timeout": {"type": "millis", "ms": 500},
               "follow_redirects": false, "max_redirects": 3,
               "forward_authorization_header": false}}),
    )
    .await;
}

#[tokio::test]
async fn docs_tags_and_seq_match_update_request() {
    parity(
        "users/get.yml",
        &[
            "--docs",
            "",
            "--tag",
            "slow",
            "--unset-tag",
            "smoke",
            "--seq",
            "4",
        ],
        json!({"docs": "", "tags": ["slow"], "seq": 4}),
    )
    .await;
}

#[tokio::test]
async fn sync_query_matches_update_request() {
    parity(
        "skew.yml",
        &["--sync-query", "url"],
        json!({"sync_query": "url"}),
    )
    .await;
    parity(
        "skew.yml",
        &["--sync-query", "params"],
        json!({"sync_query": "params"}),
    )
    .await;
}

#[tokio::test]
async fn a_missing_request_exits_3_with_the_mcp_message() {
    let a = copy();
    let b = copy();
    let message = b
        .mcp("update_request", json!({"path": "nope.yml", "docs": "x"}))
        .await
        .expect_err("refused");
    a.item(&["request", "nope.yml", "--docs", "x"])
        .code(3)
        .stdout("")
        .stderr(format!("error: {message}\n"));
    assert_eq!(a.tree(), copy().tree());
}

#[test]
fn an_unchanged_edit_writes_nothing_and_json_reports_it() {
    let a = copy();
    a.item(&[
        "request",
        "users/get.yml",
        "--header",
        "X-A=1",
        "--docs",
        "notes",
    ])
    .code(0)
    .stdout("unchanged users/get.yml\n");
    assert_eq!(a.tree(), copy().tree());
    let json_of = |args: &[&str]| -> Value {
        let mut all = vec!["--format", "json", "request", "users/get.yml"];
        all.extend_from_slice(args);
        serde_json::from_str(&stdout(&a.item(&all).code(0))).expect("json")
    };
    assert_eq!(
        json_of(&["--docs", "x"]),
        json!({"path": "users/get.yml", "changed": true})
    );
    assert_eq!(
        json_of(&["--docs", "x"]),
        json!({"path": "users/get.yml", "changed": false})
    );
}

#[test]
fn bad_flags_exit_3_and_write_nothing() {
    let a = copy();
    a.item(&["request", "users/get.yml"])
        .code(3)
        .stdout("")
        .stderr(contains("give at least one"));
    a.item(&["request", "users/get.yml", "--unset-header", "X-Gone"])
        .code(3)
        .stderr("error: no header X-Gone in users/get.yml\n");
    a.item(&["request", "users/get.yml", "--path-param", "nope=1"])
        .code(3)
        .stderr("error: no path param nope in users/get.yml\n");
    a.item(&["request", "users/get.yml", "--timeout", "0"])
        .code(3)
        .stderr(contains("unlimited"));
    a.item(&["request", "users/get.yml", "--method", "BAD METHOD"])
        .code(3)
        .stderr(contains("method"));
    a.item(&["request", "multi.yml", "--body", "x"])
        .code(3)
        .stderr(contains("--body-mode"));
    a.item(&["request", "gql.yml", "--body-mode", "json"])
        .code(3)
        .stderr(contains("graphql"));
    a.item(&["request", "users/get.yml", "--form-file", "f=/x"])
        .code(3)
        .stderr(contains("multipart"));
    a.item(&[
        "request",
        "users/get.yml",
        "--body",
        "x",
        "--body-file",
        "f",
    ])
    .code(3)
    .stderr(contains("cannot be used with"));
    assert_eq!(a.tree(), copy().tree());
}

#[test]
fn help_names_the_request_action_and_where_secrets_go() {
    apinata()
        .args(["item", "request", "--help"])
        .assert()
        .code(0)
        .stdout(contains("--body-file"))
        .stdout(contains("apinata env set --secret"));
}
