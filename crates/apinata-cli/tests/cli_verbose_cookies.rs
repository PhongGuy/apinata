//! `run --verbose` lists each hop's Set-Cookie verdicts on stderr by name, never by value.

use crate::common;

use common::{ROOT, apinata, write_tree};
use std::process::Output;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The cookie values the chain sets; none may be printed.
pub(crate) const VALUES: [&str; 3] = ["KeptVal-Q1x", "SecureVal-Q2x", "NextVal-Q3x"];

/// Runs a collection whose one request reaches `app.test` (mapped to 127.0.0.1) at `/start`,
/// which stores one cookie, has a Secure one rejected over http and redirects to `/next`,
/// which stores another.
async fn run_chain(verbose: bool) -> (Output, u16) {
    let (_mock, dir, port) = chain_dir("").await;
    let mut cmd = apinata();
    cmd.arg("run").arg(dir.path());
    if verbose {
        cmd.arg("--verbose");
    }
    (cmd.output().expect("runs"), port)
}

/// The values the pre-request script's cookies carry; none may be printed either.
pub(crate) const SCRIPT_VALUES: [&str; 2] = ["PreVal-Q4x", "EvilVal-Q5x"];

/// A request `runtime` whose pre-request script stores one cookie and has one for another
/// domain refused.
pub(crate) fn pre_script_runtime() -> String {
    let code = format!(
        "await bru.cookies.add({{ key: \"pre\", value: \"{}\" }});\nawait bru.cookies.add({{ key: \"evil\", value: \"{}\", domain: \"evil.example\" }});",
        SCRIPT_VALUES[0], SCRIPT_VALUES[1]
    );
    let body: String = code.lines().map(|l| format!("        {l}\n")).collect();
    format!("runtime:\n  scripts:\n    - type: before-request\n      code: |-\n{body}")
}

