//! Sending an authorization-code request signs in through the dialog.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::oauth2_signin::{self, Showing};
use apinata_core::{ApprovalInbox, ExecError, OAuth2Error, Outcome, RunError, TrustStore};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{App, AppContext as _, TestAppContext};
use std::sync::{Arc, Mutex};
use support::*;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "tok-signin-31c7";

pub fn provider(cx: &mut TestAppContext) -> MockServer {
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(MockServer::builder().start());
    let body = format!(r#"{{"access_token":"{TOKEN}","token_type":"Bearer","expires_in":3600}}"#);
    handle.block_on(
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=authorization_code"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&server),
    );
    handle.block_on(
        Mock::given(method("GET"))
            .and(path("/r"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server),
    );
    server
}
pub fn collection(uri: &str, callback: &str) -> tempfile::TempDir {
    let file = format!(
        "{}  auth:\n    type: oauth2\n    flow: authorization_code\n    authorizationUrl: \"{uri}/authorize\"\n    accessTokenUrl: \"{uri}/token\"\n{callback}    credentials:\n      clientId: app\n    pkce: {{}}\n    tokenConfig:\n      id: main\n      placement:\n        header: Bearer\n",
        request("a", &format!("{uri}/r"))
    );
    write_tree(&[("opencollection.yml", ROOT), ("a.yml", &file)])
}
fn param(url: &str, name: &str) -> String {
    url::Url::parse(url)
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default()
}
/// Plays the browser: GET the redirect URI with a code and the prompt's state.
fn browse(url: &str) {
    use std::io::{Read, Write};
    let mut callback = url::Url::parse(&param(url, "redirect_uri")).unwrap();
    callback
        .query_pairs_mut()
        .append_pair("code", "c0de")
        .append_pair("state", &param(url, "state"));
    let addr = (
        callback.host_str().unwrap().to_owned(),
        callback.port().unwrap(),
    );
    let target = format!("{}?{}", callback.path(), callback.query().unwrap());
    std::thread::spawn(move || {
        let mut s = std::net::TcpStream::connect(addr).unwrap();
        write!(
            s,
            "GET {target} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
    });
}
pub fn opener(cx: &mut TestAppContext, play: bool) -> Arc<Mutex<Vec<String>>> {
    let opened = Arc::new(Mutex::new(Vec::new()));
    let seen = opened.clone();
    cx.update(|cx| {
        oauth2_signin::set_opener(
            Arc::new(move |url: &str, _: &mut App| {
                seen.lock().unwrap().push(url.to_owned());
                if play {
                    browse(url);
                }
            }),
            cx,
        )
    });
    opened
}
pub fn showing(cx: &mut TestAppContext) -> Option<Showing> {
    cx.update(|cx| oauth2_signin::showing(cx))
}
fn dialog_open(cx: &mut TestAppContext, s: &Started) -> bool {
    cx.update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap()
}
fn click(cx: &mut TestAppContext, s: &Started, id: &'static str) {
    cx.update_window(s.win, |_, w, cx| w.click(id, cx)).unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
}
fn token_posts(cx: &mut TestAppContext, server: &MockServer) -> usize {
    served(cx, server).iter().filter(|p| *p == "/token").count()
}
pub fn start_send(
    cx: &mut TestAppContext,
    s: &Started,
) -> gpui_kit::Entity<apinata_app::send_model::SendModel> {
    let model = sender(cx, &s.ws);
    let root = s.root.clone();
    assert!(model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx)));
    model
}

#[gpui_kit::test]
fn a_send_signs_in_through_the_loopback_and_sends_the_token(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = provider(cx);
    let col = collection(&server.uri(), "");
    let s = started(cx, &col);
    let opened = opener(cx, true);
    let model = start_send(cx, &s);
    let result = finished(cx, &model);
    assert!(result.error.is_none(), "{:?}", result.error);
    let opened = opened.lock().unwrap().clone();
    assert_eq!(opened.len(), 1);
    assert!(
        opened[0].starts_with(&format!("{}/authorize?response_type=code", server.uri())),
        "{}",
        opened[0]
    );
    assert_eq!(token_posts(cx, &server), 1);
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let received = handle
        .block_on(server.received_requests())
        .unwrap_or_default();
    let sent = received
        .iter()
        .find(|r| r.url.path() == "/r")
        .expect("the request");
    let expected = format!("Bearer {TOKEN}");
    assert_eq!(
        sent.headers
            .get("authorization")
            .and_then(|v| v.to_str().ok()),
        Some(expected.as_str())
    );
    wait_real(cx, |cx| showing(cx).is_none());
    assert!(!dialog_open(cx, &s));
}

#[gpui_kit::test]
fn cancel_in_the_dialog_cancels_the_send(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = provider(cx);
    let col = collection(&server.uri(), "");
    let s = started(cx, &col);
    let _opened = opener(cx, false);
    let model = start_send(cx, &s);
    wait_real(cx, |cx| showing(cx).is_some());
    let shown = showing(cx).unwrap();
    assert!(!shown.paste);
    assert_eq!(
        shown.host,
        url::Url::parse(&server.uri()).unwrap().host_str().unwrap()
    );
    frame(cx, s.win);
    click(cx, &s, "oauth2-signin-cancel");
    let result = finished(cx, &model);
    assert_eq!(result.outcome, Outcome::Cancelled);
    assert!(
        matches!(result.error, Some(RunError::Exec(ExecError::Cancelled))),
        "{:?}",
        result.error
    );
    assert_eq!(token_posts(cx, &server), 0);
    wait_real(cx, |cx| showing(cx).is_none());
    assert!(!dialog_open(cx, &s));
}

#[gpui_kit::test]
fn enter_on_the_focused_cancel_button_cancels_the_send(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = provider(cx);
    let col = collection(&server.uri(), "");
    let s = started(cx, &col);
    let _opened = opener(cx, false);
    let model = start_send(cx, &s);
    wait_real(cx, |cx| showing(cx).is_some());
    frame(cx, s.win);
    enter_on(cx, s.win, "oauth2-signin-cancel");
    let result = finished(cx, &model);
    assert!(
        matches!(result.error, Some(RunError::Exec(ExecError::Cancelled))),
        "{:?}",
        result.error
    );
    wait_real(cx, |cx| showing(cx).is_none());
    assert!(!dialog_open(cx, &s));
}

#[gpui_kit::test]
fn cancelling_the_dialog_cancels_every_send_queued_on_it(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = provider(cx);
    let col = collection(&server.uri(), "");
    let s = started(cx, &col);
    let opened = opener(cx, false);
    let models: Vec<_> = (0..3).map(|_| start_send(cx, &s)).collect();
    wait_real(cx, |cx| showing(cx).is_some());
    frame(cx, s.win);
    click(cx, &s, "oauth2-signin-cancel");
    for model in &models {
        assert_eq!(finished(cx, model).outcome, Outcome::Cancelled);
    }
    wait_real(cx, |cx| showing(cx).is_none());
    cx.run_until_parked();
    assert!(
        showing(cx).is_none(),
        "a queued send opened another sign-in"
    );
    assert_eq!(opened.lock().unwrap().len(), 1);
    assert_eq!(token_posts(cx, &server), 0);
    assert!(!dialog_open(cx, &s));
}

#[gpui_kit::test]
fn a_provider_error_closes_the_dialog(cx: &mut TestAppContext) {
    http_ready(cx);
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(MockServer::builder().start());
    handle.block_on(
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(
                ResponseTemplate::new(400).set_body_string(r#"{"error":"invalid_grant"}"#),
            )
            .mount(&server),
    );
    let col = collection(&server.uri(), "");
    let s = started(cx, &col);
    let _opened = opener(cx, true);
    let model = start_send(cx, &s);
    let result = finished(cx, &model);
    assert!(
        matches!(
            result.error,
            Some(RunError::OAuth2(OAuth2Error::Provider { status: 400, .. }))
        ),
        "{:?}",
        result.error
    );
    wait_real(cx, |cx| showing(cx).is_none());
    assert!(!dialog_open(cx, &s));
}

#[gpui_kit::test]
fn a_non_loopback_callback_asks_for_the_pasted_address(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = provider(cx);
    let col = collection(
        &server.uri(),
        "    callbackUrl: https://app.example.test/cb\n",
    );
    let s = started(cx, &col);
    let _opened = opener(cx, false);
    let model = start_send(cx, &s);
    wait_real(cx, |cx| showing(cx).is_some());
    assert!(showing(cx).unwrap().paste);
    frame(cx, s.win);
    replace_input(
        cx,
        s.win,
        None,
        "oauth2-signin-paste",
        "https://app.example.test/cb?nothing=1",
    );
    click(cx, &s, "oauth2-signin-continue");
    let shown = showing(cx).expect("the dialog stays open");
    assert!(
        shown
            .error
            .is_some_and(|e| e.contains("no authorization code"))
    );
    let state = param(&shown.url, "state");
    replace_input(
        cx,
        s.win,
        None,
        "oauth2-signin-paste",
        &format!("https://app.example.test/cb?code=c0de&state={state}"),
    );
    click(cx, &s, "oauth2-signin-continue");
    let result = finished(cx, &model);
    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(token_posts(cx, &server), 1);
}

#[gpui_kit::test]
fn without_an_opener_the_dialog_still_offers_the_url(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = provider(cx);
    let col = collection(&server.uri(), "");
    let s = started(cx, &col);
    let model = start_send(cx, &s);
    wait_real(cx, |cx| showing(cx).is_some());
    frame(cx, s.win);
    click(cx, &s, "oauth2-signin-copy");
    let url = showing(cx).unwrap().url;
    assert!(url.starts_with(&format!("{}/authorize?", server.uri())));
    assert_eq!(cx.read_from_clipboard().and_then(|c| c.text()), Some(url));
    click(cx, &s, "oauth2-signin-cancel");
    finished(cx, &model);
}

#[gpui_kit::test]
fn an_approval_waits_until_the_sign_in_dialog_closes(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = provider(cx);
    let col = collection(&server.uri(), "");
    let s = started(cx, &col);
    let opened = opener(cx, false);
    let model = start_send(cx, &s);
    wait_real(cx, |cx| showing(cx).is_some());
    frame(cx, s.win);
    // The MCP server files an approval while the sign-in dialog is up.
    let store = TrustStore::empty(s.files.path().join("trusted-hosts.json"));
    let inbox = ApprovalInbox::new(s.files.path().join("approvals"));
    inbox
        .request(&store, &s.root, "api.example.com", "a.yml")
        .unwrap();
    let approvals = cx.update(|cx| cx.global::<AppState>().approvals.clone());
    approvals.update(cx, |a, cx| a.refresh(cx));
    cx.run_until_parked();
    frame(cx, s.win);
    assert!(s.shell.read_with(cx, |v, _| v.approval_shown().is_none()));
    let url = opened.lock().unwrap()[0].clone();
    browse(&url);
    let result = finished(cx, &model);
    assert!(result.error.is_none(), "{:?}", result.error);
    wait_real(cx, |cx| showing(cx).is_none());
    wait_real(cx, |cx| {
        s.shell.read_with(cx, |v, _| v.approval_shown().is_some())
    });
    assert!(present(cx, s.win, "approval-host"));
    assert!(!present(cx, s.win, "oauth2-signin"));
}

fn show_approval(cx: &mut TestAppContext, s: &Started) {
    let store = TrustStore::empty(s.files.path().join("trusted-hosts.json"));
    let inbox = ApprovalInbox::new(s.files.path().join("approvals"));
    inbox
        .request(&store, &s.root, "api.example.com", "a.yml")
        .unwrap();
    let approvals = cx.update(|cx| cx.global::<AppState>().approvals.clone());
    approvals.update(cx, |a, cx| a.refresh(cx));
    wait_real(cx, |cx| {
        s.shell.read_with(cx, |v, _| v.approval_shown().is_some())
    });
    frame(cx, s.win);
    assert!(present(cx, s.win, "approval-host"));
}

#[gpui_kit::test]
fn a_sign_in_over_an_open_approval_leaves_the_approval(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = provider(cx);
    let col = collection(&server.uri(), "");
    let s = started(cx, &col);
    let opened = opener(cx, false);
    show_approval(cx, &s);
    let model = start_send(cx, &s);
    wait_real(cx, |cx| showing(cx).is_some());
    frame(cx, s.win);
    assert!(present(cx, s.win, "oauth2-signin"));
    let url = opened.lock().unwrap()[0].clone();
    browse(&url);
    let result = finished(cx, &model);
    assert!(result.error.is_none(), "{:?}", result.error);
    wait_real(cx, |cx| showing(cx).is_none());
    frame(cx, s.win);
    assert!(!present(cx, s.win, "oauth2-signin"));
    assert!(present(cx, s.win, "approval-host"));
    assert!(s.shell.read_with(cx, |v, _| v.approval_shown().is_some()));
}

#[gpui_kit::test]
fn cancelling_a_sign_in_over_an_approval_leaves_the_approval(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = provider(cx);
    let col = collection(&server.uri(), "");
    let s = started(cx, &col);
    let _opened = opener(cx, false);
    show_approval(cx, &s);
    let model = start_send(cx, &s);
    wait_real(cx, |cx| showing(cx).is_some());
    frame(cx, s.win);
    click(cx, &s, "oauth2-signin-cancel");
    assert_eq!(finished(cx, &model).outcome, Outcome::Cancelled);
    wait_real(cx, |cx| showing(cx).is_none());
    frame(cx, s.win);
    assert!(!present(cx, s.win, "oauth2-signin"));
    assert!(present(cx, s.win, "approval-host"));
}
