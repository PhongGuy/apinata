//! `run --disable-cookies`: no cookie is stored or sent, across redirects and requests.

use crate::common;

use common::{ROOT, apinata, run_json, write_tree};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn request(name: &str, seq: u32, url: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"{url}\"\n"
    )
}

/// `/login` answers 302 to `/home` setting `sid=abc`; `/home` and `/echo` answer 200.
async fn login_server() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(path("/login"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", "/home")
                .insert_header("set-cookie", "sid=abc; Path=/"),
        )
        .mount(&s)
        .await;
    for at in ["/home", "/echo"] {
        Mock::given(path(at))
            .respond_with(ResponseTemplate::new(200))
            .mount(&s)
            .await;
    }
    s
}

/// Runs login then echo with `extra` flags; returns the Cookie header `/home` and `/echo` saw.
async fn cookies_seen(extra: &[&str]) -> Vec<Option<String>> {
    let s = login_server().await;
    let u = s.uri();
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("login", 1, &format!("{u}/login"))),
        ("b.yml", &request("echo", 2, &format!("{u}/echo"))),
    ]);
    let (code, report) = run_json(dir.path(), extra);
    assert_eq!(code, 0, "{report}");
    let got = s.received_requests().await.expect("recorded");
    let paths: Vec<&str> = got.iter().map(|r| r.url.path()).collect();
    assert_eq!(paths, ["/login", "/home", "/echo"]);
    got[1..]
        .iter()
        .map(|r| {
            r.headers
                .get("cookie")
                .map(|v| v.to_str().expect("ascii").to_owned())
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_the_flag_the_cookie_rides_the_redirect_and_the_next_request() {
    let sid = Some("sid=abc".to_owned());
    assert_eq!(cookies_seen(&[]).await, [sid.clone(), sid]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disable_cookies_stores_and_sends_none() {
    assert_eq!(cookies_seen(&["--disable-cookies"]).await, [None, None]);
}

#[test]
fn run_help_describes_the_flag() {
    let out = apinata().args(["run", "--help"]).output().expect("runs");
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(help.contains("--disable-cookies"), "{help}");
    assert!(
        help.contains("Do not store or send cookies between requests in this run"),
        "{help}"
    );
}

/// A collection of `/login` then `/echo` against `s`, and a fresh jar path beside it.
fn login_echo(s: &MockServer, echo_first: bool) -> tempfile::TempDir {
    let u = s.uri();
    let (a, b) = if echo_first {
        ("/echo", "/login")
    } else {
        ("/login", "/echo")
    };
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("one", 1, &format!("{u}{a}"))),
        ("b.yml", &request("two", 2, &format!("{u}{b}"))),
    ])
}

fn run_jar(dir: &std::path::Path, jar: &std::path::Path, extra: &[&str]) -> std::process::Output {
    let home = tempfile::tempdir().expect("home");
    std::fs::create_dir(home.path().join("r")).expect("runtime dir");
    apinata()
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("c"))
        .env("XDG_DATA_HOME", home.path().join("d"))
        .env("XDG_RUNTIME_DIR", home.path().join("r"))
        .arg("run")
        .arg(dir)
        .arg("--cookie-jar")
        .arg(jar)
        .args(extra)
        .output()
        .expect("runs")
}

async fn echo_cookie(s: &MockServer) -> Vec<Option<String>> {
    s.received_requests()
        .await
        .expect("recorded")
        .iter()
        .filter(|r| r.url.path() == "/echo")
        .map(|r| {
            r.headers
                .get("cookie")
                .map(|v| v.to_str().unwrap().to_owned())
        })
        .collect()
}