/// The chain's server and collection, the request file ending with `runtime`, and its port.
pub(crate) async fn chain_dir(runtime: &str) -> (MockServer, tempfile::TempDir, u16) {
    let mock = MockServer::start().await;
    let port = mock.address().port();
    Mock::given(path("/start"))
        .respond_with(
            ResponseTemplate::new(302)
                .append_header("set-cookie", format!("kept={}; Path=/", VALUES[0]).as_str())
                .append_header(
                    "set-cookie",
                    format!("sec={}; Secure; Path=/", VALUES[1]).as_str(),
                )
                .insert_header("location", format!("http://app.test:{port}/next").as_str()),
        )
        .mount(&mock)
        .await;
    Mock::given(path("/next"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("set-cookie", format!("next={}; Path=/", VALUES[2]).as_str()),
        )
        .mount(&mock)
        .await;
    let root = format!(
        "{ROOT}extensions:\n  apinata:\n    hostOverrides:\n      - host: \"app.test\"\n        address: \"127.0.0.1\"\n"
    );
    let request = format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://app.test:{port}/start\"\n{runtime}"
    );
    let dir = write_tree(&[("opencollection.yml", &root), ("r.yml", &request)]);
    (mock, dir, port)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verbose_prints_one_line_per_set_cookie_with_its_verdict_and_hop() {
    let (out, port) = run_chain(true).await;
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let lines: Vec<&str> = stderr
        .lines()
        .filter(|l| l.starts_with("  set-cookie "))
        .collect();
    assert_eq!(
        lines,
        [
            format!("  set-cookie kept: Stored (app.test:{port}/start)"),
            format!("  set-cookie sec: Secure cookie over http (app.test:{port}/start)"),
            format!("  set-cookie next: Stored (app.test:{port}/next)"),
        ],
        "{stderr}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verbose_never_prints_a_cookie_value() {
    let (out, _) = run_chain(true).await;
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stderr.contains("set-cookie sec:"), "{stderr}");
    for value in VALUES {
        assert!(!stderr.contains(value), "{value} on stderr: {stderr}");
        assert!(!stdout.contains(value), "{value} on stdout: {stdout}");
    }
}

/// `run --verbose` over the chain with the pre-request script's cookies.
async fn run_scripted_chain() -> (Output, u16) {
    let (_mock, dir, port) = chain_dir(&pre_script_runtime()).await;
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .arg("--verbose")
        .output()
        .expect("runs");
    (out, port)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verbose_prints_script_cookie_lines_before_the_hop_lines() {
    let (out, port) = run_scripted_chain().await;
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let lines: Vec<&str> = stderr
        .lines()
        .filter(|l| l.starts_with("  set-cookie "))
        .collect();
    assert_eq!(
        lines,
        [
            "  set-cookie pre: Stored (pre-request script)".to_owned(),
            "  set-cookie evil: Domain does not match host (pre-request script)".to_owned(),
            format!("  set-cookie kept: Stored (app.test:{port}/start)"),
            format!("  set-cookie sec: Secure cookie over http (app.test:{port}/start)"),
            format!("  set-cookie next: Stored (app.test:{port}/next)"),
        ],
        "{stderr}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verbose_never_prints_a_script_cookie_value() {
    let (out, _) = run_scripted_chain().await;
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stderr.contains("(pre-request script)"), "{stderr}");
    for value in VALUES.iter().chain(&SCRIPT_VALUES) {
        assert!(!stderr.contains(value), "{value} on stderr: {stderr}");
        assert!(!stdout.contains(value), "{value} on stdout: {stdout}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_verbose_no_set_cookie_line_appears() {
    let (out, _) = run_chain(false).await;
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("set-cookie"), "{stderr}");
}

/// `run --verbose` over one request whose `/one` answers with the Set-Cookie value `cookie`,
/// with `key` a secret supplied as `SECRET-Name7` and written into the URL's query.
async fn run_one(cookie: &str) -> Output {
    let mock = MockServer::start().await;
    Mock::given(path("/one"))
        .respond_with(ResponseTemplate::new(200).insert_header("set-cookie", cookie))
        .mount(&mock)
        .await;
    let request = format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{}/one?k={{{{key}}}}\"\n",
        mock.uri()
    );
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - secret: true\n    name: key\n",
        ),
        ("r.yml", &request),
    ]);
    let mut cmd = apinata();
    cmd.arg("run").arg(dir.path()).args([
        "--env",
        "dev",
        "--env-var",
        "key=SECRET-Name7",
        "--verbose",
    ]);
    cmd.output().expect("runs")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unparseable_cookie_prints_as_unparsed_without_its_text() {
    let out = run_one("NamelessVal-Z9; Path=/").await;
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stderr.contains("  set-cookie (unparsed): Could not be parsed ("),
        "{stderr}"
    );
    assert!(!stderr.contains("NamelessVal-Z9"), "{stderr}");
    assert!(!stdout.contains("NamelessVal-Z9"), "{stdout}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_secret_used_as_a_cookie_name_is_redacted() {
    let out = run_one("SECRET-Name7=v; Path=/").await;
    let stderr = String::from_utf8_lossy(&out.stderr);
    let line = stderr
        .lines()
        .find(|l| l.starts_with("  set-cookie "))
        .unwrap_or_else(|| panic!("no set-cookie line: {stderr}"));
    assert!(line.contains(": Stored ("), "{stderr}");
    assert!(!stderr.contains("SECRET-Name7"), "{stderr}");
}

/// The value the failed send's pre-request script stores; it may never be printed.
pub(crate) const FAILED_VALUE: &str = "FailVal-Q9x";

/// A collection whose one request goes to a closed 127.0.0.1 port after its pre-request
/// script stored one cookie.
pub(crate) fn failed_dir() -> tempfile::TempDir {
    let code = format!("await bru.cookies.add({{ key: \"pre\", value: \"{FAILED_VALUE}\" }});");
    let request = format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://127.0.0.1:1/r\"\nruntime:\n  scripts:\n    - type: before-request\n      code: |-\n        {code}\n"
    );
    write_tree(&[("opencollection.yml", ROOT), ("r.yml", &request)])
}

#[test]
fn verbose_prints_a_failed_sends_script_cookie_line() {
    let dir = failed_dir();
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .arg("--verbose")
        .output()
        .expect("runs");
    assert_ne!(out.status.code(), Some(0), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let lines: Vec<&str> = stderr
        .lines()
        .filter(|l| l.starts_with("  set-cookie "))
        .collect();
    assert_eq!(
        lines,
        ["  set-cookie pre: Stored (pre-request script)"],
        "{stderr}"
    );
    assert!(!stderr.contains(FAILED_VALUE), "{stderr}");
    assert!(!String::from_utf8_lossy(&out.stdout).contains(FAILED_VALUE));
}
