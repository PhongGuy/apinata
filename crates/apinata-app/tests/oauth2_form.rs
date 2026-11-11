//! The OAuth 2.0 form around Get token: missing fields, hints, the cancelled sign-in and a
//! layout that never moves under the pointer.

use crate::support;

use apinata_app::request_editor::{RequestEditor, Section};
use apinata_app::token_panel::TokenPanel;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Bounds, Entity, Pixels, TestAppContext};
use support::*;

const BLANK: &str = "    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"\"\n    tokenConfig:\n      id: main\n";

fn panel_of(cx: &mut TestAppContext, view: &Entity<RequestEditor>) -> Entity<TokenPanel> {
    view.read_with(cx, |v, cx| {
        v.auth().read(cx).oauth2().read(cx).token_panel().cloned()
    })
    .expect("a token panel")
}

fn settle(cx: &mut TestAppContext, p: &Entity<TokenPanel>) {
    wait_real(cx, |cx| !p.read_with(cx, |p, _| p.is_busy()));
}

// An editor on `auth`, tall enough that the whole form is drawn, on its Auth section.
fn auth_editor(
    cx: &mut TestAppContext,
    auth: &str,
) -> (
    tempfile::TempDir,
    tempfile::TempDir,
    AnyWindowHandle,
    Entity<TokenPanel>,
) {
    http_ready(cx);
    let file = format!("{}  auth:\n{auth}", request("a", "http://x.test/r"));
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &file)]);
    let (state, ws, root) = open_dir(cx, &col);
    let (win, view) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.simulate_window_resize(
        win,
        gpui_kit::size(gpui_kit::px(1200.), gpui_kit::px(2400.)),
    );
    cx.update_window(win, |_, w, cx| {
        w.click(("section", Section::Auth as usize), cx);
        w.render_frame(cx)
    })
    .unwrap();
    let panel = panel_of(cx, &view);
    settle(cx, &panel);
    frame(cx, win);
    (col, state, win, panel)
}

fn bounds(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> Bounds<Pixels> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).bounds()
    })
    .unwrap()
}

#[gpui_kit::test]
fn get_token_marks_every_missing_field_at_once(cx: &mut TestAppContext) {
    let (_col, _state, win, panel) = auth_editor(cx, BLANK);
    panel.update(cx, |p, cx| p.get_token(cx));
    settle(cx, &panel);
    assert_eq!(
        panel.read_with(cx, |p, _| p.error()).as_deref(),
        Some("Access token URL and Client ID are empty")
    );
    assert!(present(cx, win, "oauth2-access-token-url-invalid-icon"));
    assert!(present(cx, win, "oauth2-client-id-invalid-icon"));
    assert!(!present(cx, win, "oauth2-scope-invalid-icon"));

    // Filling one field clears its mark and leaves the other.
    type_at_end(cx, win, None, "oauth2-client-id", "app");
    cx.run_until_parked();
    assert!(!present(cx, win, "oauth2-client-id-invalid-icon"));
    assert!(present(cx, win, "oauth2-access-token-url-invalid-icon"));
}

#[gpui_kit::test]
fn the_token_panel_follows_the_fields_and_its_message_moves_none(cx: &mut TestAppContext) {
    let (_col, _state, win, panel) = auth_editor(cx, BLANK);
    let rows = [
        "oauth2-flow-row",
        "oauth2-access-token-url-row",
        "oauth2-client-id-row",
        "oauth2-client-auth-row",
        "oauth2-token-source-row",
        "oauth2-auto-refresh-row",
        "oauth2-param-add",
    ];
    let before: Vec<_> = rows.iter().map(|id| bounds(cx, win, id)).collect();
    let token = bounds(cx, win, "oauth2-token-row");
    let add = bounds(cx, win, "oauth2-param-add");
    assert!(token.top() >= add.bottom(), "{token:?} above {add:?}");

    panel.update(cx, |p, cx| p.get_token(cx));
    settle(cx, &panel);
    frame(cx, win);
    assert!(present(cx, win, "oauth2-token-error"));
    let after: Vec<_> = rows.iter().map(|id| bounds(cx, win, id)).collect();
    assert_eq!(after, before, "a message moved the form");
}

#[gpui_kit::test]
fn the_jargon_fields_carry_hints(cx: &mut TestAppContext) {
    let (_col, _state, win, _panel) = auth_editor(cx, BLANK);
    for (row, hint) in [
        ("oauth2-token-id-row", "oauth2-token-id-hint"),
        ("oauth2-token-source-row", "oauth2-token-source-hint"),
        ("oauth2-client-auth-row", "oauth2-client-auth-hint"),
    ] {
        let (row, hint) = (bounds(cx, win, row), bounds(cx, win, hint));
        assert!(
            row.contains(&hint.origin) && hint.bottom() <= row.bottom(),
            "{hint:?} outside {row:?}"
        );
    }
}