fn all_output(out: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_jar_file_carries_a_cookie_to_the_next_run_only_with_the_flag() {
    let s = login_server().await;
    let work = tempfile::tempdir().unwrap();
    let jar = work.path().join("jar.json");
    let login = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("login", 1, &format!("{}/login", s.uri()))),
    ]);
    let echo = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("echo", 1, &format!("{}/echo", s.uri()))),
    ]);
    let first = run_jar(login.path(), &jar, &[]);
    assert_eq!(first.status.code(), Some(0), "{}", all_output(&first));
    assert!(jar.is_file());
    let second = run_jar(echo.path(), &jar, &[]);
    assert_eq!(second.status.code(), Some(0), "{}", all_output(&second));
    let (code, _) = run_json(echo.path(), &[]);
    assert_eq!(code, 0);
    assert_eq!(echo_cookie(&s).await, [Some("sid=abc".to_owned()), None]);
    let note = String::from_utf8_lossy(&first.stderr).to_lowercase();
    assert!(note.contains("git"), "{note}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_expired_cookie_in_the_file_is_not_sent_and_not_saved() {
    let s = login_server().await;
    let work = tempfile::tempdir().unwrap();
    let jar = work.path().join("jar.json");
    let host = s.address().ip().to_string();
    let cookie = |n: &str, e: &str| {
        format!(
            r#"{{"domain":"{host}","path":"/","name":"{n}","value":"v{n}","expires":{e},"secure":false,"httpOnly":false,"hostOnly":true}}"#
        )
    };
    std::fs::write(
        &jar,
        format!("[{},{}]", cookie("old", "1"), cookie("live", "4102444800")),
    )
    .unwrap();
    let dir = login_echo(&s, true);
    let out = run_jar(dir.path(), &jar, &["--format", "json"]);
    assert_eq!(out.status.code(), Some(0), "{}", all_output(&out));
    assert_eq!(echo_cookie(&s).await, [Some("live=vlive".to_owned())]);
    let saved = std::fs::read_to_string(&jar).unwrap();
    assert!(!saved.contains("\"old\""), "{saved}");
    assert!(saved.contains("\"live\""), "{saved}");
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_jar_file_is_mode_0600_when_created_and_when_it_already_existed() {
    use std::os::unix::fs::PermissionsExt;
    let s = login_server().await;
    let work = tempfile::tempdir().unwrap();
    let dir = login_echo(&s, false);
    let fresh = work.path().join("fresh.json");
    run_jar(dir.path(), &fresh, &[]);
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&fresh), 0o600);
    let old = work.path().join("old.json");
    std::fs::write(&old, "[]").unwrap();
    std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o644)).unwrap();
    run_jar(dir.path(), &old, &[]);
    assert_eq!(mode(&old), 0o600);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failing_run_still_saves_the_jar() {
    let s = login_server().await;
    Mock::given(path("/boom"))
        .respond_with(ResponseTemplate::new(500).insert_header("set-cookie", "sid=abc; Path=/"))
        .mount(&s)
        .await;
    let u = s.uri();
    let check_500 = "runtime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &format!("{}{check_500}", request("boom", 1, &format!("{u}/boom"))),
        ),
    ]);
    let work = tempfile::tempdir().unwrap();
    let jar = work.path().join("jar.json");
    let out = run_jar(dir.path(), &jar, &[]);
    assert_ne!(out.status.code(), Some(0));
    assert!(std::fs::read_to_string(&jar).unwrap().contains("\"sid\""));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cookie_jar_conflicts_with_disable_cookies_and_sends_nothing() {
    let s = login_server().await;
    let dir = login_echo(&s, false);
    let work = tempfile::tempdir().unwrap();
    let jar = work.path().join("jar.json");
    let out = run_jar(dir.path(), &jar, &["--disable-cookies"]);
    // The CLI maps every usage error, clap's included, to exit 3.
    assert_eq!(out.status.code(), Some(3), "{}", all_output(&out));
    assert!(s.received_requests().await.unwrap().is_empty());
    assert!(!jar.exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_malformed_jar_file_is_an_error_and_is_left_untouched() {
    let s = login_server().await;
    let dir = login_echo(&s, false);
    let work = tempfile::tempdir().unwrap();
    let jar = work.path().join("jar.json");
    std::fs::write(&jar, "not json hunter2").unwrap();
    let out = run_jar(dir.path(), &jar, &[]);
    // A file that fails to load is an input error, exit 3.
    assert_eq!(out.status.code(), Some(3), "{}", all_output(&out));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("jar"),
        "{}",
        all_output(&out)
    );
    assert!(s.received_requests().await.unwrap().is_empty());
    assert_eq!(std::fs::read_to_string(&jar).unwrap(), "not json hunter2");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreadable_jar_path_exits_3_and_sends_nothing() {
    let s = login_server().await;
    let dir = login_echo(&s, false);
    let work = tempfile::tempdir().unwrap();
    // A directory cannot be read as a jar file.
    let jar = work.path().join("jar.json");
    std::fs::create_dir(&jar).unwrap();
    let out = run_jar(dir.path(), &jar, &[]);
    assert_eq!(out.status.code(), Some(3), "{}", all_output(&out));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("jar"),
        "{}",
        all_output(&out)
    );
    assert!(s.received_requests().await.unwrap().is_empty());
    assert!(jar.is_dir());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_output_ever_shows_the_cookie_value() {
    let s = MockServer::start().await;
    Mock::given(path("/login"))
        .respond_with(
            ResponseTemplate::new(200).insert_header("set-cookie", "sid=topsecretvalue; Path=/"),
        )
        .mount(&s)
        .await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("login", 1, &format!("{}/login", s.uri()))),
    ]);
    let work = tempfile::tempdir().unwrap();
    let jar = work.path().join("jar.json");
    let first = run_jar(dir.path(), &jar, &["--verbose"]);
    assert!(!all_output(&first).contains("topsecretvalue"));
    std::fs::write(&jar, "[{broken topsecretvalue").unwrap();
    let bad = run_jar(dir.path(), &jar, &["--verbose"]);
    assert!(!all_output(&bad).contains("topsecretvalue"));
    // A valid jar that holds the value, loaded and sent back, with the JSON report on stdout.
    let _ = std::fs::remove_file(&jar);
    run_jar(dir.path(), &jar, &[]);
    assert!(
        std::fs::read_to_string(&jar)
            .unwrap()
            .contains("topsecretvalue")
    );
    let third = run_jar(dir.path(), &jar, &["--verbose", "--format", "json"]);
    assert!(!all_output(&third).contains("topsecretvalue"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_script_cookie_reaches_the_jar_file_and_the_next_run_sends_it() {
    let s = login_server().await;
    let u = s.uri();
    let setter = format!(
        "{}runtime:\n  scripts:\n    - type: before-request\n      code: |-\n        await bru.cookies.add({{ key: \"made\", value: \"by-script\" }});\n",
        request("set", 1, &format!("{u}/home"))
    );
    let first_dir = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &setter)]);
    let echo = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("echo", 1, &format!("{u}/echo"))),
    ]);
    let work = tempfile::tempdir().unwrap();
    let jar = work.path().join("jar.json");
    let first = run_jar(first_dir.path(), &jar, &["--no-keychain"]);
    assert_eq!(first.status.code(), Some(0), "{}", all_output(&first));
    let second = run_jar(echo.path(), &jar, &["--no-keychain"]);
    assert_eq!(second.status.code(), Some(0), "{}", all_output(&second));
    assert_eq!(echo_cookie(&s).await, [Some("made=by-script".to_owned())]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_jar_file_with_public_suffix_cookies_warns_once_and_still_sends() {
    let s = login_server().await;
    let work = tempfile::tempdir().unwrap();
    let jar = work.path().join("jar.json");
    let wide = |domain: &str, name: &str| {
        format!(
            r#"{{"domain":"{domain}","path":"/","name":"{name}","value":"VAL{name}","expires":null,"secure":false,"httpOnly":false,"hostOnly":false}}"#
        )
    };
    std::fs::write(
        &jar,
        format!(
            "[{},{},{}]",
            wide("co.uk", "suffixa"),
            wide("com", "suffixb"),
            wide("internal", "dotless")
        ),
    )
    .unwrap();
    let dir = login_echo(&s, true);
    let out = run_jar(dir.path(), &jar, &[]);
    assert_eq!(out.status.code(), Some(0), "{}", all_output(&out));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let notes: Vec<&str> = stderr
        .lines()
        .filter(|l| l.contains("saved cookie"))
        .collect();
    assert_eq!(
        notes,
        [
            "warning: 3 saved cookies were dropped: their domains are public suffixes, so they would reach every site under them."
        ],
        "{stderr}"
    );
    let all = all_output(&out);
    for word in ["suffixa", "suffixb", "dotless", "VAL"] {
        assert!(!all.contains(word), "{word}: {all}");
    }
    assert_eq!(echo_cookie(&s).await, [None]);
    let saved = std::fs::read_to_string(&jar).unwrap();
    assert!(!saved.contains("suffixa"), "{saved}");
}
