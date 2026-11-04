//! The token panel under the OAuth 2.0 editor.

use crate::support;

use apinata_app::config_editor::{ConfigEditor, ConfigTarget};
use apinata_app::request_editor::{RequestEditor, Section};
use apinata_app::token_panel::{TokenPanel, status_line};
use apinata_app::workspace::Workspace;
use apinata_core::TokenStatus;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use std::path::Path;
use std::time::{Duration, SystemTime};
use support::*;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn provider(cx: &mut TestAppContext, token: &str, refresh: Option<ResponseTemplate>) -> MockServer {
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(MockServer::builder().start());
    handle.block_on(
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=client_credentials"))
            .respond_with(ResponseTemplate::new(200).set_body_string(token.to_owned()))
            .mount(&server),
    );
    if let Some(refresh) = refresh {
        handle.block_on(
            Mock::given(method("POST"))
                .and(path("/token"))
                .and(body_string_contains("grant_type=refresh_token"))
                .respond_with(refresh)
                .mount(&server),
        );
    }
    server
}
fn cc(uri: &str) -> String {
    format!(
        "    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{uri}/token\"\n    credentials:\n      clientId: app\n    tokenConfig:\n      id: main\n"
    )
}
fn request_panel(cx: &mut TestAppContext, view: &Entity<RequestEditor>) -> Entity<TokenPanel> {
    view.read_with(cx, |v, cx| {
        v.auth().read(cx).oauth2().read(cx).token_panel().cloned()
    })
    .expect("a token panel")
}
fn status(cx: &mut TestAppContext, p: &Entity<TokenPanel>) -> String {
    p.read_with(cx, |p, _| p.status_text())
}
fn settle_panel(cx: &mut TestAppContext, p: &Entity<TokenPanel>) {
    wait_real(cx, |cx| !p.read_with(cx, |p, _| p.is_busy()));
}
fn posts(cx: &mut TestAppContext, server: &MockServer, grant: &str) -> usize {
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    handle
        .block_on(server.received_requests())
        .unwrap_or_default()
        .iter()
        .filter(|r| String::from_utf8_lossy(&r.body).contains(&format!("grant_type={grant}")))
        .count()
}

#[test]
fn status_lines_describe_the_token() {
    let now = SystemTime::now();
    let at = |secs: u64| Some(now + Duration::from_secs(secs));
    let line = |s: TokenStatus| status_line(Some(&s), now);
    assert_eq!(status_line(None, now), "No token");
    assert_eq!(
        line(TokenStatus::new(now, at(54 * 60 + 20), false, None)),
        "Valid, expires in 54 min"
    );
    assert_eq!(
        line(TokenStatus::new(
            now,
            at(2 * 3600 + 5 * 60 + 1),
            false,
            None
        )),
        "Valid, expires in 2 h 5 min"
    );
    assert_eq!(
        line(TokenStatus::new(now, at(40), false, None)),
        "Valid, expires in less than a minute"
    );
    assert_eq!(
        line(TokenStatus::new(now, None, false, None)),
        "Valid, no expiry"
    );
    assert_eq!(
        line(TokenStatus::new(
            now,
            Some(now - Duration::from_secs(1)),
            false,
            None
        )),
        "Expired"
    );
    assert_eq!(
        line(TokenStatus::new(now, None, true, None)),
        "Valid, no expiry, refresh token"
    );
    assert_eq!(
        line(TokenStatus::new(now, Some(now), true, None)),
        "Expired, refresh token"
    );
}

#[gpui_kit::test]
fn get_token_fetches_and_clear_forgets(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = provider(
        cx,
        r#"{"access_token":"t1","expires_in":3600,"scope":"read write"}"#,
        None,
    );
    let file = format!(
        "{}  auth:\n{}",
        request("a", "http://x.test/r"),
        cc(&server.uri())
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &file)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, view) = editor_for(cx, &ws, &root, "a.yml", false);
    let panel = request_panel(cx, &view);
    settle_panel(cx, &panel);
    assert_eq!(status(cx, &panel), "No token");
    assert_eq!(
        panel.read_with(cx, |p, _| p.notice()).as_deref(),
        Some("tokens are kept in memory only")
    );

    cx.update_window(win, |_, w, cx| {
        w.click(("section", Section::Auth as usize), cx);
        w.render_frame(cx)
    })
    .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("oauth2-get-token", cx))
        .unwrap();
    wait_real(cx, |cx| status(cx, &panel).starts_with("Valid"));
    assert_eq!(status(cx, &panel), "Valid, expires in 59 min");
    assert_eq!(
        panel.read_with(cx, |p, _| p.scope_text()).as_deref(),
        Some("Scopes granted: read write")
    );
    assert_eq!(posts(cx, &server, "client_credentials"), 1);

    panel.update(cx, |p, cx| p.clear(cx));
    wait_real(cx, |cx| status(cx, &panel) == "No token");
}

