//! The exit-code convention, one test per command and code:
//! 0 success; 1 a result the user asked to judge failed; 2 a request or transport failure;
//! 3 a usage, input or setup error. `self-update --check` keeps 2 for "update available"
//! (pinned in cli_self_update) and `mock` Ctrl+C exits 0 (pinned in cli_mock).

use std::path::Path;
use std::time::Duration;

use crate::common::{self, ROOT, check, dead_port, get, write_tree};
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The binary with scratch HOME and XDG dirs and no session bus, so no command can reach the
/// real keyring or config.
fn cli() -> (assert_cmd::Command, tempfile::TempDir) {
    let home = tempfile::TempDir::new().expect("home");
    let mut cmd = common::apinata();
    cmd.env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("XDG_CACHE_HOME", home.path().join("cache"))
        .env_remove("XDG_RUNTIME_DIR")
        .timeout(Duration::from_secs(30));
    (cmd, home)
}

fn req(name: &str, url: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n"
    )
}

/// A collection with one request to `url`.
fn collection(url: &str) -> tempfile::TempDir {
    write_tree(&[("opencollection.yml", ROOT), ("a.yml", &req("a", url))])
}

/// An empty directory: not a collection.
fn empty() -> tempfile::TempDir {
    tempfile::TempDir::new().expect("dir")
}

fn code_of(args: &[&str], dir: &Path) -> i32 {
    let (mut c, _h) = cli();
    c.current_dir(dir).args(args);
    let out = c.output().expect("runs");
    out.status.code().expect("code")
}

// run

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_exits_0_1_and_2_for_pass_failed_assertion_and_request_error() {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let url = format!("{}/x", s.uri());
    let pass = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &get("a", 1, &url, &check("res.status", "eq", "200")),
        ),
    ]);
    let fail = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &get("a", 1, &url, &check("res.status", "eq", "201")),
        ),
    ]);
    let down = collection(&format!("http://127.0.0.1:{}/", dead_port()));
    let (p, f, d) = (
        pass.path().to_owned(),
        fail.path().to_owned(),
        down.path().to_owned(),
    );
    let codes = tokio::task::spawn_blocking(move || {
        (
            code_of(&["run", "."], &p),
            code_of(&["run", "."], &f),
            code_of(&["run", "."], &d),
        )
    })
    .await
    .unwrap();
    assert_eq!(codes, (0, 1, 2));
}

#[test]
fn run_exits_3_without_a_collection_and_for_a_request_file_that_fails_to_load() {
    assert_eq!(code_of(&["run", "."], empty().path()), 3);
    let broken = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", "info: [this is: not valid\n"),
    ]);
    assert_eq!(code_of(&["run", "."], broken.path()), 3);
}

/// One request tagged `smoke` and `fast`.
fn tagged_collection() -> tempfile::TempDir {
    let body = req("a", "http://127.0.0.1:9/").replace(
        "  seq: 1
",
        "  seq: 1
  tags:
    - smoke
    - fast
",
    );
    write_tree(&[("opencollection.yml", ROOT), ("a.yml", &body)])
}

fn stderr_of(args: &[&str], dir: &Path) -> (i32, String) {
    let (mut c, _h) = cli();
    c.current_dir(dir).args(args);
    let out = c.output().expect("runs");
    (
        out.status.code().expect("code"),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn run_exits_3_when_no_request_matches_the_tag_filters() {
    let dir = tagged_collection();
    let (code, err) = stderr_of(&["run", ".", "--tags", "nope"], dir.path());
    assert_eq!(code, 3, "{err}");
    assert!(err.contains("no requests matched: tags nope"), "{err}");
    let (code, err) = stderr_of(&["run", ".", "--exclude-tags", "smoke,fast"], dir.path());
    assert_eq!(code, 3, "{err}");
    assert!(
        err.contains("no requests matched: excluding tags smoke, fast"),
        "{err}"
    );
}

#[test]
fn run_exits_3_for_a_folder_with_no_requests() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "sub/folder.yml",
            "info:
  name: sub
",
        ),
    ]);
    let (code, err) = stderr_of(&["run", "sub"], dir.path());
    assert_eq!(code, 3, "{err}");
    assert!(
        err.contains("no requests to run in the selected path"),
        "{err}"
    );
}

