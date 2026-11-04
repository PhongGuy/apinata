//! `apinata item folder`: each edit leaves the collection exactly as MCP `update_folder` does
//! with the equivalent full lists. One fixture is copied twice; the CLI edits one copy,
//! `apinata mcp` the other, and the trees must match byte for byte.

use crate::cli_item::{Copy, render, stdout};
use crate::common;

use std::collections::BTreeMap;
use std::fs;
use std::time::Duration;

use common::{apinata, write_tree};
use predicates::prelude::PredicateBooleanExt as _;
use predicates::str::contains;
use serde_json::{Value, json};
use tempfile::TempDir;

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: shop\nrequest:\n  headers:\n    - name: X-Root\n      value: r\n    - name: X-Old\n      value: o\n";

const USERS: &str = "info:\n  name: users\n  type: folder\n  seq: 1\nrequest:\n  headers:\n    - name: X-A\n      value: \"1\"\n  variables:\n    - name: host\n      value: localhost\n    - name: n\n      value:\n        type: number\n        data: \"1\"\n  auth:\n    type: bearer\n    token: t\ndocs: notes\n";

/// A root with headers, a folder with every section and a folder without folder.yml.
fn fixture() -> TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", USERS),
        (
            "users/get.yml",
            "info:\n  name: get\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: http://localhost/u\n",
        ),
        (
            "plain/a.yml",
            "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: http://localhost/a\n",
        ),
    ])
}

fn copy() -> Copy {
    Copy::of(&fixture())
}

/// Run `item folder <cli>` on one copy and `update_folder` with `args` on another; the trees
/// must match. Returns the CLI's stdout and the tree.
async fn parity(cli: &[&str], args: Value) -> (String, BTreeMap<String, String>) {
    let a = copy();
    let b = copy();
    let mut all = vec!["folder"];
    all.extend_from_slice(cli);
    let out = stdout(&a.item(&all).code(0));
    b.mcp("update_folder", args)
        .await
        .expect("the tool succeeds");
    let tree = a.tree();
    assert_eq!(tree, b.tree(), "cli {cli:?} and update_folder differ");
    assert_ne!(tree, copy().tree(), "the edit changed something");
    (out, tree)
}

#[tokio::test]
async fn root_headers_set_and_unset_match_update_folder() {
    let (out, tree) = parity(
        &[
            ".",
            "--header",
            "X-Root=new=1",
            "--header",
            "X-New=n",
            "--unset-header",
            "X-Old",
        ],
        json!({"path": "", "headers": [
            {"name": "X-Root", "value": "new=1", "enabled": true},
            {"name": "X-New", "value": "n", "enabled": true},
        ]}),
    )
    .await;
    assert_eq!(out, "updated the collection (opencollection.yml)\n");
    insta::assert_snapshot!("item_folder_root_headers", render(&tree));
}

#[tokio::test]
async fn folder_vars_keep_their_type_and_unset_matches_update_folder() {
    let (out, tree) = parity(
        &[
            "users",
            "--var",
            "n=5",
            "--var",
            "extra=e",
            "--unset-var",
            "host",
        ],
        json!({"path": "users", "vars": [
            {"name": "n", "value": "5", "type": "number", "enabled": true},
            {"name": "extra", "value": "e", "enabled": true},
        ]}),
    )
    .await;
    assert_eq!(out, "updated users (users/folder.yml)\n");
    insta::assert_snapshot!("item_folder_vars", render(&tree));
}

#[tokio::test]
async fn each_auth_mode_matches_update_folder() {
    let cases: [(&[&str], Value); 6] = [
        (&["--auth", "none"], json!({"type": "none"})),
        (&["--auth", "inherit"], json!({"type": "inherit"})),
        (
            &["--bearer", "{{token}}"],
            json!({"type": "bearer", "token": "{{token}}"}),
        ),
        (
            &["--basic", "ann:p:w"],
            json!({"type": "basic", "username": "ann", "password": "p:w"}),
        ),
        (
            &["--digest", "dan:pw"],
            json!({"type": "digest", "username": "dan", "password": "pw"}),
        ),
        (
            &["--api-key", "X-Key={{k}}", "--api-key-in", "query"],
            json!({"type": "apikey", "key": "X-Key", "value": "{{k}}", "placement": "query"}),
        ),
    ];
    for (flags, auth) in cases {
        let mut cli = vec!["users"];
        cli.extend_from_slice(flags);
        let (out, _) = parity(&cli, json!({"path": "users", "auth": auth})).await;
        assert_eq!(out, "updated users (users/folder.yml)\n", "{flags:?}");
    }
}

#[tokio::test]
async fn an_api_key_goes_in_a_header_by_default() {
    parity(
        &["plain", "--api-key", "X-Key=v"],
        json!({"path": "plain", "auth":
            {"type": "apikey", "key": "X-Key", "value": "v", "placement": "header"}}),
    )
    .await;
}