#[gpui_kit::test]
fn the_buttons_stay_in_view_after_a_token_appears(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = provider(
        cx,
        r#"{"access_token":"t1","refresh_token":"r1","expires_in":3600,"scope":"read write"}"#,
        None,
    );
    let file = format!(
        "{}  auth:\n{}",
        request("a", "http://x.test/r"),
        cc(&server.uri())
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &file)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, view) = editor_for(cx, &ws, &root, "a.yml", false);
    let panel = request_panel(cx, &view);
    cx.update_window(win, |_, w, cx| {
        w.click(("section", Section::Auth as usize), cx);
        w.render_frame(cx)
    })
    .unwrap();
    panel.update(cx, |p, cx| p.get_token(cx));
    wait_real(cx, |cx| status(cx, &panel).ends_with("refresh token"));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let body = w.find("section-body").bounds();
        for id in [
            "oauth2-get-token",
            "oauth2-refresh-token",
            "oauth2-clear-token",
        ] {
            let b = w.find(id).bounds();
            assert!(
                b.top() >= body.top() && b.bottom() <= body.bottom(),
                "{id} at {b:?} is outside the visible section body {body:?}"
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn refresh_appears_with_a_refresh_token_and_posts_one(cx: &mut TestAppContext) {
    http_ready(cx);
    let refreshed =
        ResponseTemplate::new(200).set_body_string(r#"{"access_token":"t2","expires_in":120}"#);
    let server = provider(
        cx,
        r#"{"access_token":"t1","refresh_token":"r1","expires_in":3600}"#,
        Some(refreshed),
    );
    let file = format!(
        "{}  auth:\n{}",
        request("a", "http://x.test/r"),
        cc(&server.uri())
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &file)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (_win, view) = editor_for(cx, &ws, &root, "a.yml", false);
    let panel = request_panel(cx, &view);
    panel.update(cx, |p, cx| p.get_token(cx));
    wait_real(cx, |cx| {
        status(cx, &panel) == "Valid, expires in 59 min, refresh token"
    });
    panel.update(cx, |p, cx| p.refresh_token(cx));
    wait_real(cx, |cx| {
        status(cx, &panel) == "Valid, expires in 1 min, refresh token"
    });
    assert_eq!(posts(cx, &server, "refresh_token"), 1);
}

#[gpui_kit::test]
fn a_provider_error_shows_as_the_notice(cx: &mut TestAppContext) {
    http_ready(cx);
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(MockServer::builder().start());
    handle.block_on(
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(
                ResponseTemplate::new(401).set_body_string(r#"{"error":"invalid_client"}"#),
            )
            .mount(&server),
    );
    let file = format!(
        "{}  auth:\n{}",
        request("a", "http://x.test/r"),
        cc(&server.uri())
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &file)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (_win, view) = editor_for(cx, &ws, &root, "a.yml", false);
    let panel = request_panel(cx, &view);
    panel.update(cx, |p, cx| p.get_token(cx));
    settle_panel(cx, &panel);
    assert_eq!(
        panel.read_with(cx, |p, _| p.notice()).as_deref(),
        Some("the token endpoint answered 401: invalid_client")
    );
    assert_eq!(status(cx, &panel), "No token");
}

fn config_editor(
    cx: &mut TestAppContext,
    ws: &Entity<Workspace>,
    root: &Path,
    target: ConfigTarget,
) -> (AnyWindowHandle, Entity<ConfigEditor>) {
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let (ws, root) = (ws.clone(), root.to_path_buf());
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ConfigEditor::new(ws, root, target, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    (win, view)
}

#[gpui_kit::test]
fn a_folder_panel_gets_the_folder_token(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = provider(cx, r#"{"access_token":"tf"}"#, None);
    let folder = format!(
        "info:\n  name: F\n  type: folder\n  seq: 1\nrequest:\n  auth:\n{}",
        cc(&server.uri())
    );
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder),
        ("f/a.yml", &request("a", "http://x.test/r")),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (_win, view) = config_editor(cx, &ws, &root, ConfigTarget::Folder(ip("f")));
    let panel = view
        .read_with(cx, |v, cx| {
            v.auth().read(cx).oauth2().read(cx).token_panel().cloned()
        })
        .expect("a token panel");
    panel.update(cx, |p, cx| p.get_token(cx));
    wait_real(cx, |cx| status(cx, &panel) == "Valid, no expiry");
    assert_eq!(posts(cx, &server, "client_credentials"), 1);
}

#[gpui_kit::test]
fn get_token_signs_in_for_the_code_flow(cx: &mut TestAppContext) {
    use apinata_app::oauth2_signin;
    http_ready(cx);
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(MockServer::builder().start());
    handle.block_on(
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=authorization_code"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"{"access_token":"tc","expires_in":600}"#),
            )
            .mount(&server),
    );
    let uri = server.uri();
    let file = format!(
        "{}  auth:\n    type: oauth2\n    flow: authorization_code\n    authorizationUrl: \"{uri}/authorize\"\n    accessTokenUrl: \"{uri}/token\"\n    credentials:\n      clientId: app\n",
        request("a", "http://x.test/r")
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &file)]);
    let s = started(cx, &col);
    let opened = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen = opened.clone();
    cx.update(|cx| {
        oauth2_signin::set_opener(
            std::sync::Arc::new(move |url: &str, _: &mut gpui_kit::App| {
                seen.lock().unwrap().push(url.to_owned())
            }),
            cx,
        )
    });
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let view = s
        .shell
        .read_with(cx, |sh, cx| sh.tab_area().read(cx).editor(tab).cloned())
        .expect("an editor");
    let panel = request_panel(cx, &view);
    panel.update(cx, |p, cx| p.get_token(cx));
    wait_real(cx, |_| !opened.lock().unwrap().is_empty());
    // A refresh while the browser is out must not abort the sign-in.
    panel.update(cx, |p, cx| p.refresh(cx));
    cx.run_until_parked();
    assert!(panel.read_with(cx, |p, _| p.is_busy()));
    assert!(cx.update(|cx| oauth2_signin::showing(cx)).is_some());
    let url = url::Url::parse(&opened.lock().unwrap()[0]).unwrap();
    let get = |k: &str| {
        url.query_pairs()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.into_owned())
            .unwrap()
    };
    let mut cb = url::Url::parse(&get("redirect_uri")).unwrap();
    cb.query_pairs_mut()
        .append_pair("code", "c0de")
        .append_pair("state", &get("state"));
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        let mut s = std::net::TcpStream::connect(("127.0.0.1", cb.port().unwrap())).unwrap();
        write!(
            s,
            "GET {}?{} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
            cb.path(),
            cb.query().unwrap()
        )
        .unwrap();
        let _ = s.read_to_string(&mut String::new());
    });
    wait_real(cx, |cx| status(cx, &panel) == "Valid, expires in 9 min");
    assert!(
        panel.read_with(cx, |p, _| p.notice()).is_none(),
        "the started app keeps tokens encrypted"
    );
}

