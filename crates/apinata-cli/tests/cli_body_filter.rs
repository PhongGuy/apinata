//! `--filter` on `run` and `send`: a JSONPath or XPath over each printed body.

use crate::common::{ROOT, apinata, write_tree};
use std::process::Output;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const LIST: &str = r#"[{"name":"first","n":1},{"name":"second","n":2}]"#;

async fn server() -> MockServer {
    let s = MockServer::start().await;
    let mount = |p: &'static str, body: &'static str, ct: &'static str| {
        Mock::given(path(p)).respond_with(ResponseTemplate::new(200).set_body_raw(body, ct))
    };
    mount("/list", LIST, "application/json").mount(&s).await;
    mount(
        "/xml",
        "<r><i id=\"1\">x</i><i id=\"2\">y</i></r>",
        "application/xml",
    )
    .mount(&s)
    .await;
    mount("/text", "plain words", "text/plain").mount(&s).await;
    // The secret is spelled with a JSON escape, so only the decoded match shows it.
    mount("/esc", r#"{"b":"tok\u002dQQ"}"#, "application/json")
        .mount(&s)
        .await;
    mount(
        "/secret",
        r#"{"token":"SUPERSECRET","n":1}"#,
        "application/json",
    )
    .mount(&s)
    .await;
    s
}

fn dir_for(url: &str) -> tempfile::TempDir {
    let request = format!(
        "info:\n  name: A\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n"
    );
    write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - secret: true\n    name: key\n",
        ),
        ("a.yml", &request),
    ])
}

fn run(dir: &std::path::Path, args: &[&str]) -> Output {
    apinata()
        .arg("run")
        .arg(dir)
        .args(args)
        .output()
        .expect("runs")
}

fn stdout(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).expect("utf-8")
}