#[tokio::test]
async fn docs_set_and_blank_remove_match_update_folder() {
    let (_, tree) = parity(
        &["users", "--docs", "# Users\n\nAll of them."],
        json!({"path": "users", "docs": "# Users\n\nAll of them."}),
    )
    .await;
    insta::assert_snapshot!("item_folder_docs", render(&tree));
    parity(
        &["users", "--docs", ""],
        json!({"path": "users", "docs": ""}),
    )
    .await;
}

#[tokio::test]
async fn docs_file_matches_update_folder() {
    let a = copy();
    let b = copy();
    let file = a.root().join("../notes.md");
    fs::write(&file, "From a file\n").expect("write");
    a.item(&["folder", ".", "--docs-file", file.to_str().expect("utf-8")])
        .code(0);
    b.mcp(
        "update_folder",
        json!({"path": "", "docs": "From a file\n"}),
    )
    .await
    .expect("the tool succeeds");
    assert_eq!(a.tree(), b.tree());
}

#[test]
fn docs_file_dash_reads_stdin() {
    let a = copy();
    let mut cmd = apinata();
    for (k, v) in a.envs() {
        cmd.env(k, v);
    }
    cmd.args(["item", "--collection"])
        .arg(a.root())
        .args(["folder", "plain", "--docs-file", "-"])
        .write_stdin("Piped notes")
        .timeout(Duration::from_secs(30))
        .assert()
        .code(0)
        .stdout("updated plain (plain/folder.yml)\n");
    let text = fs::read_to_string(a.root().join("plain/folder.yml")).expect("read");
    assert!(text.contains("Piped notes"), "{text}");
}

#[tokio::test]
async fn a_folder_without_folder_yml_gets_one_matching_update_folder() {
    let (out, tree) = parity(
        &["plain", "--header", "X-P=1", "--var", "v=2"],
        json!({"path": "plain",
            "headers": [{"name": "X-P", "value": "1", "enabled": true}],
            "vars": [{"name": "v", "value": "2", "enabled": true}]}),
    )
    .await;
    assert_eq!(out, "updated plain (plain/folder.yml)\n");
    insta::assert_snapshot!("item_folder_created", render(&tree));
}

#[test]
fn a_disabled_header_set_again_is_enabled() {
    let a = copy();
    fs::write(
        a.root().join("users/folder.yml"),
        USERS.replace(
            "      value: \"1\"\n",
            "      value: \"1\"\n      disabled: true\n",
        ),
    )
    .expect("write");
    a.item(&["folder", "users", "--header", "X-A=1"])
        .code(0)
        .stdout("updated users (users/folder.yml)\n");
    let text = fs::read_to_string(a.root().join("users/folder.yml")).expect("read");
    assert!(
        text.contains("    - name: X-A\n      value: \"1\"\n  variables:"),
        "enabled again: {text}"
    );
}

#[test]
fn an_unchanged_edit_writes_nothing() {
    let a = copy();
    let file = a.root().join("users/folder.yml");
    let before = fs::metadata(&file)
        .expect("stat")
        .modified()
        .expect("mtime");
    a.item(&["folder", "users", "--header", "X-A=1", "--docs", "notes"])
        .code(0)
        .stdout("unchanged users (users/folder.yml)\n");
    a.item(&["folder", "plain", "--docs", ""])
        .code(0)
        .stdout("unchanged plain (plain/folder.yml)\n");
    assert_eq!(a.tree(), copy().tree());
    assert_eq!(
        fs::metadata(&file)
            .expect("stat")
            .modified()
            .expect("mtime"),
        before
    );
}

#[test]
fn json_output_names_the_path_the_file_and_whether_it_changed() {
    let a = copy();
    let json_of = |args: &[&str]| -> Value {
        let mut all = vec!["--format", "json", "folder"];
        all.extend_from_slice(args);
        serde_json::from_str(&stdout(&a.item(&all).code(0))).expect("json")
    };
    assert_eq!(
        json_of(&["users", "--docs", "x"]),
        json!({"path": "users", "file": "users/folder.yml", "changed": true})
    );
    assert_eq!(
        json_of(&["users", "--docs", "x"]),
        json!({"path": "users", "file": "users/folder.yml", "changed": false})
    );
    assert_eq!(
        json_of(&[".", "--auth", "inherit"]),
        json!({"path": "", "file": "opencollection.yml", "changed": true})
    );
}

