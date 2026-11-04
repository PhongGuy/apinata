//! The Copy as curl palette entry and sidebar menu item.

use crate::support;

use apinata_app::sidebar::menu::MenuEntry;
use apinata_app::sidebar::model::NodeKind;
use gpui_kit::component::WindowExt as _;
use gpui_kit::{AppContext as _, ClipboardItem, Focusable as _, TestAppContext};
use support::*;
use wiremock::matchers::{method, path};

fn focus_shell(cx: &mut TestAppContext, s: &Started) {
    let shell = s.shell.clone();
    cx.update_window(s.win, |_, w, cx| {
        shell.read(cx).focus_handle(cx).focus(w, cx)
    })
    .unwrap();
}
fn palette(cx: &mut TestAppContext, s: &Started, text: &str) {
    focus_shell(cx, s);
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        use gpui_kit::test::TestWindowExt as _;
        w.input(text, cx)
    })
    .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "enter");
    cx.run_until_parked();
}
fn clipboard(cx: &mut TestAppContext) -> Option<String> {
    cx.read_from_clipboard().and_then(|c| c.text())
}
fn toasts(cx: &mut TestAppContext, s: &Started) -> usize {
    cx.update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}
fn with_request(file: &str, extra: &[(&str, &str)]) -> tempfile::TempDir {
    let mut files = vec![("opencollection.yml", ROOT), ("a.yml", file)];
    files.extend_from_slice(extra);
    write_tree(&files)
}

#[gpui_kit::test]
fn the_palette_copies_the_active_request(cx: &mut TestAppContext) {
    let col = with_request(&request("a", "http://x.test/ok"), &[]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    palette(cx, &s, "copy as curl");
    assert_eq!(
        clipboard(cx).as_deref(),
        Some("curl --request 'GET' \\\n  --url 'http://x.test/ok'")
    );
}

#[gpui_kit::test]
fn the_sidebar_menu_copies_that_request(cx: &mut TestAppContext) {
    let col = with_request(&request("a", "http://x.test/m"), &[]);
    let s = started(cx, &col);
    choose_menu(cx, s.win, "a.yml", NodeKind::Request, MenuEntry::CopyAsCurl);
    assert_eq!(
        clipboard(cx).as_deref(),
        Some("curl --request 'GET' \\\n  --url 'http://x.test/m'")
    );
}

#[gpui_kit::test]
fn secrets_wait_for_confirmation(cx: &mut TestAppContext) {
    let file = format!(
        "{}  headers:\n    - name: Authorization\n      value: \"Bearer {{{{process.env.APINATA_COPY_CURL_TOKEN}}}}\"\n",
        request("a", "http://x.test/s")
    );
    let col = with_request(&file, &[(".env", "APINATA_COPY_CURL_TOKEN=s3cret\n")]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    cx.write_to_clipboard(ClipboardItem::new_string("before".into()));
    palette(cx, &s, "copy as curl");
    assert!(confirm_open(cx));
    answer_confirm(cx, "confirm-cancel");
    assert_eq!(clipboard(cx).as_deref(), Some("before"));
    palette(cx, &s, "copy as curl");
    answer_confirm(cx, "confirm-copy");
    assert!(clipboard(cx).is_some_and(|t| t.contains("Authorization: Bearer s3cret")));
}

#[gpui_kit::test]
fn an_undefined_variable_copies_nothing_and_says_why(cx: &mut TestAppContext) {
    let col = with_request(&request("a", "http://x.test/{{nope}}"), &[]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    cx.write_to_clipboard(ClipboardItem::new_string("before".into()));
    let shown = toasts(cx, &s);
    palette(cx, &s, "copy as curl");
    assert!(!confirm_open(cx));
    assert_eq!(clipboard(cx).as_deref(), Some("before"));
    assert_eq!(toasts(cx, &s), shown + 1);
}

#[gpui_kit::test]
fn an_oauth2_request_without_a_token_is_not_copied_and_fetches_nothing(cx: &mut TestAppContext) {
    http_ready(cx);
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(wiremock::MockServer::start());
    let file = format!(
        "{}  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{}/token\"\n    credentials:\n      clientId: app\n",
        request("a", "http://x.test/o"),
        server.uri()
    );
    let col = with_request(&file, &[]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    cx.write_to_clipboard(ClipboardItem::new_string("before".into()));
    palette(cx, &s, "copy as curl");
    wait_real(cx, |cx| toasts(cx, &s) > 0);
    assert_eq!(clipboard(cx).as_deref(), Some("before"));
    assert_eq!(toasts(cx, &s), 1);
    let received = handle
        .block_on(server.received_requests())
        .unwrap_or_default();
    assert!(
        received.is_empty(),
        "Copy as curl contacted the token endpoint"
    );
}

#[gpui_kit::test]
fn a_cached_token_is_copied_without_a_token_call(cx: &mut TestAppContext) {
    http_ready(cx);
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(wiremock::MockServer::builder().start());
    let body = r#"{"access_token":"tok-copy-9a1e","token_type":"Bearer","expires_in":3600}"#;
    handle.block_on(
        wiremock::Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(body))
            .mount(&server),
    );
    handle.block_on(
        wiremock::Mock::given(method("GET"))
            .and(path("/o"))
            .respond_with(wiremock::ResponseTemplate::new(200))
            .mount(&server),
    );
    let file = format!(
        "{}  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{}/token\"\n    credentials:\n      clientId: app\n    tokenConfig:\n      placement:\n        header: Bearer\n",
        request("a", &format!("{}/o", server.uri())),
        server.uri()
    );
    let col = with_request(&file, &[]);
    let s = started(cx, &col);
    let model = sender(cx, &s.ws);
    let root = s.root.clone();
    assert!(model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx)));
    finished(cx, &model);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    palette(cx, &s, "copy as curl");
    answer_confirm(cx, "confirm-copy");
    assert!(
        clipboard(cx).is_some_and(|t| t.contains("Authorization: Bearer tok-copy-9a1e")),
        "{:?}",
        clipboard(cx)
    );
    let posts = handle
        .block_on(server.received_requests())
        .unwrap_or_default()
        .iter()
        .filter(|r| r.method.as_str() == "POST")
        .count();
    assert_eq!(posts, 1, "Copy as curl fetched a token");
}