#[gpui_kit::test]
fn an_auth_edit_reads_the_status_again(cx: &mut TestAppContext) {
    use apinata_app::oauth2_editor::OAuth2Toggle;
    http_ready(cx);
    let server = provider(cx, r#"{"access_token":"t1"}"#, None);
    let file = format!(
        "{}  auth:\n{}",
        request("a", "http://x.test/r"),
        cc(&server.uri())
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &file)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, view) = editor_for(cx, &ws, &root, "a.yml", false);
    let panel = request_panel(cx, &view);
    settle_panel(cx, &panel);
    let oauth2 = view.read_with(cx, |v, cx| v.auth().read(cx).oauth2().clone());
    cx.update_window(win, |_, w, cx| {
        oauth2.update(cx, |o, cx| {
            o.set_toggle(OAuth2Toggle::AutoFetch, false, w, cx)
        })
    })
    .unwrap();
    assert!(
        panel.read_with(cx, |p, _| p.is_busy()),
        "an auth edit starts a read"
    );
    settle_panel(cx, &panel);
}

fn basic_auth_file() -> String {
    format!(
        "{}  auth:\n    type: basic\n    username: u\n    password: p\n",
        request("a", "http://x.test/r")
    )
}

#[gpui_kit::test]
fn a_non_oauth_auth_is_never_read(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &basic_auth_file())]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (_win, view) = editor_for(cx, &ws, &root, "a.yml", false);
    let panel = request_panel(cx, &view);
    cx.executor().advance_clock(Duration::from_secs(31));
    cx.run_until_parked();
    assert_eq!(panel.read_with(cx, |p, _| p.reads_started()), 0);
}