fn stderr(out: &Output) -> String {
    String::from_utf8(out.stderr.clone()).expect("utf-8")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_singular_path_prints_only_that_value() {
    let s = server().await;
    let dir = dir_for(&format!("{}/list", s.uri()));
    let out = run(dir.path(), &["--body", "--filter", "$[0].name"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out), "\"first\"\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wildcard_prints_the_matches_as_an_array() {
    let s = server().await;
    let dir = dir_for(&format!("{}/list", s.uri()));
    let out = run(dir.path(), &["--body", "--filter", "$[*].n"]);
    assert_eq!(stdout(&out), "[\n  1,\n  2\n]\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_match_prints_an_empty_array_and_exits_0() {
    let s = server().await;
    let dir = dir_for(&format!("{}/list", s.uri()));
    let out = run(dir.path(), &["--body", "--filter", "$[*].nope"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stdout(&out), "[]\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn show_response_keeps_the_head_and_filters_the_body() {
    let s = server().await;
    let dir = dir_for(&format!("{}/list", s.uri()));
    let out = run(dir.path(), &["-i", "--filter", "$[1].name"]);
    let text = stdout(&out);
    assert!(text.starts_with("HTTP/1.1 200 OK\n"), "{text}");
    assert!(text.contains("content-type: application/json\n"), "{text}");
    assert!(text.ends_with("\n\n\"second\"\n"), "{text}");
    assert!(!text.contains("first"), "{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_xml_body_is_filtered_by_xpath() {
    let s = server().await;
    let dir = dir_for(&format!("{}/xml", s.uri()));
    let out = run(dir.path(), &["--body", "--filter", "//i/@id"]);
    assert_eq!(stdout(&out), "1\n2\n");
    let none = run(dir.path(), &["--body", "--filter", "//nope"]);
    assert_eq!(none.status.code(), Some(0));
    assert_eq!(stdout(&none), "");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn secrets_are_redacted_before_the_filter_runs() {
    let s = server().await;
    let dir = dir_for(&format!("{}/secret?k={{{{key}}}}", s.uri()));
    let env = ["--env", "dev", "--env-var", "key=SUPERSECRET"];
    let out = run(
        dir.path(),
        &[&env[..], &["--body", "--filter", "$.token"]].concat(),
    );
    let text = stdout(&out);
    assert!(!text.contains("SUPERSECRET"), "{text}");
    assert_eq!(text, "\"[redacted]\"\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_body_that_is_not_json_or_xml_is_an_error_naming_its_type() {
    let s = server().await;
    let dir = dir_for(&format!("{}/text", s.uri()));
    let out = run(dir.path(), &["--body", "--filter", "$.a"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("cannot apply --filter"),
        "{}",
        stderr(&out)
    );
    assert!(stderr(&out).contains("text/plain"), "{}", stderr(&out));
    assert_eq!(stdout(&out), "");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bad_expression_exits_3_naming_it_and_sends_nothing() {
    let s = server().await;
    let dir = dir_for(&format!("{}/list", s.uri()));
    let out = run(dir.path(), &["--body", "--filter", "$[?("]);
    assert_eq!(out.status.code(), Some(3));
    assert!(
        stderr(&out).contains("invalid --filter '$[?('"),
        "{}",
        stderr(&out)
    );
    assert!(s.received_requests().await.expect("recorded").is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_filter_needs_output_that_prints_a_body_and_not_the_json_report() {
    let s = server().await;
    let dir = dir_for(&format!("{}/list", s.uri()));
    for args in [
        &["--filter", "$[0]"][..],
        &["-i", "--format", "json", "--filter", "$[0]"][..],
    ] {
        let out = run(dir.path(), args);
        assert_eq!(out.status.code(), Some(3), "{args:?}: {}", stderr(&out));
        assert!(stderr(&out).contains("--filter"), "{}", stderr(&out));
    }
    assert!(s.received_requests().await.expect("recorded").is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_filters_the_printed_body() {
    let s = server().await;
    let cwd = tempfile::tempdir().unwrap();
    let url = format!("{}/list", s.uri());
    let out = apinata()
        .current_dir(cwd.path())
        .args(["send", "--no-keychain", &url, "--filter", "$[1].n"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out), "2\n");
    let json = apinata()
        .current_dir(cwd.path())
        .args([
            "send",
            "--no-keychain",
            &url,
            "--format",
            "json",
            "--filter",
            "$",
        ])
        .output()
        .unwrap();
    assert_eq!(json.status.code(), Some(3));
    let bad = apinata()
        .current_dir(cwd.path())
        .args(["send", "--no-keychain", &url, "--filter", "$[?("])
        .output()
        .unwrap();
    assert_eq!(bad.status.code(), Some(3));
    assert_eq!(s.received_requests().await.expect("recorded").len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_secret_hidden_by_a_json_escape_is_redacted_after_the_filter_too() {
    let s = server().await;
    let dir = dir_for(&format!("{}/esc?k={{{{key}}}}", s.uri()));
    let env = ["--env", "dev", "--env-var", "key=tok-QQ"];
    for mode in ["--body", "-i"] {
        let out = run(dir.path(), &[&env[..], &[mode, "--filter", "$.b"]].concat());
        let text = stdout(&out);
        assert!(!text.contains("tok-QQ"), "{mode}: {text}");
        assert!(text.contains("[redacted]"), "{mode}: {text}");
    }
    let cwd = tempfile::tempdir().unwrap();
    let url = format!("{}/esc?k={{{{KEY_QQ}}}}", s.uri());
    let out = apinata()
        .current_dir(cwd.path())
        // A value read from the process environment is secret, so send redacts it too.
        .env("KEY_QQ", "tok-QQ")
        .args([
            "send",
            "--no-keychain",
            "--env-var",
            "KEY_QQ",
            &url,
            "--filter",
            "$.b",
        ])
        .output()
        .unwrap();
    assert!(!stdout(&out).contains("tok-QQ"), "{}", stdout(&out));
    assert!(stdout(&out).contains("[redacted]"), "{}", stdout(&out));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unfilterable_body_errors_that_request_and_the_run_goes_on() {
    let s = server().await;
    let req = |name: &str, seq: u32, path: &str, extra: &str| {
        format!(
            "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"{}/{path}\"\n{extra}",
            s.uri()
        )
    };
    let fail = "runtime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"500\"\n";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("A", 1, "text", "")),
        ("b.yml", &req("B", 2, "list", fail)),
        ("c.yml", &req("C", 3, "list", "")),
    ]);
    let out = run(dir.path(), &["--body", "--filter", "$[0].name"]);
    // An errored request outranks a failed one in the report's own rule; nothing is masked as 3.
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert_eq!(stdout(&out), "\"first\"\n\"first\"\n");
    let err = stderr(&out);
    assert!(err.contains("cannot apply --filter"), "{err}");
    assert!(err.contains("1 failed") || err.contains("failed"), "{err}");
    assert!(
        err.contains("1 errored") || err.contains("errored"),
        "{err}"
    );
}