#[tokio::test]
async fn an_unknown_folder_exits_3_with_the_mcp_message() {
    let a = copy();
    let b = copy();
    let message = b
        .mcp("update_folder", json!({"path": "nope", "docs": "x"}))
        .await
        .expect_err("refused");
    a.item(&["folder", "nope", "--docs", "x"])
        .code(3)
        .stdout("")
        .stderr(format!("error: {message}\n"));
    assert_eq!(a.tree(), copy().tree());
}

#[test]
fn unsetting_a_missing_header_or_variable_exits_3_and_writes_nothing() {
    let a = copy();
    a.item(&["folder", "users", "--docs", "x", "--unset-header", "X-Gone"])
        .code(3)
        .stdout("")
        .stderr("error: no header X-Gone in users\n");
    a.item(&["folder", ".", "--unset-var", "v"])
        .code(3)
        .stderr("error: no variable v in the collection\n");
    assert_eq!(a.tree(), copy().tree());
}

#[test]
fn a_mistyped_header_never_echoes_its_value() {
    let a = copy();
    let out = a
        .item(&[
            "folder",
            "users",
            "--header",
            "Authorization: Bearer s3cr3t",
        ])
        .code(3)
        .stdout("");
    let stderr = String::from_utf8(out.get_output().stderr.clone()).expect("utf-8");
    assert!(stderr.contains("--header takes NAME=VALUE"), "{stderr}");
    assert!(!stderr.contains("s3cr3t"), "{stderr}");
    a.item(&["folder", "users", "--var", "token: s3cr3t"])
        .code(3)
        .stderr(predicates::str::contains("s3cr3t").not());
}

#[test]
fn a_broken_folder_yml_exits_3_and_is_left_alone() {
    let a = copy();
    let file = a.root().join("users/folder.yml");
    fs::write(&file, "info: [unclosed\n").expect("write");
    a.item(&["folder", "users", "--docs", "x"])
        .code(3)
        .stdout("")
        .stderr(contains("folder.yml"));
    assert_eq!(
        fs::read_to_string(&file).expect("read"),
        "info: [unclosed\n"
    );
}

#[test]
fn bad_flags_exit_3_and_write_nothing() {
    let a = copy();
    a.item(&["folder", "users"]).code(3).stdout("").stderr(
        "error: give at least one of --header, --unset-header, --enable-header, \
         --disable-header, --var, --unset-var, --enable-var, --disable-var, an auth flag, \
         --docs or --docs-file\n",
    );
    a.item(&["folder", "users", "--header", "novalue"])
        .code(3)
        .stderr(contains("NAME=VALUE"));
    a.item(&["folder", "users", "--basic", "nocolon"])
        .code(3)
        .stderr(contains("USER:PASS"));
    a.item(&["folder", "users", "--bearer", "t", "--auth", "none"])
        .code(3)
        .stderr(contains("cannot be used with"));
    a.item(&["folder", "users", "--docs", "x", "--docs-file", "f"])
        .code(3)
        .stderr(contains("cannot be used with"));
    a.item(&["folder", "users", "--api-key-in", "query"])
        .code(3)
        .stderr(contains("--api-key"));
    assert_eq!(a.tree(), copy().tree());
}

#[test]
fn help_names_the_folder_action() {
    apinata()
        .args(["item", "folder", "--help"])
        .assert()
        .code(0)
        .stdout(contains("--unset-header"))
        .stdout(contains("apinata env set --secret"));
}

/// An authorization code oauth2 auth in the shared JSON shape, every field given.
fn oauth2_json() -> Value {
    json!({"type": "oauth2", "flow": "authorization_code",
        "authorization_url": "https://id.test/authorize", "access_token_url": "https://id.test/token",
        "refresh_token_url": "", "callback_url": "http://localhost:8765/cb", "client_id": "app",
        "client_secret": "{{client_secret}}", "client_auth": "basic_auth_header",
        "username": "", "password": "", "scope": "read write", "state": "", "pkce": true,
        "token_id": "main", "token_placement": {"type": "query", "key": "access_token"},
        "token_source": "access_token", "auto_fetch": true, "auto_refresh": true,
        "params": [{"stage": "authorizationRequest", "name": "audience", "value": "api",
            "placement": "query"}]})
}

#[tokio::test]
async fn auth_json_oauth2_matches_update_folder_and_the_core_setter() {
    let auth = oauth2_json();
    let text = auth.to_string();
    let (out, tree) = parity(
        &["users", "--auth-json", &text],
        json!({"path": "users", "auth": auth}),
    )
    .await;
    assert_eq!(out, "updated users (users/folder.yml)\n");
    insta::assert_snapshot!("item_folder_oauth2", render(&tree));
    // The app's oauth2 editor saves through the folder file's own setter; same bytes.
    let c = copy();
    let mut col = apinata_core::Collection::open(c.root()).expect("opens");
    let f = col
        .folder_file_mut(&apinata_core::ItemPath::new("users").expect("path"))
        .expect("folder file");
    f.set_auth(apinata_core::parse_auth_json(&text).expect("parses"));
    f.save().expect("saves");
    assert_eq!(c.tree(), tree);
}