#[gpui_kit::test]
fn an_oauth_auth_is_read_again_on_the_timer(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = provider(cx, r#"{"access_token":"t1"}"#, None);
    let file = format!(
        "{}  auth:\n{}",
        request("a", "http://x.test/r"),
        cc(&server.uri())
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &file)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (_win, view) = editor_for(cx, &ws, &root, "a.yml", false);
    let panel = request_panel(cx, &view);
    settle_panel(cx, &panel);
    let before = panel.read_with(cx, |p, _| p.reads_started());
    assert!(before >= 1, "the opened OAuth 2.0 auth is read once loaded");
    cx.executor().advance_clock(Duration::from_secs(16));
    cx.run_until_parked();
    assert!(panel.read_with(cx, |p, _| p.reads_started()) > before);
}

#[gpui_kit::test]
fn an_action_that_cannot_run_says_why(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &basic_auth_file())]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (_win, view) = editor_for(cx, &ws, &root, "a.yml", false);
    let panel = request_panel(cx, &view);
    panel.update(cx, |p, cx| p.get_token(cx));
    cx.run_until_parked();
    assert!(!panel.read_with(cx, |p, _| p.is_busy()));
    assert_eq!(
        panel.read_with(cx, |p, _| p.notice()).as_deref(),
        Some("no OAuth 2.0 auth to act on")
    );
}

#[gpui_kit::test]
fn a_successful_send_or_fetch_clears_the_last_error(cx: &mut TestAppContext) {
    http_ready(cx);
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(MockServer::builder().start());
    let refused = || {
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(
                ResponseTemplate::new(401).set_body_string(r#"{"error":"invalid_client"}"#),
            )
            .up_to_n_times(1)
    };
    handle.block_on(refused().mount(&server));
    handle.block_on(
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"access_token":"t1"}"#))
            .mount(&server),
    );
    handle.block_on(
        Mock::given(method("GET"))
            .and(path("/r"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server),
    );
    let uri = server.uri();
    let file = format!("{}  auth:\n{}", request("a", &format!("{uri}/r")), cc(&uri));
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &file)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (_win, view) = editor_for(cx, &ws, &root, "a.yml", false);
    let panel = request_panel(cx, &view);
    let failed = Some("the token endpoint answered 401: invalid_client");
    let memory = Some("tokens are kept in memory only");

    panel.update(cx, |p, cx| p.get_token(cx));
    settle_panel(cx, &panel);
    assert_eq!(panel.read_with(cx, |p, _| p.notice()).as_deref(), failed);

    // A send fetches the token itself and succeeds: the old error goes.
    let model = view.read_with(cx, |v, _| v.send_model().clone());
    assert!(model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx)));
    let result = finished(cx, &model);
    assert!(result.error.is_none(), "{:?}", result.error);
    wait_real(cx, |cx| {
        panel.read_with(cx, |p, _| p.notice()).as_deref() == memory
    });
    // The refresh clears the error before its status read lands, so wait for the read.
    settle_panel(cx, &panel);
    assert!(status(cx, &panel).starts_with("Valid"));

    // A later failed fetch shows again, and a successful one clears it.
    handle.block_on(server.reset());
    handle.block_on(refused().mount(&server));
    handle.block_on(
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"access_token":"t2"}"#))
            .mount(&server),
    );
    panel.update(cx, |p, cx| p.clear(cx));
    settle_panel(cx, &panel);
    panel.update(cx, |p, cx| p.get_token(cx));
    settle_panel(cx, &panel);
    assert_eq!(panel.read_with(cx, |p, _| p.notice()).as_deref(), failed);
    panel.update(cx, |p, cx| p.get_token(cx));
    settle_panel(cx, &panel);
    assert_eq!(panel.read_with(cx, |p, _| p.notice()).as_deref(), memory);
}

// The label of the element `id` in the panel's window, `None` when it is not drawn.
fn drawn_label(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> Option<String> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

// A failed Get token reads with the editor's label and is drawn as an error of its own,
// apart from the informational notice line.
#[gpui_kit::test]
fn an_action_error_names_the_field_and_is_drawn_as_an_error(cx: &mut TestAppContext) {
    http_ready(cx);
    let auth = "    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"\"\n    credentials:\n      clientId: app\n    tokenConfig:\n      id: main\n";
    let file = format!("{}  auth:\n{auth}", request("a", "http://x.test/r"));
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &file)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, view) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.update_window(win, |_, w, cx| {
        w.click(("section", Section::Auth as usize), cx);
        w.render_frame(cx)
    })
    .unwrap();
    let panel = request_panel(cx, &view);
    panel.update(cx, |p, cx| p.get_token(cx));
    settle_panel(cx, &panel);
    let error = "Access token URL is empty";
    assert_eq!(
        panel.read_with(cx, |p, _| p.notice()).as_deref(),
        Some(error)
    );
    assert_eq!(
        drawn_label(cx, win, "oauth2-token-error").as_deref(),
        Some(error)
    );
    assert_ne!(
        drawn_label(cx, win, "oauth2-token-notice").as_deref(),
        Some(error)
    );
}

