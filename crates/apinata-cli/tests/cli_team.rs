//! `apinata team`: password and single sign-on against a fake team server, status from the
//! team file and the keychain, sign-out, and the exit codes and text each outcome gives.

use crate::common::{apinata, apinata_tokio, dead_port};
use apinata_core::team::{Account, ServerUrl, TeamStore};
use serde_json::{Value, json};
use std::path::PathBuf;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, BufReader};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn tok() -> String {
    format!("plk_{}", "A".repeat(43))
}

/// A file-backed secret store and team file, so separate runs of the binary share a sign-in.
struct Stores {
    dir: TempDir,
}

impl Stores {
    fn new() -> Stores {
        Stores {
            dir: TempDir::new().unwrap(),
        }
    }

    fn team_file(&self) -> PathBuf {
        self.dir.path().join("team.json")
    }

    fn secrets_file(&self) -> PathBuf {
        self.dir.path().join("secrets.json")
    }

    fn cmd(&self) -> assert_cmd::Command {
        let mut cmd = apinata();
        cmd.env("APINATA_TEST_TEAM_FILE", self.team_file())
            .env("APINATA_TEST_SECRETS_FILE", self.secrets_file());
        cmd
    }

    fn tokio_cmd(&self) -> tokio::process::Command {
        let mut cmd = apinata_tokio();
        cmd.env("APINATA_TEST_TEAM_FILE", self.team_file())
            .env("APINATA_TEST_SECRETS_FILE", self.secrets_file());
        cmd
    }

    fn accounts(&self) -> Vec<Account> {
        TeamStore::load(&self.team_file())
            .unwrap()
            .accounts()
            .to_vec()
    }

    fn secrets_text(&self) -> String {
        std::fs::read_to_string(self.secrets_file()).unwrap_or_default()
    }

    /// Keeps an account for ada on `server` with `expires_at`, and its token in the store.
    fn keep(&self, server: &str, expires_at: Option<&str>) {
        let url = ServerUrl::parse(server).unwrap();
        let mut account = Account::new(&url, "ada", "Ada Lovelace", "7", false);
        account.expires_at = expires_at.map(str::to_owned);
        TeamStore::update(&self.team_file(), |t| t.put_account(account)).unwrap();
        let mut entries: Vec<Value> =
            serde_json::from_str(&self.secrets_text()).unwrap_or_default();
        entries.push(json!({"collection": "", "environment": "team-server",
            "variable": format!("{url}#ada"), "value": tok()}));
        std::fs::write(self.secrets_file(), Value::from(entries).to_string()).unwrap();
    }
}

fn user(display: &str) -> Value {
    json!({"username": "ada", "display_name": display, "email": null,
           "server_admin": false, "disabled": false, "created_at": "2026-11-15T10:00:00Z"})
}

pub(crate) fn signed(display: &str) -> ResponseTemplate {
    ResponseTemplate::new(201).set_body_json(json!({"token": tok(),
        "token_meta": {"id": 7, "name": "Apinata", "prefix": "plk_AAAA",
            "created_at": "2026-11-15T10:00:00Z", "last_used_at": null,
            "expires_at": "2099-01-01T00:00:00Z"},
        "user": user(display)}))
}

pub(crate) async fn answer(s: &MockServer, verb: &str, at: &str, response: ResponseTemplate) {
    Mock::given(method(verb))
        .and(path(at))
        .respond_with(response)
        .mount(s)
        .await;
}

/// A server offering password sign-in and, with `sso`, single sign-on.
pub(crate) async fn server(sso: bool) -> MockServer {
    let s = MockServer::start().await;
    let oidc = if sso {
        json!({"name": "Corp"})
    } else {
        json!(null)
    };
    let info = json!({"version": "0.1.0", "api": 1, "auth": {"password": true, "oidc": oidc}});
    answer(
        &s,
        "GET",
        "/api/v1/server",
        ResponseTemplate::new(200).set_body_json(info),
    )
    .await;
    s
}

async fn me(s: &MockServer, display: &str) {
    let body = json!({"user": user(display),
        "identity": {"name": "Ada Lovelace", "email": "ada@users.noreply.127.0.0.1"},
        "workspaces": []});
    answer(
        s,
        "GET",
        "/api/v1/me",
        ResponseTemplate::new(200).set_body_json(body),
    )
    .await;
}

async fn calls(s: &MockServer) -> usize {
    s.received_requests().await.unwrap().len()
}

/// Runs a blocking command off the test's runtime, so the mock server keeps answering.
async fn run(mut cmd: assert_cmd::Command) -> (i32, String, String) {
    let out = tokio::task::spawn_blocking(move || cmd.output().unwrap())
        .await
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    no_token(&stdout, &stderr);
    (out.status.code().unwrap_or(-1), stdout, stderr)
}

fn no_token(stdout: &str, stderr: &str) {
    for text in [stdout, stderr] {
        assert!(!text.contains("plk_A"), "a token was printed: {text}");
    }
}

