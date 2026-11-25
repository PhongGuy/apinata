//! Single sign-on from the join dialog: the button the server's answer offers, the browser wait
//! with its copyable URL, the session it keeps, and the prompt after a sign-in expired.

use crate::support::potluck::{error, tok, token_meta, user};
use crate::support::team::*;
use crate::support::*;
use apinata_app::team_join::{self, JoinPhase, JoinView};
use apinata_core::SecretStore;
use apinata_core::team::{Account, TeamStore, token_key};
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use serde_json::{Value, json};
use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const REDEEM: &str = "/api/v1/oidc/redeem";

fn answer(cx: &mut TestAppContext, server: &MockServer, verb: &str, at: &str, r: ResponseTemplate) {
    mount(
        cx,
        server,
        Mock::given(method(verb)).and(path(at)).respond_with(r),
    );
}

/// The server answer with `password` sign-in and `oidc` as given.
fn info_with(password: bool, oidc: Option<Value>) -> ResponseTemplate {
    let mut auth = json!({"password": password});
    if let Some(oidc) = oidc {
        auth["oidc"] = oidc;
    }
    ResponseTemplate::new(200).set_body_json(json!({"version": "0.1.0", "api": 1, "auth": auth}))
}

fn acme() -> ResponseTemplate {
    info_with(true, Some(json!({"name": "Acme ID"})))
}

fn redeemed(id: i64, expires_at: &str) -> ResponseTemplate {
    let mut meta = token_meta(id);
    meta["expires_at"] = json!(expires_at);
    ResponseTemplate::new(201)
        .set_body_json(json!({"token": tok(), "token_meta": meta, "user": user("ada", false)}))
}

/// Installs a browser opener that records each URL instead of opening it.
fn recording_opener(cx: &mut TestAppContext) -> Arc<Mutex<Vec<String>>> {
    let opened = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&opened);
    cx.update(|cx| {
        apinata_app::oauth2_signin::set_opener(
            Arc::new(move |url: &str, _: &mut gpui_kit::App| {
                seen.lock().unwrap().push(url.to_owned())
            }),
            cx,
        )
    });
    opened
}

fn open_join(cx: &mut TestAppContext, s: &Started, server: &str) -> Entity<JoinView> {
    let ws = s.ws.clone();
    let server = server.to_owned();
    let view = cx
        .update_window(s.win, |_, w, cx| {
            team_join::open_for(&ws, &server, "", w, cx)
        })
        .unwrap();
    frame(cx, s.win);
    view
}

fn sso_name(cx: &mut TestAppContext, view: &Entity<JoinView>) -> Option<String> {
    view.read_with(cx, |v, _| v.sso_name().map(str::to_owned))
}