#[test]
fn a_plan_with_only_a_load_error_still_exits_3_with_the_load_error() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            "info: [this is: not valid
",
        ),
    ]);
    let (code, err) = stderr_of(&["run", ".", "--tags", "nope"], dir.path());
    assert_eq!(code, 3, "{err}");
    assert!(!err.contains("no requests matched"), "{err}");
}

#[test]
fn run_exits_3_when_a_report_file_cannot_be_written() {
    let dir = collection("http://{{nope}}/");
    let out = dir.path().join("missing-dir/report.json");
    let code = code_of(
        &["run", ".", "--reporter-json", out.to_str().unwrap()],
        dir.path(),
    );
    assert_eq!(code, 3);
}

// check

#[test]
fn check_exits_0_1_and_3() {
    assert_eq!(code_of(&["check"], collection("http://x/").path()), 0);
    assert_eq!(
        code_of(&["check"], collection("http://{{nope}}/").path()),
        1
    );
    assert_eq!(code_of(&["check"], empty().path()), 3);
}

// ls, search, replace, env, trust, gen

#[test]
fn ls_exits_0_and_3() {
    assert_eq!(code_of(&["ls"], collection("http://x/").path()), 0);
    assert_eq!(
        code_of(&["ls", "--json"], collection("http://x/").path()),
        0
    );
    assert_eq!(code_of(&["ls"], empty().path()), 3);
}

#[test]
fn search_exits_0_and_3() {
    assert_eq!(code_of(&["search", "x"], collection("http://x/").path()), 0);
    assert_eq!(
        code_of(&["search", "--json", "zzz"], collection("http://x/").path()),
        0
    );
    assert_eq!(code_of(&["search", "x"], empty().path()), 3);
}

#[test]
fn replace_exits_0_and_3() {
    let dir = collection("http://x/");
    assert_eq!(code_of(&["replace", "--dry-run", "x", "y"], dir.path()), 0);
    assert_eq!(
        code_of(&["replace", "--dry-run", "x", "y"], empty().path()),
        3
    );
}

#[test]
fn env_exits_0_and_3() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", "name: dev\nvariables: []\n"),
    ]);
    assert_eq!(code_of(&["env", "ls"], dir.path()), 0);
    assert_eq!(code_of(&["env", "show", "nope"], dir.path()), 3);
}

#[test]
fn trust_exits_0_and_3() {
    let dir = collection("http://x/");
    assert_eq!(code_of(&["trust", "list"], dir.path()), 0);
    assert_eq!(code_of(&["trust", "list"], empty().path()), 3);
}

#[test]
fn gen_exits_0_and_3() {
    let dir = collection("http://x/");
    let file = dir.path().join("a.yml");
    let f = file.to_str().unwrap();
    assert_eq!(code_of(&["gen", "--target", "curl", f], dir.path()), 0);
    assert_eq!(
        code_of(&["gen", "--target", "curl", "missing.yml"], dir.path()),
        3
    );
}

// import, export, convert

#[test]
fn import_exits_0_and_3() {
    let scratch = empty();
    let input = scratch.path().join("c.json");
    std::fs::write(
        &input,
        r#"{"info":{"name":"S","schema":"https://schema.getpostman.com/json/collection/v2.1.0/collection.json"},"item":[]}"#,
    )
    .unwrap();
    let into = scratch.path().join("into");
    std::fs::create_dir(&into).unwrap();
    let args = |p: &Path| {
        vec![
            "import".to_owned(),
            "collection".to_owned(),
            p.display().to_string(),
            "--into".to_owned(),
            into.display().to_string(),
        ]
    };
    let a = args(&input);
    let a: Vec<&str> = a.iter().map(String::as_str).collect();
    assert_eq!(code_of(&a, scratch.path()), 0);
    let missing = args(&scratch.path().join("absent.json"));
    let missing: Vec<&str> = missing.iter().map(String::as_str).collect();
    assert_eq!(code_of(&missing, scratch.path()), 3);
}