fn sign_in(stores: &Stores, server: &str, password: &str) -> assert_cmd::Command {
    let mut cmd = stores.cmd();
    cmd.args(["team", "sign-in", server, "--username", "ada"])
        .write_stdin(format!("{password}\n"));
    cmd
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_password_sign_in_then_status_then_sign_out() {
    let s = server(false).await;
    answer(&s, "POST", "/api/v1/session", signed("Ada Lovelace")).await;
    me(&s, "Ada Lovelace").await;
    answer(&s, "DELETE", "/api/v1/tokens/7", ResponseTemplate::new(204)).await;
    let stores = Stores::new();

    let (code, stdout, stderr) = run(sign_in(&stores, &s.uri(), "hunter2")).await;
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("Ada Lovelace"), "{stdout}");
    let sent: Value = serde_json::from_slice(
        &s.received_requests()
            .await
            .unwrap()
            .iter()
            .find(|r| r.url.path() == "/api/v1/session")
            .unwrap()
            .body,
    )
    .unwrap();
    assert_eq!(sent["password"], "hunter2");
    assert_eq!(stores.accounts().len(), 1);
    assert!(stores.secrets_text().contains(&tok()));

    let mut status = stores.cmd();
    status.args(["team", "status", &s.uri()]);
    let (code, stdout, stderr) = run(status).await;
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("ada"), "{stdout}");

    let mut out = stores.cmd();
    out.args(["team", "sign-out", &s.uri()]);
    let (code, _, stderr) = run(out).await;
    assert_eq!(code, 0, "{stderr}");
    let revoked = s.received_requests().await.unwrap();
    assert!(revoked.iter().any(|r| r.url.path() == "/api/v1/tokens/7"));
    assert!(stores.accounts().is_empty());
    assert!(!stores.secrets_text().contains("plk_"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wrong_password_exits_1_and_keeps_nothing() {
    let s = server(false).await;
    let refused = ResponseTemplate::new(401)
        .set_body_json(json!({"error": {"code": "unauthenticated", "message": "no"}}));
    answer(&s, "POST", "/api/v1/session", refused).await;
    let stores = Stores::new();
    let (code, _, stderr) = run(sign_in(&stores, &s.uri(), "wrong")).await;
    assert_eq!(code, 1, "{stderr}");
    assert!(stores.accounts().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_team_file_another_process_is_changing_exits_3_and_keeps_nothing() {
    let s = server(false).await;
    answer(&s, "POST", "/api/v1/session", signed("Ada Lovelace")).await;
    answer(&s, "DELETE", "/api/v1/tokens/7", ResponseTemplate::new(204)).await;
    let stores = Stores::new();
    // This test process stands in for an app window that is mid-update.
    let lock = std::fs::File::create(stores.dir.path().join("team.json.lock")).unwrap();
    lock.try_lock().unwrap();

    let (code, _, stderr) = run(sign_in(&stores, &s.uri(), "hunter2")).await;
    assert_eq!(code, 3, "{stderr}");
    assert!(
        stderr.contains("Another Apinata window or command is changing the team file"),
        "{stderr}"
    );
    drop(lock);
    assert!(stores.accounts().is_empty());
    assert!(!stores.secrets_text().contains("plk_"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_that_is_down_exits_2() {
    let stores = Stores::new();
    let down = format!("http://127.0.0.1:{}", dead_port());
    let (code, _, stderr) = run(sign_in(&stores, &down, "hunter2")).await;
    assert_eq!(code, 2, "{stderr}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_expired_sign_in_exits_1_without_calling_the_server() {
    let s = server(false).await;
    me(&s, "Ada Lovelace").await;
    let stores = Stores::new();
    stores.keep(&s.uri(), Some("2020-01-01T00:00:00Z"));
    let mut status = stores.cmd();
    status.args(["team", "status", &s.uri()]);
    let (code, _, stderr) = run(status).await;
    assert_eq!(code, 1);
    let want = format!(
        "the sign-in to {0} has expired; run \"apinata team sign-in {0}\"",
        s.uri()
    );
    assert!(stderr.contains(&want), "{stderr}");
    assert_eq!(calls(&s).await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_keychain_refuses_sign_in_and_sign_out_before_any_call() {
    let s = server(false).await;
    answer(&s, "POST", "/api/v1/session", signed("Ada Lovelace")).await;
    let stores = Stores::new();
    let mut cmd = sign_in(&stores, &s.uri(), "hunter2");
    cmd.arg("--no-keychain");
    let (code, _, stderr) = run(cmd).await;
    assert_eq!(code, 3, "{stderr}");
    assert!(stderr.contains("--no-keychain"), "{stderr}");

    stores.keep(&s.uri(), None);
    let mut out = stores.cmd();
    out.args(["team", "sign-out", &s.uri(), "--no-keychain"]);
    let (code, _, stderr) = run(out).await;
    assert_eq!(code, 3, "{stderr}");
    assert_eq!(stores.accounts().len(), 1);
    assert_eq!(calls(&s).await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_keychain_status_shows_the_local_account_unchecked() {
    let s = server(false).await;
    let stores = Stores::new();
    stores.keep(&s.uri(), Some("2099-01-01T00:00:00Z"));
    let mut status = stores.cmd();
    status.args(["team", "status", "--no-keychain"]);
    let (code, stdout, stderr) = run(status).await;
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stdout.contains("ada") && stdout.contains("2099-01-01"),
        "{stdout}"
    );
    assert!(stdout.contains("not checked"), "{stdout}");
    assert_eq!(calls(&s).await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_keychain_status_opens_no_secret_store() {
    let s = server(false).await;
    me(&s, "Ada Lovelace").await;
    let stores = Stores::new();
    stores.keep(&s.uri(), None);
    // The report line shows when a store is used, so the run with the keychain proves it works.
    let mut checked = stores.cmd();
    checked
        .env("APINATA_TEST_REPORT_STORE", "1")
        .args(["team", "status"]);
    let (_, _, stderr) = run(checked).await;
    assert!(stderr.contains("secret store:"), "{stderr}");
    let mut local = stores.cmd();
    local
        .env("APINATA_TEST_REPORT_STORE", "1")
        .args(["team", "status", "--no-keychain"]);
    let (code, _, stderr) = run(local).await;
    assert_eq!(code, 0, "{stderr}");
    assert!(!stderr.contains("secret store:"), "{stderr}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_over_several_servers_exits_with_the_highest_code() {
    let valid = server(false).await;
    me(&valid, "Ada Lovelace").await;
    let expired = server(false).await;
    let stores = Stores::new();
    stores.keep(&valid.uri(), None);
    stores.keep(&expired.uri(), Some("2020-01-01T00:00:00Z"));
    let mut status = stores.cmd();
    status.args(["team", "status"]);
    let (code, stdout, stderr) = run(status).await;
    assert_eq!(code, 1, "{stdout}{stderr}");
    assert!(stdout.contains("signed in as"), "{stdout}");

    let down = format!("http://127.0.0.1:{}", dead_port());
    stores.keep(&down, None);
    let mut status = stores.cmd();
    status.args(["team", "status", "--format", "json"]);
    let (code, stdout, stderr) = run(status).await;
    assert_eq!(code, 2, "{stdout}{stderr}");
    let states: Vec<String> = serde_json::from_str::<Vec<Value>>(&stdout)
        .unwrap()
        .iter()
        .map(|r| r["state"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(states, ["signed_in", "expired", "error"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_with_a_refused_token_reports_signed_out_and_forgets_the_token() {
    let s = server(false).await;
    let refused = ResponseTemplate::new(401)
        .set_body_json(json!({"error": {"code": "unauthenticated", "message": "no"}}));
    answer(&s, "GET", "/api/v1/me", refused).await;
    let stores = Stores::new();
    stores.keep(&s.uri(), None);
    let mut status = stores.cmd();
    status.args(["team", "status", "--format", "json"]);
    let (code, stdout, stderr) = run(status).await;
    assert_eq!(code, 1, "{stderr}");
    let rows: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(rows[0]["state"], "signed_out");
    assert!(stderr.contains("no longer accepted"), "{stderr}");
    assert!(!stores.secrets_text().contains("plk_"));
    assert_eq!(stores.accounts().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sign_out_when_not_signed_in_exits_0_without_a_call() {
    let s = server(false).await;
    let stores = Stores::new();
    let mut out = stores.cmd();
    out.args(["team", "sign-out", &s.uri()]);
    let (code, stdout, stderr) = run(out).await;
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("Not signed in"), "{stdout}");
    assert_eq!(calls(&s).await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn json_status_has_no_token_fields() {
    let s = server(false).await;
    me(&s, "Ada Lovelace").await;
    let stores = Stores::new();
    stores.keep(&s.uri(), None);
    let mut status = stores.cmd();
    status.args(["team", "status", "--format", "json"]);
    let (code, stdout, stderr) = run(status).await;
    assert_eq!(code, 0, "{stderr}");
    let rows: Value = serde_json::from_str(&stdout).unwrap();
    let row = &rows.as_array().unwrap()[0];
    assert_eq!(row["username"], "ada");
    assert_eq!(row["state"], "signed_in");
    assert!(
        row.as_object()
            .unwrap()
            .keys()
            .all(|k| !k.contains("token")),
        "{row}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hidden_character_in_the_display_name_is_escaped() {
    let s = server(false).await;
    answer(&s, "POST", "/api/v1/session", signed("Ada\u{202E}evil")).await;
    me(&s, "Ada\u{202E}evil").await;
    let stores = Stores::new();
    let (code, stdout, stderr) = run(sign_in(&stores, &s.uri(), "hunter2")).await;
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("Ada\\u{202E}evil"), "{stdout}");
    assert!(!stdout.contains('\u{202E}'));

    let mut status = stores.cmd();
    status.args(["team", "status", "--format", "json"]);
    let (_, stdout, _) = run(status).await;
    assert!(
        !stdout.contains('\u{202E}') && !stdout.contains("\\u202e"),
        "{stdout}"
    );
    assert!(stdout.contains("Ada\\\\u{202E}evil"), "{stdout}");
}

/// Starts an SSO sign-in that opens no browser and reads the start URL off stderr.
async fn start_sso(stores: &Stores, s: &MockServer) -> (tokio::process::Child, url::Url) {
    let (child, url, _, _) = start_sso_with(stores, s, |cmd| {
        cmd.arg("--no-browser");
    })
    .await;
    (child, url)
}

/// Starts an SSO sign-in after `setup` adjusts the command; returns the child, the start URL
/// the stderr seen up to the wait line, and a task yielding the rest of stderr.
async fn start_sso_with(
    stores: &Stores,
    s: &MockServer,
    setup: impl FnOnce(&mut tokio::process::Command),
) -> (
    tokio::process::Child,
    url::Url,
    String,
    tokio::task::JoinHandle<String>,
) {
    let mut cmd = stores.tokio_cmd();
    cmd.args(["team", "sign-in", &s.uri(), "--sso"])
        .stdin(std::process::Stdio::null());
    setup(&mut cmd);
    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
    let mut seen = String::new();
    let found = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut start = None;
        while let Some(line) = lines.next_line().await.unwrap() {
            assert!(!line.contains("plk_A"));
            seen.push_str(&line);
            seen.push('\n');
            // The start URL is the one line that is nothing but an address; the wait line,
            // printed once the port is listening, follows it.
            match url::Url::parse(line.trim()) {
                Ok(url) if url.path() == "/api/v1/oidc/start" => start = Some(url),
                _ if start.is_some() && line.contains("Ctrl+C") => return start,
                _ => {}
            }
        }
        None
    })
    .await
    .unwrap();
    let url = found.unwrap_or_else(|| panic!("no start URL in: {seen}"));
    assert!(seen.contains("5 minutes"), "{seen}");
    let port = param(&url, "port");
    assert!(seen.contains(&format!("port {port}")), "{seen}");
    // The rest of stderr must still drain, or the child blocks on a full pipe.
    let rest = tokio::spawn(async move {
        let mut rest = String::new();
        while let Ok(Some(line)) = lines.next_line().await {
            assert!(!line.contains("plk_A"));
            rest.push_str(&line);
            rest.push('\n');
        }
        rest
    });
    (child, url, seen, rest)
}

fn param(url: &url::Url, key: &str) -> String {
    url.query_pairs()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
        .unwrap_or_else(|| panic!("{key} missing from {url}"))
}

/// Plays the browser coming back to the loopback port with a code, then waits for the child.
async fn come_back(child: tokio::process::Child, url: &url::Url) -> std::process::Output {
    let port: u16 = param(url, "port").parse().unwrap();
    let state = param(url, "state");
    let _ = tokio::task::spawn_blocking(move || {
        use std::io::{Read, Write};
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(
            s,
            "GET /callback?code=the-code&state={state} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut page = String::new();
        let _ = s.read_to_string(&mut page);
    })
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(30), child.wait_with_output())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn single_sign_on_redeems_the_code_the_browser_brings_back() {
    let s = server(true).await;
    answer(&s, "POST", "/api/v1/oidc/redeem", signed("Ada Lovelace")).await;
    let stores = Stores::new();
    let (child, url) = start_sso(&stores, &s).await;
    let out = come_back(child, &url).await;
    let stdout = String::from_utf8(out.stdout).unwrap();
    no_token(&stdout, "");
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    let redeem = s.received_requests().await.unwrap();
    let redeem = redeem
        .iter()
        .find(|r| r.url.path() == "/api/v1/oidc/redeem")
        .expect("redeemed");
    let body: Value = serde_json::from_slice(&redeem.body).unwrap();
    assert_eq!(body["code"], "the-code");
    assert!(body["verifier"].as_str().is_some_and(|v| !v.is_empty()));
    assert_eq!(stores.accounts().len(), 1);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ctrl_c_during_single_sign_on_exits_130_and_keeps_nothing() {
    let s = server(true).await;
    let stores = Stores::new();
    let (mut child, _) = start_sso(&stores, &s).await;
    let id = child.id().unwrap().to_string();
    let killed = std::process::Command::new("kill")
        .args(["-INT", &id])
        .status()
        .unwrap();
    assert!(killed.success());
    let status = tokio::time::timeout(std::time::Duration::from_secs(30), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status.code(), Some(130));
    assert!(stores.accounts().is_empty());
    assert!(!stores.secrets_text().contains("plk_"));
}

/// Starts an SSO sign-in with piped stdin and pastes `lines` into it, then waits for the
/// child; returns its output and all of its stderr.
async fn paste_sso(
    s: &MockServer,
    stores: &Stores,
    lines: impl Fn(&str) -> Vec<String>,
) -> (std::process::Output, String) {
    use tokio::io::AsyncWriteExt as _;
    let (mut child, url, seen, rest) = start_sso_with(stores, s, |cmd| {
        cmd.arg("--no-browser").stdin(std::process::Stdio::piped());
    })
    .await;
    let redirect = format!("http://127.0.0.1:{}/callback", param(&url, "port"));
    let mut stdin = child.stdin.take().unwrap();
    for line in lines(&format!(
        "{redirect}?code=the-code&state={}",
        param(&url, "state")
    )) {
        stdin
            .write_all(format!("{line}\n").as_bytes())
            .await
            .unwrap();
    }
    let out = tokio::time::timeout(std::time::Duration::from_secs(30), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    drop(stdin);
    let rest = tokio::time::timeout(std::time::Duration::from_secs(30), rest)
        .await
        .expect("stderr closed")
        .unwrap();
    (out, format!("{seen}{rest}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pasted_redirect_completes_single_sign_on_without_printing_the_code() {
    let s = server(true).await;
    answer(&s, "POST", "/api/v1/oidc/redeem", signed("Ada Lovelace")).await;
    let stores = Stores::new();
    let (out, stderr) = paste_sso(&s, &stores, |valid| vec![format!("  {valid}  ")]).await;
    let stdout = String::from_utf8(out.stdout).unwrap();
    no_token(&stdout, &stderr);
    assert_eq!(out.status.code(), Some(0), "{stdout}{stderr}");
    assert!(
        stderr.contains("Or paste the address your browser ended on (within a minute):"),
        "{stderr}"
    );
    for text in [&stdout, &stderr] {
        assert!(!text.contains("the-code"), "the code was printed: {text}");
    }
    let bodies = s.received_requests().await.unwrap();
    let redeem: Vec<_> = bodies
        .iter()
        .filter(|r| r.url.path() == "/api/v1/oidc/redeem")
        .collect();
    assert_eq!(redeem.len(), 1);
    let body: Value = serde_json::from_slice(&redeem[0].body).unwrap();
    assert_eq!(body["code"], "the-code");
    assert_eq!(stores.accounts().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_paste_is_explained_and_a_correct_one_still_signs_in() {
    let s = server(true).await;
    answer(&s, "POST", "/api/v1/oidc/redeem", signed("Ada Lovelace")).await;
    let stores = Stores::new();
    let (out, stderr) = paste_sso(&s, &stores, |valid| {
        let (head, _) = valid.split_once("state=").unwrap();
        vec![
            format!("{head}state=forged-state"),
            format!("{valid}&{}", "x".repeat(9 * 1024)),
            String::new(),
            valid.to_owned(),
        ]
    })
    .await;
    let stdout = String::from_utf8(out.stdout).unwrap();
    no_token(&stdout, &stderr);
    assert_eq!(out.status.code(), Some(0), "{stdout}{stderr}");
    assert!(
        stderr.contains("that address belongs to another sign-in"),
        "{stderr}"
    );
    assert!(stderr.contains("longer than 8 KiB"), "{stderr}");
    for text in [&stdout, &stderr] {
        assert!(
            !text.contains("the-code") && !text.contains("forged"),
            "{text}"
        );
    }
    assert_eq!(stores.accounts().len(), 1);
    let calls = s.received_requests().await.unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|r| r.url.path() == "/api/v1/oidc/redeem")
            .count(),
        1
    );
}

/// A token as a CI job would hold it, distinct from the one the keychain keeps.
fn env_tok() -> String {
    format!("plk_{}", "E".repeat(43))
}

/// The command with the token for `server` in the environment.
fn with_env(stores: &Stores, server: &str) -> assert_cmd::Command {
    let mut cmd = stores.cmd();
    cmd.env("APINATA_TEAM_TOKEN", env_tok())
        .env("APINATA_TEAM_SERVER", server);
    cmd
}

/// Runs `cmd` and also checks no token of any kind reached stdout or stderr.
async fn run_clean(cmd: assert_cmd::Command) -> (i32, String, String) {
    let (code, stdout, stderr) = run(cmd).await;
    assert!(
        !stdout.contains("plk_") && !stderr.contains("plk_"),
        "{stdout}{stderr}"
    );
    (code, stdout, stderr)
}

/// The Authorization headers `s` received.
async fn bearers(s: &MockServer) -> Vec<String> {
    s.received_requests()
        .await
        .unwrap()
        .iter()
        .filter_map(|r| r.headers.get("authorization"))
        .map(|v| v.to_str().unwrap().to_owned())
        .collect()
}

/// Both store files with content, so a test can show neither was rewritten.
fn seed_files(stores: &Stores) -> (Vec<u8>, Vec<u8>) {
    TeamStore::update(&stores.team_file(), |_| {}).unwrap();
    std::fs::write(stores.secrets_file(), "[]").unwrap();
    (
        std::fs::read(stores.team_file()).unwrap(),
        std::fs::read(stores.secrets_file()).unwrap(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn env_status_needs_no_account_and_changes_no_store() {
    let s = server(false).await;
    me(&s, "Ada Lovelace").await;
    let stores = Stores::new();
    let (team, secrets) = seed_files(&stores);
    let mut status = with_env(&stores, &s.uri());
    status
        .env("APINATA_TEST_REPORT_STORE", "1")
        .args(["team", "status"]);
    let (code, stdout, stderr) = run_clean(status).await;
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stdout.contains("signed in as Ada Lovelace (ada)")
            && stdout.contains("(token from APINATA_TEAM_TOKEN)"),
        "{stdout}"
    );
    assert!(!stderr.contains("secret store:"), "{stderr}");
    assert_eq!(bearers(&s).await, [format!("Bearer {}", env_tok())]);
    assert_eq!(std::fs::read(stores.team_file()).unwrap(), team);
    assert_eq!(std::fs::read(stores.secrets_file()).unwrap(), secrets);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn env_status_in_json_names_the_environment_as_the_source() {
    let s = server(false).await;
    me(&s, "Ada Lovelace").await;
    let stores = Stores::new();
    let mut status = with_env(&stores, &s.uri());
    status.args(["team", "status", &s.uri(), "--format", "json"]);
    let (code, stdout, stderr) = run_clean(status).await;
    assert_eq!(code, 0, "{stderr}");
    let rows: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(rows[0]["source"], "environment", "{rows}");
    assert_eq!(rows[0]["state"], "signed_in", "{rows}");
    assert_eq!(rows[0]["username"], "ada", "{rows}");
    assert_eq!(rows.as_array().unwrap().len(), 1, "{rows}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_env_token_exits_1_and_keeps_the_kept_sign_in() {
    let s = server(false).await;
    let refused = ResponseTemplate::new(401)
        .set_body_json(json!({"error": {"code": "unauthenticated", "message": "no"}}));
    answer(&s, "GET", "/api/v1/me", refused).await;
    let stores = Stores::new();
    stores.keep(&s.uri(), None);
    let team = std::fs::read(stores.team_file()).unwrap();
    let secrets = std::fs::read(stores.secrets_file()).unwrap();
    let mut status = with_env(&stores, &s.uri());
    status.args(["team", "status"]);
    let (code, _, stderr) = run_clean(status).await;
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.contains("APINATA_TEAM_TOKEN"), "{stderr}");
    assert_eq!(bearers(&s).await, [format!("Bearer {}", env_tok())]);
    assert_eq!(std::fs::read(stores.team_file()).unwrap(), team);
    assert_eq!(std::fs::read(stores.secrets_file()).unwrap(), secrets);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_token_without_a_server_exits_3_before_any_request() {
    let s = server(false).await;
    me(&s, "Ada Lovelace").await;
    let stores = Stores::new();
    stores.keep(&s.uri(), None);
    for server in [None, Some("")] {
        let mut status = stores.cmd();
        status.env("APINATA_TEAM_TOKEN", env_tok());
        if let Some(server) = server {
            status.env("APINATA_TEAM_SERVER", server);
        }
        status.args(["team", "status"]);
        let (code, _, stderr) = run_clean(status).await;
        assert_eq!(code, 3, "{stderr}");
        assert!(stderr.contains("APINATA_TEAM_SERVER"), "{stderr}");
    }
    assert_eq!(calls(&s).await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_malformed_token_names_the_variable_and_hides_the_value() {
    let s = server(false).await;
    let stores = Stores::new();
    let mut status = stores.cmd();
    status
        .env("APINATA_TEAM_TOKEN", "plk_not-a-real-token-Zq81")
        .env("APINATA_TEAM_SERVER", s.uri())
        .args(["team", "status"]);
    let (code, stdout, stderr) = run_clean(status).await;
    assert_eq!(code, 3, "{stderr}");
    assert!(stderr.contains("APINATA_TEAM_TOKEN"), "{stderr}");
    assert!(!format!("{stdout}{stderr}").contains("Zq81"), "{stderr}");
    assert_eq!(calls(&s).await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bad_server_variable_names_the_variable() {
    let stores = Stores::new();
    let mut status = with_env(&stores, "ftp://nope.invalid");
    status.args(["team", "status"]);
    let (code, _, stderr) = run_clean(status).await;
    assert_eq!(code, 3, "{stderr}");
    assert!(stderr.contains("APINATA_TEAM_SERVER"), "{stderr}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_empty_token_counts_as_unset() {
    let s = server(false).await;
    let stores = Stores::new();
    let mut status = stores.cmd();
    status
        .env("APINATA_TEAM_TOKEN", "")
        .env("APINATA_TEAM_SERVER", s.uri())
        .args(["team", "status"]);
    let (code, _, stderr) = run_clean(status).await;
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.contains("not signed in to any"), "{stderr}");
    assert_eq!(calls(&s).await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_env_token_never_goes_to_another_server() {
    let kept = server(false).await;
    me(&kept, "Ada Lovelace").await;
    let env_server = server(false).await;
    me(&env_server, "Ada Lovelace").await;
    let stores = Stores::new();
    stores.keep(&kept.uri(), None);
    let mut status = with_env(&stores, &env_server.uri());
    status.args(["team", "status", "--format", "json"]);
    let (code, stdout, stderr) = run_clean(status).await;
    assert_eq!(code, 0, "{stdout}{stderr}");
    let sent = bearers(&kept).await;
    assert!(!sent.is_empty());
    assert!(sent.iter().all(|b| !b.contains(&env_tok())), "{sent:?}");
    assert_eq!(
        bearers(&env_server).await,
        [format!("Bearer {}", env_tok())]
    );
    let rows: Vec<Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(rows.len(), 2, "{stdout}");

    // Asked about the kept server only, the environment's server is not reached at all.
    let mut one = with_env(&stores, &env_server.uri());
    one.args(["team", "status", &kept.uri()]);
    let (code, _, stderr) = run_clean(one).await;
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(bearers(&env_server).await.len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_env_token_wins_over_the_keychain_for_its_server() {
    let s = server(false).await;
    me(&s, "Ada Lovelace").await;
    let stores = Stores::new();
    stores.keep(&s.uri(), None);
    let mut status = with_env(&stores, &s.uri());
    status.args(["team", "status", "--format", "json"]);
    let (code, stdout, stderr) = run_clean(status).await;
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(bearers(&s).await, [format!("Bearer {}", env_tok())]);
    let rows: Vec<Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(rows.len(), 1, "{stdout}");
    assert_eq!(rows[0]["source"], "environment");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn env_status_works_with_no_keychain() {
    let s = server(false).await;
    me(&s, "Ada Lovelace").await;
    let stores = Stores::new();
    let mut status = with_env(&stores, &s.uri());
    status
        .env("APINATA_TEST_REPORT_STORE", "1")
        .args(["team", "status", "--no-keychain"]);
    let (code, stdout, stderr) = run_clean(status).await;
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stdout.contains("(token from APINATA_TEAM_TOKEN)"),
        "{stdout}"
    );
    assert!(!stderr.contains("secret store:"), "{stderr}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sign_out_refuses_for_the_env_server_and_deletes_nothing() {
    let s = server(false).await;
    answer(&s, "DELETE", "/api/v1/tokens/7", ResponseTemplate::new(204)).await;
    let stores = Stores::new();
    stores.keep(&s.uri(), None);
    let secrets = stores.secrets_text();
    let mut out = with_env(&stores, &s.uri());
    out.args(["team", "sign-out", &s.uri()]);
    let (code, _, stderr) = run_clean(out).await;
    assert_eq!(code, 3, "{stderr}");
    assert!(
        stderr.contains("the token comes from the environment"),
        "{stderr}"
    );
    assert_eq!(stores.accounts().len(), 1);
    assert_eq!(stores.secrets_text(), secrets);
    assert_eq!(calls(&s).await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sign_in_ignores_the_variables() {
    let s = server(false).await;
    answer(&s, "POST", "/api/v1/session", signed("Ada Lovelace")).await;
    let stores = Stores::new();
    let mut cmd = sign_in(&stores, &s.uri(), "hunter2");
    cmd.env("APINATA_TEAM_TOKEN", env_tok());
    let (code, _, stderr) = run_clean(cmd).await;
    assert_eq!(code, 0, "{stderr}");
    assert!(stores.secrets_text().contains(&tok()));
    assert!(!stores.secrets_text().contains(&env_tok()));
}

#[test]
fn a_template_cannot_read_the_env_token() {
    let dir = crate::common::write_tree(&[
        ("opencollection.yml", crate::common::ROOT),
        (
            "r.yml",
            "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://127.0.0.1:1/{{process.env.APINATA_TEAM_TOKEN}}\"\n",
        ),
    ]);
    let home = TempDir::new().unwrap();
    let out = apinata()
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("APINATA_TEAM_TOKEN", env_tok())
        .args(["run", "--no-keychain"])
        .arg(dir.path())
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!out.status.success(), "{text}");
    assert!(!text.contains("plk_"), "{text}");
    assert!(
        text.contains("variable `process.env.APINATA_TEAM_TOKEN` is not defined"),
        "{text}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plain_http_server_that_is_not_local_is_refused() {
    let stores = Stores::new();
    let mut status = with_env(&stores, "http://team.invalid");
    status.args(["team", "status"]);
    let (code, _, stderr) = run_clean(status).await;
    assert_eq!(code, 3, "{stderr}");
    assert!(
        stderr.contains("APINATA_TEAM_SERVER") && stderr.contains("https"),
        "{stderr}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bad_env_token_does_not_block_signing_out_of_another_server() {
    let s = server(false).await;
    answer(&s, "DELETE", "/api/v1/tokens/7", ResponseTemplate::new(204)).await;
    let stores = Stores::new();
    stores.keep(&s.uri(), None);
    let mut out = stores.cmd();
    out.env("APINATA_TEAM_TOKEN", "not-a-token")
        .env("APINATA_TEAM_SERVER", "https://elsewhere.invalid")
        .args(["team", "sign-out", &s.uri()]);
    let (code, _, stderr) = run_clean(out).await;
    assert_eq!(code, 0, "{stderr}");
    assert!(stores.accounts().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bad_env_token_for_this_server_still_refuses_sign_out() {
    let s = server(false).await;
    let stores = Stores::new();
    stores.keep(&s.uri(), None);
    let mut out = stores.cmd();
    out.env("APINATA_TEAM_TOKEN", "not-a-token")
        .env("APINATA_TEAM_SERVER", s.uri())
        .args(["team", "sign-out", &s.uri()]);
    let (code, _, stderr) = run_clean(out).await;
    assert_eq!(code, 3, "{stderr}");
    assert_eq!(stores.accounts().len(), 1);
    assert_eq!(calls(&s).await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_corrupt_team_file_does_not_hide_the_env_server() {
    let s = server(false).await;
    me(&s, "Ada Lovelace").await;
    let stores = Stores::new();
    std::fs::write(stores.team_file(), "{ not json").unwrap();

    // Asked about the environment's server alone, the team file is never needed.
    let mut one = with_env(&stores, &s.uri());
    one.args(["team", "status", &s.uri()]);
    let (code, stdout, stderr) = run_clean(one).await;
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stdout.contains("(token from APINATA_TEAM_TOKEN)"),
        "{stdout}"
    );

    // Asked about every server, the env row comes first and the file problem still fails.
    let mut all = with_env(&stores, &s.uri());
    all.args(["team", "status", "--format", "json"]);
    let (code, stdout, stderr) = run_clean(all).await;
    assert_eq!(code, 3, "{stderr}");
    let rows: Vec<Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(rows.len(), 1, "{stdout}");
    assert_eq!(rows[0]["source"], "environment");
    assert!(stderr.contains("error:"), "{stderr}");
    assert_eq!(
        std::fs::read_to_string(stores.team_file()).unwrap(),
        "{ not json"
    );
}

/// The opener `team sign-in --sso` starts, played by a fake `xdg-open` first on PATH.
#[cfg(all(unix, not(target_os = "macos")))]
mod browser {
    use super::*;
    use std::path::Path;
    use std::time::{Duration, Instant};

    const FALLBACK: &str = "could not open a browser; open the address above";

    /// A directory holding an `xdg-open` that records its argv and env to `record`, prints
    /// junk on both streams and then sleeps far longer than a sign-in takes.
    fn fake_opener(record: &Path) -> TempDir {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = TempDir::new().unwrap();
        let script = dir.path().join("xdg-open");
        let tmp = record.with_extension("tmp");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\n{{ echo \"pid $$\"; for a in \"$@\"; do echo \"arg $a\"; done; env | sed 's/^/env /'; }} > '{tmp}'\nmv '{tmp}' '{rec}'\necho OPENER-JUNK-OUT\necho OPENER-JUNK-ERR >&2\nexec sleep 30\n",
                tmp = tmp.display(),
                rec = record.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        dir
    }

    fn path_with(dir: &Path) -> String {
        format!(
            "{}:{}",
            dir.display(),
            std::env::var("PATH").unwrap_or_default()
        )
    }

    /// The record the fake wrote, waiting up to ten seconds for it.
    async fn recorded(record: &Path) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(text) = std::fs::read_to_string(record) {
                return text;
            }
            assert!(Instant::now() < deadline, "the opener never ran");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Stops the sleeping fake by the pid it recorded.
    fn stop(record: &str) {
        if let Some(pid) = record.lines().find_map(|l| l.strip_prefix("pid ")) {
            let _ = std::process::Command::new("kill").arg(pid).status();
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_start_url_opens_in_the_browser_without_holding_up_the_sign_in() {
        let s = server(true).await;
        answer(&s, "POST", "/api/v1/oidc/redeem", signed("Ada Lovelace")).await;
        let stores = Stores::new();
        let scratch = TempDir::new().unwrap();
        let record = scratch.path().join("opened.txt");
        let fake = fake_opener(&record);
        let begun = Instant::now();
        let (child, url, seen, rest) = start_sso_with(&stores, &s, |cmd| {
            cmd.env("PATH", path_with(fake.path()))
                .env("DISPLAY", ":99")
                .env("CI", "")
                .env("APINATA_TEAM_TOKEN", env_tok());
        })
        .await;
        let out = come_back(child, &url).await;
        let rest = tokio::time::timeout(Duration::from_secs(30), rest)
            .await
            .expect("stderr closed")
            .unwrap();
        let elapsed = begun.elapsed();
        let opened = recorded(&record).await;
        stop(&opened);
        let stdout = String::from_utf8(out.stdout).unwrap();
        assert_eq!(out.status.code(), Some(0), "{seen}{rest}");
        assert!(elapsed < Duration::from_secs(20), "took {elapsed:?}");
        assert!(opened.contains(&format!("arg {url}")), "{opened}");
        assert!(!opened.contains("APINATA_TEAM_TOKEN"), "{opened}");
        let all = format!("{stdout}{seen}{rest}");
        assert!(!all.contains("OPENER-JUNK"), "{all}");
        assert!(!all.contains(FALLBACK), "{all}");
        assert_eq!(stores.accounts().len(), 1);
    }

    /// Signs in with the fake opener on PATH after `setup`, and checks it was never started.
    async fn assert_not_opened(setup: impl FnOnce(&mut tokio::process::Command)) {
        let s = server(true).await;
        answer(&s, "POST", "/api/v1/oidc/redeem", signed("Ada Lovelace")).await;
        let stores = Stores::new();
        let scratch = TempDir::new().unwrap();
        let record = scratch.path().join("opened.txt");
        let fake = fake_opener(&record);
        let (child, url, seen, _) = start_sso_with(&stores, &s, |cmd| {
            cmd.env("PATH", path_with(fake.path()));
            setup(cmd);
        })
        .await;
        let out = come_back(child, &url).await;
        assert_eq!(out.status.code(), Some(0), "{seen}");
        tokio::time::sleep(Duration::from_secs(1)).await;
        if let Ok(opened) = std::fs::read_to_string(&record) {
            stop(&opened);
            panic!("the opener ran: {opened}");
        }
        assert!(!seen.contains(FALLBACK), "{seen}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn no_browser_opens_nothing() {
        assert_not_opened(|cmd| {
            cmd.env("DISPLAY", ":99").arg("--no-browser");
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ci_opens_nothing() {
        assert_not_opened(|cmd| {
            cmd.env("DISPLAY", ":99").env("CI", "1");
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn no_display_opens_nothing() {
        assert_not_opened(|cmd| {
            cmd.env("DISPLAY", "").env_remove("WAYLAND_DISPLAY");
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn with_no_opener_the_address_is_left_to_the_user() {
        let s = server(true).await;
        answer(&s, "POST", "/api/v1/oidc/redeem", signed("Ada Lovelace")).await;
        let stores = Stores::new();
        let empty = TempDir::new().unwrap();
        let (child, url, seen, _) = start_sso_with(&stores, &s, |cmd| {
            cmd.env("PATH", empty.path())
                .env("WAYLAND_DISPLAY", "wayland-9");
        })
        .await;
        assert!(seen.contains(FALLBACK), "{seen}");
        let out = come_back(child, &url).await;
        assert_eq!(out.status.code(), Some(0), "{seen}");
        assert_eq!(stores.accounts().len(), 1);
    }
}
