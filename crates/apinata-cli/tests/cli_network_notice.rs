//! A collection whose network settings changed since the last run says so on stderr, once.
//! The line never blocks the run and `--quiet` silences it.

use crate::common;

use common::{ROOT, apinata, write_tree};
use std::fs;
use std::path::Path;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const CHANGED: &str = "collection network settings changed since the last run";
const OVERRIDE: &str = "Host override added: api.example.com -> 203.0.113.5";

const WITH_OVERRIDE: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nextensions:\n  apinata:\n    hostOverrides:\n      - host: api.example.com\n        address: 203.0.113.5\n";

async fn server() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    s
}

fn collection(s: &MockServer, root: &str) -> tempfile::TempDir {
    let request = format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{}/ok\"\n",
        s.uri()
    );
    write_tree(&[("opencollection.yml", root), ("r.yml", &request)])
}

/// One run with its own acknowledgement file; returns the exit code and stderr.
fn run(dir: &Path, ack: &Path, extra: &[&str]) -> (i32, String) {
    let out = apinata()
        .env("APINATA_TEST_NETWORK_ACK", ack)
        .arg("run")
        .arg(dir)
        .args(extra)
        .output()
        .expect("runs");
    (
        out.status.code().expect("exit code"),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn edit_root(dir: &Path, root: &str) {
    fs::write(dir.join("opencollection.yml"), root).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_changed_setting_warns_once_and_does_not_block() {
    let s = server().await;
    let dir = collection(&s, ROOT);
    let state = tempfile::tempdir().unwrap();
    let ack = state.path().join("network-ack.json");
    let (code, err) = run(dir.path(), &ack, &[]);
    assert_eq!((code, err.contains(CHANGED)), (0, false), "{err}");
    edit_root(dir.path(), WITH_OVERRIDE);
    let (code, err) = run(dir.path(), &ack, &[]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(err.matches(CHANGED).count(), 1, "{err}");
    assert!(err.contains(OVERRIDE), "{err}");
    assert!(
        err.lines()
            .any(|l| l.starts_with("warning: ") && l.contains(CHANGED)),
        "{err}"
    );
    // Seen once, so the next run is quiet.
    let (_, err) = run(dir.path(), &ack, &[]);
    assert!(!err.contains(CHANGED), "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn quiet_hides_the_line_but_still_records_the_change() {
    let s = server().await;
    let dir = collection(&s, ROOT);
    let state = tempfile::tempdir().unwrap();
    let ack = state.path().join("network-ack.json");
    run(dir.path(), &ack, &[]);
    edit_root(dir.path(), WITH_OVERRIDE);
    let (code, err) = run(dir.path(), &ack, &["--quiet"]);
    assert_eq!(code, 0, "{err}");
    assert!(!err.contains(CHANGED) && !err.contains(OVERRIDE), "{err}");
    let (_, err) = run(dir.path(), &ack, &[]);
    assert!(!err.contains(CHANGED), "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn quiet_is_accepted_before_the_subcommand() {
    let s = server().await;
    let dir = collection(&s, WITH_OVERRIDE);
    let state = tempfile::tempdir().unwrap();
    let ack = state.path().join("network-ack.json");
    let out = apinata()
        .env("APINATA_TEST_NETWORK_ACK", &ack)
        .arg("--quiet")
        .arg("run")
        .arg(dir.path())
        .output()
        .expect("runs");
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{err}");
    assert!(!err.contains(OVERRIDE), "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_first_run_of_a_collection_that_sets_something_says_so_once() {
    let s = server().await;
    let dir = collection(&s, WITH_OVERRIDE);
    let state = tempfile::tempdir().unwrap();
    let ack = state.path().join("network-ack.json");
    let (code, err) = run(dir.path(), &ack, &[]);
    assert_eq!(code, 0, "{err}");
    assert!(!err.contains(CHANGED), "{err}");
    assert_eq!(err.matches("this collection sets").count(), 1, "{err}");
    assert!(err.contains(OVERRIDE), "{err}");
    let (_, err) = run(dir.path(), &ack, &[]);
    assert!(!err.contains("this collection sets"), "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_first_run_with_nothing_set_is_silent() {
    let s = server().await;
    let dir = collection(&s, ROOT);
    let state = tempfile::tempdir().unwrap();
    let ack = state.path().join("network-ack.json");
    let (code, err) = run(dir.path(), &ack, &[]);
    assert_eq!(code, 0, "{err}");
    assert!(
        !err.contains("this collection sets") && !err.contains(CHANGED),
        "{err}"
    );
}

const UNREADABLE: &str = "cannot verify network settings";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreadable_record_warns_instead_of_staying_silent() {
    let s = server().await;
    let dir = collection(&s, WITH_OVERRIDE);
    let state = tempfile::tempdir().unwrap();
    let ack = state.path().join("network-ack.json");
    fs::write(&ack, "not json").unwrap();
    for _ in 0..2 {
        let (code, err) = run(dir.path(), &ack, &[]);
        assert_eq!(code, 0, "{err}");
        assert_eq!(err.to_lowercase().matches(UNREADABLE).count(), 1, "{err}");
    }
    let (_, err) = run(dir.path(), &ack, &["--quiet"]);
    assert!(!err.to_lowercase().contains(UNREADABLE), "{err}");
    assert_eq!(fs::read_to_string(&ack).unwrap(), "not json");
}

fn proxied(user: &str) -> String {
    format!(
        "opencollection: \"1.0.0\"\ninfo:\n  name: t\nconfig:\n  proxy:\n    inherit: false\n    config:\n      protocol: http\n      hostname: 127.0.0.1\n      port: 9\n      auth:\n        username: {user}\n        password: PASSWORD-SENTINEL\n"
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_notice_never_prints_a_credential() {
    let s = server().await;
    let dir = collection(&s, &proxied("USER-ONE-SENTINEL"));
    let state = tempfile::tempdir().unwrap();
    let ack = state.path().join("network-ack.json");
    let (_, first) = run(dir.path(), &ack, &[]);
    edit_root(dir.path(), &proxied("USER-TWO-SENTINEL"));
    let (_, second) = run(dir.path(), &ack, &[]);
    assert!(second.contains("Proxy credentials changed"), "{second}");
    for err in [&first, &second] {
        for secret in [
            "USER-ONE-SENTINEL",
            "USER-TWO-SENTINEL",
            "PASSWORD-SENTINEL",
        ] {
            assert!(!err.contains(secret), "{secret} in {err}");
        }
    }
    assert!(!fs::read_to_string(&ack).unwrap().contains("SENTINEL"));
}

fn proxy_root(host: &str) -> String {
    format!(
        "opencollection: \"1.0.0\"\ninfo:\n  name: t\nconfig:\n  proxy:\n    inherit: false\n    config:\n      protocol: http\n      hostname: \"{host}\"\n      port: 9\n"
    )
}

/// [`run`], returning stdout and stderr together.
fn run_all(dir: &Path, ack: &Path) -> String {
    let out = apinata()
        .env("APINATA_TEST_NETWORK_ACK", ack)
        .arg("run")
        .arg(dir)
        .output()
        .expect("runs");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

// A proxy host with userinfo is refused at resolve, so nothing is sent and nothing asked; the
// credential reaches neither the report, stderr nor any record.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn userinfo_in_a_proxy_host_never_reaches_stderr_or_the_record() {
    let s = server().await;
    let dir = collection(&s, &proxy_root("user:USERINFO-SENTINEL@127.0.0.1"));
    let state = tempfile::tempdir().unwrap();
    let ack = state.path().join("network-ack.json");
    let first = run_all(dir.path(), &ack);
    edit_root(dir.path(), &proxy_root("user:OTHER-SENTINEL@127.0.0.1"));
    let second = run_all(dir.path(), &ack);
    assert!(second.contains("not a valid host"), "{second}");
    for out in [&first, &second] {
        assert!(!out.contains("SENTINEL"), "{out}");
    }
    assert!(
        !fs::read_to_string(&ack)
            .unwrap_or_default()
            .contains("SENTINEL")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_variable_proxy_follows_the_selected_environment() {
    let s = server().await;
    let dir = collection(&s, &proxy_root("{{p}}"));
    let env = |value: &str| format!("name: dev\nvariables:\n  - name: p\n    value: {value}\n");
    fs::create_dir_all(dir.path().join("environments")).unwrap();
    let write_env =
        |value: &str| fs::write(dir.path().join("environments/dev.yml"), env(value)).unwrap();
    let state = tempfile::tempdir().unwrap();
    let ack = state.path().join("network-ack.json");
    write_env("127.0.0.1");
    run(dir.path(), &ack, &["--env", "dev"]);
    write_env("localhost");
    let (_, err) = run(dir.path(), &ack, &["--env", "dev"]);
    assert_eq!(err.matches(CHANGED).count(), 1, "{err}");
    assert!(err.contains("Proxy changed to http://localhost:9"), "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_password_with_delimiters_never_reaches_stderr_or_the_record() {
    let s = server().await;
    let dir = collection(&s, &proxy_root("u:pa/ss?w#x@127.0.0.1"));
    let state = tempfile::tempdir().unwrap();
    let ack = state.path().join("network-ack.json");
    let first = run_all(dir.path(), &ack);
    edit_root(dir.path(), &proxy_root("u:ot/her?w#y@127.0.0.1"));
    let second = run_all(dir.path(), &ack);
    assert!(second.contains("not a valid host"), "{second}");
    let record = fs::read_to_string(&ack).unwrap_or_default();
    for text in [&first, &second, &record] {
        for secret in ["pa/ss", "w#x", "ot/her", "w#y", "?w"] {
            assert!(!text.contains(secret), "{secret} in {text}");
        }
    }
}

const SHADOWED: &str = "set by a folder, request or run variable";

/// A collection whose proxy host is `{{ph}}`: 127.0.0.1 at the collection, `{{folder}}` in
/// folder `f`, which holds the only request.
fn shadowed(s: &MockServer, folder: &str, env: &str) -> tempfile::TempDir {
    let root = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nconfig:\n  proxy:\n    inherit: false\n    config:\n      protocol: http\n      hostname: \"{{ph}}\"\n      port: 9\nrequest:\n  variables:\n    - name: ph\n      value: 127.0.0.1\n";
    let folder = format!(
        "info:\n  name: f\n  type: folder\nrequest:\n  variables:\n    - name: ph\n      value: \"{folder}\"\n"
    );
    let request = format!(
        "info:\n  name: b\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{}/ok\"\n",
        s.uri()
    );
    write_tree(&[
        ("opencollection.yml", root),
        ("environments/dev.yml", env),
        ("f/folder.yml", &folder),
        ("f/b.yml", &request),
    ])
}

/// [`run`] with a scratch runtime directory and the `dev` environment.
fn run_dev(dir: &Path, ack: &Path, extra: &[&str]) -> (i32, String) {
    let runtime = tempfile::tempdir().unwrap();
    let out = apinata()
        .env("APINATA_TEST_NETWORK_ACK", ack)
        .env("XDG_RUNTIME_DIR", runtime.path())
        .arg("run")
        .arg(dir)
        .args(["--env", "dev"])
        .args(extra)
        .output()
        .expect("runs");
    (
        out.status.code().expect("exit code"),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

const DEV: &str = "name: dev\nvariables:\n  - name: unused\n    value: x\n";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_folder_variable_shadowing_the_proxy_prints_one_line_then_none() {
    let s = server().await;
    let dir = shadowed(&s, "localhost", DEV);
    let state = tempfile::tempdir().unwrap();
    let ack = state.path().join("network-ack.json");
    let (_, err) = run_dev(dir.path(), &ack, &[]);
    assert_eq!(err.matches(SHADOWED).count(), 1, "{err}");
    assert_eq!(
        err.lines().filter(|l| l.starts_with("warning: ")).count(),
        1,
        "{err}"
    );
    assert!(err.contains("localhost"), "{err}");
    let (_, err) = run_dev(dir.path(), &ack, &[]);
    assert!(!err.contains(SHADOWED) && !err.contains(CHANGED), "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn quiet_prints_no_shadow_line() {
    let s = server().await;
    let dir = shadowed(&s, "localhost", DEV);
    let state = tempfile::tempdir().unwrap();
    let ack = state.path().join("network-ack.json");
    let (_, err) = run_dev(dir.path(), &ack, &["--quiet"]);
    assert!(
        !err.contains(SHADOWED) && !err.contains("localhost"),
        "{err}"
    );
    let (_, err) = run_dev(dir.path(), &ack, &[]);
    assert!(!err.contains(SHADOWED), "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_secret_through_a_folder_variable_never_reaches_stderr_or_the_record() {
    const SECRET: &str = "s3cr3t-proxy-host";
    let s = server().await;
    let env = "name: dev\nvariables:\n  - secret: true\n    name: sec\n";
    let dir = shadowed(&s, "{{sec}}", env);
    let state = tempfile::tempdir().unwrap();
    let ack = state.path().join("network-ack.json");
    let var = format!("sec={SECRET}");
    let (_, err) = run_dev(dir.path(), &ack, &["--env-var", &var]);
    assert_eq!(err.matches(SHADOWED).count(), 1, "{err}");
    assert!(!err.contains(SECRET), "{err}");
    assert!(!fs::read_to_string(&ack).unwrap().contains(SECRET));
}