fn checked(cx: &mut TestAppContext, server: &MockServer) {
    wait_for(cx, "the server check", |cx| {
        hits_on(cx, server, "GET", "/api/v1/server") > 0
    });
    // The answer lands on the UI thread after the request is seen.
    for _ in 0..10 {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

fn click(cx: &mut TestAppContext, s: &Started, id: &'static str) {
    use gpui_kit::test::TestWindowExt as _;
    cx.update_window(s.win, |_, w, cx| w.click(id, cx)).unwrap();
    frame(cx, s.win);
}

fn param(url: &str, key: &str) -> String {
    url::Url::parse(url)
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
        .unwrap_or_else(|| panic!("{key} missing from the start URL"))
}

/// Plays the browser landing on the app's loopback port named in `start` with `query`.
fn land(start: &str, query: String) {
    let port: u16 = param(start, "port").parse().unwrap();
    std::thread::spawn(move || {
        let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        write!(
            s,
            "GET /callback?{query} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
    })
    .join()
    .unwrap();
}

/// Starts single sign-on from the button and returns the URL the dialog shows.
fn start_sso(cx: &mut TestAppContext, s: &Started, view: &Entity<JoinView>) -> String {
    click(cx, s, "team-join-sso");
    wait_for(cx, "the browser wait", |cx| {
        view.read_with(cx, |v, _| v.sso_url().is_some())
    });
    assert_eq!(view.read_with(cx, |v, _| v.phase()), JoinPhase::Browser);
    view.read_with(cx, |v, _| v.sso_url().unwrap().to_owned())
}

#[gpui_kit::test]
fn the_button_shows_only_when_the_server_names_a_provider(cx: &mut TestAppContext) {
    http_ready(cx);
    let offers = potluck_server(cx);
    answer(cx, &offers, "GET", "/api/v1/server", acme());
    let (_col, s, _secrets) = team_app(cx, |_, _| {});
    let view = open_join(cx, &s, &offers.uri());
    checked(cx, &offers);
    assert_eq!(sso_name(cx, &view).as_deref(), Some("Acme ID"));
    assert!(shown(cx, s.win, "team-join-sso"));
    click(cx, &s, "team-join-cancel");
    for oidc in [Some(Value::Null), Some(json!(false)), None] {
        let off = potluck_server(cx);
        answer(cx, &off, "GET", "/api/v1/server", info_with(true, oidc));
        let view = open_join(cx, &s, &off.uri());
        checked(cx, &off);
        assert_eq!(sso_name(cx, &view), None);
        assert!(!shown(cx, s.win, "team-join-sso"));
        click(cx, &s, "team-join-cancel");
    }
}

#[gpui_kit::test]
fn single_sign_on_keeps_the_token_and_its_expiry(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = potluck_server(cx);
    answer(cx, &server, "GET", "/api/v1/server", acme());
    answer(
        cx,
        &server,
        "POST",
        REDEEM,
        redeemed(9, "2999-01-01T00:00:00Z"),
    );
    let opened = recording_opener(cx);
    let (_col, s, secrets) = team_app(cx, |_, _| {});
    let view = open_join(cx, &s, &server.uri());
    checked(cx, &server);
    let start = start_sso(cx, &s, &view);
    assert!(start.starts_with(&format!("{}/api/v1/oidc/start?", server.uri())));
    assert_eq!(*opened.lock().unwrap(), vec![start.clone()]);
    assert!(shown(cx, s.win, "team-join-sso-url"));
    assert!(shown(cx, s.win, "team-join-copy-url"));
    let state = param(&start, "state");
    land(&start, format!("code=abc&state={state}"));
    let url = server_url(&server);
    let file = team_file(&s);
    wait_for(cx, "the account", |_| {
        TeamStore::load(&file).unwrap().account(&url).is_some()
    });
    let account = TeamStore::load(&file)
        .unwrap()
        .account(&url)
        .cloned()
        .unwrap();
    assert_eq!(account.expires_at.as_deref(), Some("2999-01-01T00:00:00Z"));
    assert_eq!(secrets.get(&token_key(&url, "ada")).unwrap(), Some(tok()));
    assert_eq!(hits_on(cx, &server, "POST", REDEEM), 1);
}

#[gpui_kit::test]
fn a_return_with_another_state_is_never_redeemed(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = potluck_server(cx);
    answer(cx, &server, "GET", "/api/v1/server", acme());
    answer(
        cx,
        &server,
        "POST",
        REDEEM,
        redeemed(9, "2999-01-01T00:00:00Z"),
    );
    recording_opener(cx);
    let (_col, s, secrets) = team_app(cx, |_, _| {});
    let view = open_join(cx, &s, &server.uri());
    checked(cx, &server);
    let start = start_sso(cx, &s, &view);
    land(&start, "code=abc&state=forged".into());
    for _ in 0..10 {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(view.read_with(cx, |v, _| v.phase()), JoinPhase::Browser);
    click(cx, &s, "team-join-cancel");
    assert_eq!(hits_on(cx, &server, "POST", REDEEM), 0);
    assert_eq!(
        secrets
            .get(&token_key(&server_url(&server), "ada"))
            .unwrap(),
        None
    );
    assert!(!team_file(&s).exists());
}

#[gpui_kit::test]
fn each_refusal_returns_to_the_form_with_its_reason(cx: &mut TestAppContext) {
    http_ready(cx);
    recording_opener(cx);
    let (_col, s, secrets) = team_app(cx, |_, _| {});
    let invalid =
        || ResponseTemplate::new(400).set_body_json(error("oidc_code_invalid", "Bad code.", None));
    let unauthenticated =
        ResponseTemplate::new(401).set_body_json(error("unauthenticated", "No.", None));
    let cases = [
        ("error=access_denied", invalid(), "The sign-in was refused."),
        (
            "error=account_not_linked",
            invalid(),
            "No account on this server is linked to that sign-in.",
        ),
        (
            "error=oidc_expired",
            invalid(),
            "The sign-in took too long in the browser.",
        ),
        (
            "code=stale",
            invalid(),
            "The sign-in could not be completed.",
        ),
        ("code=late", unauthenticated, "The sign-in was refused."),
    ];
    for (query, redeem, text) in cases {
        let server = potluck_server(cx);
        answer(cx, &server, "GET", "/api/v1/server", acme());
        answer(cx, &server, "POST", REDEEM, redeem);
        let view = open_join(cx, &s, &server.uri());
        checked(cx, &server);
        let start = start_sso(cx, &s, &view);
        let state = param(&start, "state");
        land(&start, format!("{query}&state={state}"));
        wait_for(cx, "the error", |cx| {
            view.read_with(cx, |v, _| v.error().is_some())
        });
        let (phase, err) = view.read_with(cx, |v, _| (v.phase(), v.error().map(str::to_owned)));
        assert_eq!(phase, JoinPhase::Form, "{query}");
        assert_eq!(err.as_deref(), Some(text), "{query}");
        assert_eq!(
            secrets
                .get(&token_key(&server_url(&server), "ada"))
                .unwrap(),
            None
        );
        click(cx, &s, "team-join-cancel");
    }
}

#[gpui_kit::test]
fn cancelling_the_browser_wait_closes_the_port_and_stores_nothing(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = potluck_server(cx);
    answer(cx, &server, "GET", "/api/v1/server", acme());
    recording_opener(cx);
    let (_col, s, secrets) = team_app(cx, |_, _| {});
    let view = open_join(cx, &s, &server.uri());
    checked(cx, &server);
    let start = start_sso(cx, &s, &view);
    let port: u16 = param(&start, "port").parse().unwrap();
    click(cx, &s, "team-join-cancel");
    wait_for(cx, "the port to close", |_| {
        TcpStream::connect(("127.0.0.1", port)).is_err()
    });
    assert_eq!(
        secrets
            .get(&token_key(&server_url(&server), "ada"))
            .unwrap(),
        None
    );
    assert!(!team_file(&s).exists());
}

#[gpui_kit::test]
fn a_server_with_only_single_sign_on_hides_the_password_rows(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = potluck_server(cx);
    let only = info_with(false, Some(json!({"name": "Acme ID"})))
        .set_delay(std::time::Duration::from_millis(400));
    answer(cx, &server, "GET", "/api/v1/server", only);
    let (_col, s, _secrets) = team_app(cx, |_, _| {});
    let view = open_join(cx, &s, &server.uri());
    // Until the server answers, the form still asks for a password.
    assert!(shown(cx, s.win, "team-join-password"));
    wait_for(cx, "the answer", |cx| {
        view.read_with(cx, |v, _| v.sso_name().is_some())
    });
    frame(cx, s.win);
    assert!(!view.read_with(cx, |v, _| v.password_offered()));
    assert!(!shown(cx, s.win, "team-join-password"));
    assert!(!shown(cx, s.win, "team-join-username"));
    assert!(!shown(cx, s.win, "team-join-submit"));
    assert!(shown(cx, s.win, "team-join-sso"));
}

#[gpui_kit::test]
fn an_expired_sign_in_says_so_and_offers_the_provider(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = potluck_server(cx);
    answer(cx, &server, "GET", "/api/v1/server", acme());
    let url = server_url(&server);
    let (_col, s, _secrets) = team_app(cx, |paths, _| {
        let mut account = Account::new(&url, "ada", "Ada Lovelace", "7", false);
        account.expires_at = Some("2000-01-01T00:00:00Z".into());
        TeamStore::update(&paths.team, |t| t.put_account(account)).unwrap();
    });
    let ws = s.ws.clone();
    let uri = server.uri();
    let view = cx
        .update_window(s.win, |_, w, cx| {
            team_join::open_for(&ws, &uri, "ada", w, cx)
        })
        .unwrap();
    frame(cx, s.win);
    assert_eq!(
        view.read_with(cx, |v, _| v.notice().map(str::to_owned))
            .as_deref(),
        Some("Your sign-in expired.")
    );
    assert!(shown(cx, s.win, "team-join-notice"));
    checked(cx, &server);
    frame(cx, s.win);
    assert_eq!(sso_name(cx, &view).as_deref(), Some("Acme ID"));
    assert!(shown(cx, s.win, "team-join-sso"));
}

#[gpui_kit::test]
fn an_answer_for_an_old_address_is_dropped(cx: &mut TestAppContext) {
    http_ready(cx);
    let old = potluck_server(cx);
    let late = acme().set_delay(std::time::Duration::from_millis(400));
    answer(cx, &old, "GET", "/api/v1/server", late);
    let (_col, s, _secrets) = team_app(cx, |_, _| {});
    let view = open_join(cx, &s, &old.uri());
    wait_for(cx, "the old check", |cx| {
        hits_on(cx, &old, "GET", "/api/v1/server") > 0
    });
    let other = potluck_server(cx);
    answer(cx, &other, "GET", "/api/v1/server", info_with(true, None));
    let at = other.uri();
    let v = view.clone();
    cx.update_window(s.win, |_, w, cx| {
        v.update(cx, |v, cx| {
            v.set_server(&at, w, cx);
            v.check_server(w, cx);
        })
    })
    .unwrap();
    checked(cx, &other);
    std::thread::sleep(std::time::Duration::from_millis(500));
    frame(cx, s.win);
    assert_eq!(sso_name(cx, &view), None);
    assert!(!shown(cx, s.win, "team-join-sso"));
}

/// `open_for` as `username` on a server whose kept account is ada's, expiring at `expires_at`.
fn reopen_with_account(
    cx: &mut TestAppContext,
    expires_at: &str,
    username: &str,
) -> (tempfile::TempDir, Started, Entity<JoinView>, MockServer) {
    let server = potluck_server(cx);
    answer(cx, &server, "GET", "/api/v1/server", acme());
    let url = server_url(&server);
    let expires_at = expires_at.to_owned();
    let (col, s, _secrets) = team_app(cx, |paths, _| {
        let mut account = Account::new(&url, "ada", "Ada Lovelace", "7", false);
        account.expires_at = Some(expires_at);
        TeamStore::update(&paths.team, |t| t.put_account(account)).unwrap();
    });
    let (ws, uri, user) = (s.ws.clone(), server.uri(), username.to_owned());
    let view = cx
        .update_window(s.win, |_, w, cx| {
            team_join::open_for(&ws, &uri, &user, w, cx)
        })
        .unwrap();
    frame(cx, s.win);
    (col, s, view, server)
}

#[gpui_kit::test]
fn the_expired_note_is_only_for_that_user_and_a_past_expiry(cx: &mut TestAppContext) {
    http_ready(cx);
    for (expires_at, username) in [
        ("2000-01-01T00:00:00Z", "grace"),
        ("2999-01-01T00:00:00Z", "ada"),
    ] {
        let (_col, s, view, _server) = reopen_with_account(cx, expires_at, username);
        assert_eq!(
            view.read_with(cx, |v, _| v.notice().map(str::to_owned)),
            None
        );
        assert!(!shown(cx, s.win, "team-join-notice"));
    }
}

#[gpui_kit::test]
fn another_address_drops_the_expired_note(cx: &mut TestAppContext) {
    http_ready(cx);
    let (_col, s, view, _server) = reopen_with_account(cx, "2000-01-01T00:00:00Z", "ada");
    assert!(view.read_with(cx, |v, _| v.notice().is_some()));
    let v = view.clone();
    cx.update_window(s.win, |_, w, cx| {
        v.update(cx, |v, cx| v.set_server("https://other.example.com", w, cx))
    })
    .unwrap();
    assert_eq!(
        view.read_with(cx, |v, _| v.notice().map(str::to_owned)),
        None
    );
}

/// The join dialog opened empty, so the address field has focus, with all three fields filled.
fn filled_join(cx: &mut TestAppContext, s: &Started, server: &MockServer) -> Entity<JoinView> {
    let ws = s.ws.clone();
    let view = cx
        .update_window(s.win, |_, w, cx| team_join::open(&ws, w, cx))
        .unwrap();
    frame(cx, s.win);
    let (v, uri) = (view.clone(), server.uri());
    cx.update_window(s.win, |_, w, cx| {
        v.update(cx, |v, cx| {
            v.set_server(&uri, w, cx);
            v.set_username("ada", w, cx);
            v.set_password("hunter2", w, cx);
        })
    })
    .unwrap();
    view
}

#[gpui_kit::test]
fn enter_in_the_address_never_sends_a_password_to_a_server_without_it(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = potluck_server(cx);
    let only = info_with(false, Some(json!({"name": "Acme ID"})));
    answer(cx, &server, "GET", "/api/v1/server", only);
    let session = crate::support::potluck::session(7, false);
    answer(cx, &server, "POST", "/api/v1/session", session);
    let (_col, s, _secrets) = team_app(cx, |_, _| {});
    let view = filled_join(cx, &s, &server);
    platform_press(cx, s.win, "enter");
    wait_for(cx, "the answer", |cx| {
        view.read_with(cx, |v, _| v.sso_name().is_some())
    });
    platform_press(cx, s.win, "enter");
    assert_eq!(hits_on(cx, &server, "POST", "/api/v1/session"), 0);
    assert_eq!(
        view.read_with(cx, |v, _| v.error().map(str::to_owned)),
        None
    );
    assert_eq!(view.read_with(cx, |v, _| v.phase()), JoinPhase::Form);
}

#[gpui_kit::test]
fn enter_in_the_address_signs_in_once_the_server_takes_a_password(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = potluck_server(cx);
    answer(cx, &server, "GET", "/api/v1/server", acme());
    let session = crate::support::potluck::session(7, false);
    answer(cx, &server, "POST", "/api/v1/session", session);
    let (_col, s, _secrets) = team_app(cx, |_, _| {});
    let view = filled_join(cx, &s, &server);
    platform_press(cx, s.win, "enter");
    assert_eq!(hits_on(cx, &server, "POST", "/api/v1/session"), 0);
    wait_for(cx, "the answer", |cx| {
        view.read_with(cx, |v, _| v.sso_name().is_some())
    });
    platform_press(cx, s.win, "enter");
    wait_for(cx, "the sign-in", |cx| {
        hits_on(cx, &server, "POST", "/api/v1/session") == 1
    });
}