// "Token" takes the label column and its buttons the field column, like the rows above.
#[gpui_kit::test]
fn the_token_row_sits_on_the_form_grid(cx: &mut TestAppContext) {
    use gpui_kit::px;
    http_ready(cx);
    let file = format!(
        "{}  auth:\n{}",
        request("a", "http://x.test/r"),
        cc("http://127.0.0.1:9")
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &file)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, view) = editor_for(cx, &ws, &root, "a.yml", false);
    let panel = request_panel(cx, &view);
    settle_panel(cx, &panel);
    cx.update_window(win, |_, w, cx| {
        w.click(("section", Section::Auth as usize), cx);
        w.render_frame(cx)
    })
    .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let row = w.within("oauth2-client-auth-row");
        let (label, field) = (
            row.find("form-label").bounds(),
            row.find("form-control").bounds(),
        );
        let token = w.within("oauth2-token-row");
        let (tlabel, tfield) = (
            token.find("form-label").bounds(),
            token.find("form-control").bounds(),
        );
        let get = w.find("oauth2-get-token").bounds();
        assert_eq!(tlabel.left(), label.left());
        assert_eq!(tfield.left(), field.left());
        assert!(
            get.left() >= tfield.left() - px(0.5) && get.right() <= tfield.right() + px(0.5),
            "{get:?} in {tfield:?}"
        );
        // The status line sits in the field column under the buttons, not under the labels.
        let status = w.find("oauth2-token-status").bounds();
        assert!(
            (status.left() - get.left()).abs() < px(0.5),
            "{status:?} not under {get:?}"
        );
        assert!(status.top() >= get.bottom() - px(0.5), "{status:?} {get:?}");
        assert!(
            status.bottom() <= tfield.bottom() + px(0.5),
            "{status:?} outside {tfield:?}"
        );
    })
    .unwrap();
}

// The scope and error lines join the status line in the field column, under the buttons and
// inside the row's control, not back at the label column.
#[gpui_kit::test]
fn the_scope_and_error_lines_sit_in_the_field_column(cx: &mut TestAppContext) {
    use gpui_kit::px;
    http_ready(cx);
    let server = provider(
        cx,
        r#"{"access_token":"t1","refresh_token":"r1","expires_in":3600,"scope":"read write"}"#,
        Some(ResponseTemplate::new(401).set_body_string(r#"{"error":"invalid_grant"}"#)),
    );
    let file = format!(
        "{}  auth:\n{}",
        request("a", "http://x.test/r"),
        cc(&server.uri())
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &file)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, view) = editor_for(cx, &ws, &root, "a.yml", false);
    let panel = request_panel(cx, &view);
    cx.update_window(win, |_, w, cx| {
        w.click(("section", Section::Auth as usize), cx);
        w.render_frame(cx)
    })
    .unwrap();
    panel.update(cx, |p, cx| p.get_token(cx));
    wait_real(cx, |cx| status(cx, &panel).ends_with("refresh token"));
    panel.update(cx, |p, cx| p.refresh_token(cx));
    settle_panel(cx, &panel);
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let token = w.within("oauth2-token-row");
        let field = token.find("form-control").bounds();
        let get = w.find("oauth2-get-token").bounds();
        let mut above = get.bottom();
        for id in [
            "oauth2-token-status",
            "oauth2-token-scope",
            "oauth2-token-error",
        ] {
            let line = w.find(id).bounds();
            assert!(
                (line.left() - get.left()).abs() < px(0.5),
                "{id} {line:?} not under {get:?}"
            );
            assert!(
                line.top() >= above - px(0.5),
                "{id} {line:?} below {above:?}"
            );
            assert!(
                line.right() <= field.right() + px(0.5)
                    && line.bottom() <= field.bottom() + px(0.5),
                "{id} {line:?} outside {field:?}"
            );
            above = line.bottom();
        }
    })
    .unwrap();
}