#[tokio::test]
async fn auth_json_file_and_stdin_match_auth_json() {
    let text = oauth2_json().to_string();
    let a = copy();
    let b = copy();
    let file = a.root().join("../auth.json");
    fs::write(&file, &text).expect("write");
    a.item(&[
        "folder",
        "users",
        "--auth-json-file",
        file.to_str().expect("utf-8"),
    ])
    .code(0);
    b.item(&["folder", "users", "--auth-json", &text]).code(0);
    assert_eq!(a.tree(), b.tree());
    let c = copy();
    let mut cmd = apinata();
    for (k, v) in c.envs() {
        cmd.env(k, v);
    }
    cmd.args(["item", "--collection"])
        .arg(c.root())
        .args(["folder", "users", "--auth-json-file", "-"])
        .write_stdin(text)
        .timeout(Duration::from_secs(30))
        .assert()
        .code(0);
    assert_eq!(c.tree(), b.tree());
}

#[tokio::test]
async fn disabled_headers_match_update_folder() {
    parity(
        &[
            "users",
            "--header",
            "X-B=2",
            "--disable-header",
            "X-B",
            "--disable-header",
            "X-A",
        ],
        json!({"path": "users", "headers": [
            {"name": "X-A", "value": "1", "enabled": false},
            {"name": "X-B", "value": "2", "enabled": false},
        ]}),
    )
    .await;
}

#[tokio::test]
async fn a_disabled_var_matches_update_folder() {
    let (_, tree) = parity(
        &["users", "--disable-var", "n"],
        json!({"path": "users", "vars": [
            {"name": "host", "value": "localhost", "enabled": true},
            {"name": "n", "value": "1", "type": "number", "enabled": false},
        ]}),
    )
    .await;
    insta::assert_snapshot!("item_folder_disabled_var", render(&tree));
}

#[test]
fn enable_header_and_enable_var_turn_entries_back_on() {
    let a = copy();
    fs::write(
        a.root().join("users/folder.yml"),
        USERS
            .replace(
                "      value: \"1\"\n  variables:",
                "      value: \"1\"\n      disabled: true\n  variables:",
            )
            .replace(
                "      value: localhost\n",
                "      value: localhost\n      disabled: true\n",
            ),
    )
    .expect("write");
    a.item(&[
        "folder",
        "users",
        "--enable-header",
        "X-A",
        "--enable-var",
        "host",
    ])
    .code(0)
    .stdout("updated users (users/folder.yml)\n");
    let text = fs::read_to_string(a.root().join("users/folder.yml")).expect("read");
    assert!(!text.contains("disabled"), "{text}");
    assert!(
        text.contains("    - name: host\n      value: localhost\n"),
        "{text}"
    );
}

#[test]
fn toggling_an_unknown_name_exits_3_and_writes_nothing() {
    let a = copy();
    a.item(&["folder", "users", "--disable-header", "X-Gone"])
        .code(3)
        .stdout("")
        .stderr("error: no header X-Gone in users\n");
    a.item(&["folder", ".", "--enable-var", "v"])
        .code(3)
        .stderr("error: no variable v in the collection\n");
    assert_eq!(a.tree(), copy().tree());
}

#[test]
fn a_bad_auth_json_exits_3_without_echoing_a_value() {
    let a = copy();
    for bad in [
        r#"{"type":"apikey","key":"k","value":"v","placement":"s3cr3t-Value-9"}"#,
        r#"{"type":"bearer","token":"s3cr3t-Value-9""#,
        r#"{"type":"bearer","token":"s3cr3t-Value-9","pkce":"x"}"#,
    ] {
        let out = a
            .item(&["folder", "users", "--auth-json", bad])
            .code(3)
            .stdout("");
        let stderr = String::from_utf8(out.get_output().stderr.clone()).expect("utf-8");
        assert!(stderr.starts_with("error: "), "{stderr}");
        assert!(!stderr.contains("s3cr3t"), "{stderr}");
    }
    a.item(&["folder", "users", "--auth-json", "{}", "--bearer", "t"])
        .code(3)
        .stderr(contains("cannot be used with"));
    assert_eq!(a.tree(), copy().tree());
}

#[test]
fn short_help_says_values_land_in_the_tracked_file_and_where_secrets_go() {
    apinata()
        .args(["item", "folder", "-h"])
        .assert()
        .code(0)
        .stdout(contains("--auth-json"))
        .stdout(contains("--disable-header"))
        .stdout(contains("git-tracked"))
        .stdout(contains("env set --secret"))
        .stdout(contains("Bruno").not());
}