#[test]
fn export_exits_0_and_3() {
    let dir = collection("http://x/");
    let out = empty();
    let target = out.path().join("c.json");
    let t = target.to_str().unwrap();
    let d = dir.path().to_str().unwrap();
    assert_eq!(
        code_of(
            &["export", d, "--format", "postman", "--out", t],
            out.path()
        ),
        0
    );
    let e = empty();
    assert_eq!(
        code_of(
            &[
                "export",
                e.path().to_str().unwrap(),
                "--format",
                "postman",
                "--out",
                t
            ],
            out.path()
        ),
        3
    );
}

#[test]
fn convert_exits_0_and_3() {
    let src = write_tree(&[
        (
            "bruno.json",
            "{\"version\":\"1\",\"name\":\"L\",\"type\":\"collection\"}",
        ),
        (
            "a.bru",
            "meta {\n  name: a\n  type: http\n  seq: 1\n}\n\nget {\n  url: http://x/\n}\n",
        ),
    ]);
    let s = src.path().to_str().unwrap();
    assert_eq!(
        code_of(&["convert", s, "--in-place", "--dry-run"], src.path()),
        0
    );
    assert_eq!(code_of(&["convert", s, "--in-place"], src.path()), 3);
}

// git

#[test]
fn git_exits_0_and_3_including_for_a_file_outside_the_collection() {
    let dir = collection("http://x/");
    assert_eq!(code_of(&["git", "status"], dir.path()), 0);
    assert_eq!(code_of(&["git", "diff", "../outside.yml"], dir.path()), 3);
    assert_eq!(code_of(&["git", "status"], empty().path()), 3);
}

// mock

#[test]
fn mock_exits_3_for_every_setup_failure() {
    let dir = collection("http://x/");
    let d = dir.path().to_str().unwrap();
    // A non-loopback bind, a directory that is no collection, and a port already in use.
    assert_eq!(
        code_of(&["mock", d, "--bind", "0.0.0.0", "--port", "0"], dir.path()),
        3
    );
    assert_eq!(
        code_of(
            &["mock", empty().path().to_str().unwrap(), "--port", "0"],
            dir.path()
        ),
        3
    );
    let busy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = busy.local_addr().unwrap().port().to_string();
    assert_eq!(
        code_of(
            &["mock", d, "--bind", "127.0.0.1", "--port", &port],
            dir.path()
        ),
        3
    );
}

// mcp

#[test]
fn mcp_exits_3_when_it_cannot_start() {
    assert_eq!(code_of(&["mcp"], empty().path()), 3);
    let dir = collection("http://x/");
    assert_eq!(code_of(&["mcp", "--global-env", "nope"], dir.path()), 3);
}

/// The two messages a client sends to start a session, then end of input.
const HANDSHAKE: &str = concat!(
    r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#,
    "\n",
    r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    "\n"
);

#[test]
fn mcp_exits_0_when_the_client_disconnects_after_the_handshake() {
    let dir = collection("http://x/");
    let (mut c, _h) = cli();
    c.current_dir(dir.path()).arg("mcp").write_stdin(HANDSHAKE);
    assert_eq!(c.output().unwrap().status.code(), Some(0));
}

#[test]
fn mcp_exits_2_when_the_transport_ends_before_the_handshake() {
    let dir = collection("http://x/");
    let (mut c, _h) = cli();
    c.current_dir(dir.path()).arg("mcp").write_stdin("");
    assert_eq!(c.output().unwrap().status.code(), Some(2));
}

// completions

#[test]
fn completions_exit_0_and_3() {
    assert_eq!(code_of(&["completions", "bash"], empty().path()), 0);
    assert_eq!(code_of(&["completions", "nonsense"], empty().path()), 3);
}

// An argument-parser error is 3 for every command, never clap's 2.
#[test]
fn an_unknown_flag_exits_3() {
    assert_eq!(code_of(&["check", "--nonsense"], empty().path()), 3);
}