// Looks of the painted quads that cover exactly the element `id`: border colour and width.
fn looks(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> Vec<String> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let at = w.find(id).bounds().scale(w.scale_factor());
        w.painted_quads()
            .iter()
            .filter(|q| q.bounds == at)
            .map(|q| format!("{:?} {:?}", q.border_color, q.border_widths.left))
            .collect()
    })
    .unwrap()
}

#[gpui_kit::test]
fn add_parameter_is_drawn_like_the_other_buttons(cx: &mut TestAppContext) {
    let (_col, _state, win, _panel) = auth_editor(cx, BLANK);
    let (add, get) = (
        bounds(cx, win, "oauth2-param-add"),
        bounds(cx, win, "oauth2-get-token"),
    );
    assert_eq!(add.size.height, get.size.height);
    let get_look = looks(cx, win, "oauth2-get-token");
    assert!(!get_look.is_empty(), "Get token paints a frame");
    assert_eq!(looks(cx, win, "oauth2-param-add"), get_look);
}

#[gpui_kit::test]
fn a_cancelled_sign_in_is_not_an_error(cx: &mut TestAppContext) {
    use crate::oauth2_signin::{collection, opener, provider, showing};
    http_ready(cx);
    let server = provider(cx);
    let col = collection(&server.uri(), "");
    let s = started(cx, &col);
    let _opened = opener(cx, false);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let view = s
        .shell
        .read_with(cx, |sh, cx| sh.tab_area().read(cx).editor(tab).cloned())
        .expect("an editor");
    let panel = panel_of(cx, &view);
    settle(cx, &panel);
    panel.update(cx, |p, cx| p.get_token(cx));
    wait_real(cx, |cx| showing(cx).is_some());
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("oauth2-signin-cancel", cx))
        .unwrap();
    settle(cx, &panel);
    assert_eq!(panel.read_with(cx, |p, _| p.error()), None);
    assert_eq!(
        panel.read_with(cx, |p, _| p.notice()).as_deref(),
        Some("Sign-in cancelled")
    );
    cx.update_window(s.win, |_, w, cx| {
        w.click(("section", Section::Auth as usize), cx);
        w.render_frame(cx)
    })
    .unwrap();
    frame(cx, s.win);
    assert!(present(cx, s.win, "oauth2-token-note"));
    assert!(!present(cx, s.win, "oauth2-token-error"));

    // A real failure on the next try is an error again, and the note goes.
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    handle.block_on(server.reset());
    handle.block_on(
        wiremock::Mock::given(wiremock::matchers::path("/token"))
            .respond_with(
                wiremock::ResponseTemplate::new(401)
                    .set_body_string(r#"{"error":"invalid_client"}"#),
            )
            .mount(&server),
    );
    let _playing = opener(cx, true);
    panel.update(cx, |p, cx| p.get_token(cx));
    wait_real(cx, |cx| panel.read_with(cx, |p, _| p.error()).is_some());
    settle(cx, &panel);
    assert_eq!(
        panel.read_with(cx, |p, _| p.error()).as_deref(),
        Some("The token endpoint answered 401: invalid_client")
    );
    frame(cx, s.win);
    assert!(present(cx, s.win, "oauth2-token-error"));
    assert!(!present(cx, s.win, "oauth2-token-note"));
}

#[gpui_kit::test]
fn a_folder_get_token_marks_its_missing_fields(cx: &mut TestAppContext) {
    use apinata_app::config_editor::{ConfigEditor, ConfigTarget};
    http_ready(cx);
    let folder = format!("info:\n  name: F\n  type: folder\n  seq: 1\nrequest:\n  auth:\n{BLANK}");
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder),
        ("f/a.yml", &request("a", "http://x.test/r")),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let (_win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ConfigEditor::new(ws, root, ConfigTarget::Folder(ip("f")), w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    let editor = view.read_with(cx, |v, cx| v.auth().read(cx).oauth2().clone());
    let panel = editor
        .read_with(cx, |e, _| e.token_panel().cloned())
        .expect("a token panel");
    settle(cx, &panel);
    panel.update(cx, |p, cx| p.get_token(cx));
    settle(cx, &panel);
    assert_eq!(
        panel.read_with(cx, |p, _| p.error()).as_deref(),
        Some("Access token URL and Client ID are empty")
    );
}
