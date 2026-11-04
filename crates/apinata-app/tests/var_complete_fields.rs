use crate::support;

use apinata_app::auth_section::AuthField;
use apinata_app::request_editor::{RequestEditor, Section};
use gpui_kit::AppContext as _;
use gpui_kit::component::input::InputState;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, Entity, TestAppContext};
use support::*;

const VARS_ROOT: &str = "opencollection: 1.0.0\ninfo:\n  name: c\nrequest:\n  variables:\n    - name: host\n      value: c.test\n    - name: itemId\n      value: \"7\"\n";
const REQ: &str = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: \"https://x.test/\"\n  headers:\n    - name: X-A\n      value: \"\"\n  body:\n    type: text\n    data: \"say \"\n  auth:\n    type: basic\n    username: \"\"\n    password: \"\"\n";
const JSON: &str = "info:\n  name: b\n  type: http\n  seq: 2\nhttp:\n  method: POST\n  url: \"https://x.test/\"\n  body:\n    type: json\n    data: \"\"\n";

struct Opened {
    _col: tempfile::TempDir,
    _state: tempfile::TempDir,
    ws: Entity<apinata_app::workspace::Workspace>,
    root: std::path::PathBuf,
    win: AnyWindowHandle,
    ed: Entity<RequestEditor>,
}

/// The editor for `rel` with its variables loaded.
fn opened(cx: &mut TestAppContext, rel: &str) -> Opened {
    let col = write_tree(&[
        ("opencollection.yml", VARS_ROOT),
        ("a.yml", REQ),
        ("b.yml", JSON),
    ]);
    let (state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, rel, false);
    wait_for(cx, "variables", |cx| {
        ed.read_with(cx, |e, _| e.variables().is_some())
    });
    Opened {
        _col: col,
        _state: state,
        ws,
        root,
        win,
        ed,
    }
}

fn keys(cx: &mut TestAppContext, win: AnyWindowHandle, text: &str, then: &[&str]) {
    cx.update_window(win, |_, w, cx| {
        w.input(text, cx);
        for k in then {
            w.render_frame(cx);
            w.press(k, cx);
        }
        w.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn focus_end(cx: &mut TestAppContext, win: AnyWindowHandle, input: &Entity<InputState>) {
    cx.update_window(win, |_, w, cx| {
        input.update(cx, |s, cx| s.focus(w, cx));
        w.render_frame(cx);
        w.press("end", cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn listed(cx: &mut TestAppContext, win: AnyWindowHandle) -> bool {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find("var-complete").is_some()
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_url_bar_completes_with_tab_and_the_draft_follows(cx: &mut TestAppContext) {
    let o = opened(cx, "a.yml");
    let url = o.ed.read_with(cx, |e, _| e.url_input().clone());
    focus_end(cx, o.win, &url);
    keys(cx, o.win, "{{ho", &[]);
    assert!(listed(cx, o.win));
    keys(cx, o.win, "", &["tab"]);
    assert!(!listed(cx, o.win));
    assert_eq!(
        o.ed.read_with(cx, |e, cx| e.url(cx)),
        "https://x.test/{{host}}"
    );
    let focus = o.ed.read_with(cx, |e, cx| e.url_focus(cx));
    assert!(
        cx.update_window(o.win, |_, w, _| focus.is_focused(w))
            .unwrap(),
        "Tab stayed in the field"
    );
    let (ws, root) = (o.ws.clone(), o.root.clone());
    wait_for(cx, "the draft", |cx| {
        view_of(cx, &ws, &root, "a.yml").url == "https://x.test/{{host}}"
    });
}

#[gpui_kit::test]
fn a_header_value_completes_with_enter(cx: &mut TestAppContext) {
    let o = opened(cx, "a.yml");
    cx.update_window(o.win, |_, w, cx| {
        w.click(("section", Section::Headers as usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    let table =
        o.ed.read_with(cx, |e, _| e.table(Section::Headers).cloned())
            .unwrap();
    let value = table.read_with(cx, |t, _| t.value_input(0).cloned().expect("a row"));
    focus_end(cx, o.win, &value);
    keys(cx, o.win, "{{h", &["enter"]);
    assert_eq!(
        value.read_with(cx, |s, _| s.value().to_string()),
        "{{host}}"
    );
}

#[gpui_kit::test]
fn the_username_completes_but_the_password_never_lists(cx: &mut TestAppContext) {
    let o = opened(cx, "a.yml");
    cx.update_window(o.win, |_, w, cx| {
        w.click(("section", Section::Auth as usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    let auth = o.ed.read_with(cx, |e, _| e.auth().clone());
    let username = auth.read_with(cx, |a, _| a.input(AuthField::Username).clone());
    let password = auth.read_with(cx, |a, _| a.input(AuthField::Password).clone());
    focus_end(cx, o.win, &username);
    keys(cx, o.win, "{{", &["tab"]);
    assert_eq!(
        username.read_with(cx, |s, _| s.value().to_string()),
        "{{host}}"
    );
    focus_end(cx, o.win, &password);
    keys(cx, o.win, "{{", &[]);
    assert!(!listed(cx, o.win));
}

#[gpui_kit::test]
fn the_body_editor_completes_and_keeps_its_keys_for_the_list(cx: &mut TestAppContext) {
    let o = opened(cx, "a.yml");
    cx.update_window(o.win, |_, w, cx| {
        w.click(("section", Section::Body as usize), cx);
        w.render_frame(cx);
        w.click_at("body-code", code_at(), cx);
        w.press("end", cx);
    })
    .unwrap();
    cx.run_until_parked();
    keys(cx, o.win, "{{", &["down", "up", "tab"]);
    let code = o.ed.read_with(cx, |e, cx| e.body().read(cx).code().clone());
    assert_eq!(
        code.read_with(cx, |s, _| (s.value().to_string(), s.cursor())),
        ("say {{host}}".to_owned(), 12),
        "Tab inserted the first name instead of indenting"
    );
    keys(cx, o.win, " {{zq", &[]);
    assert!(!listed(cx, o.win), "no name contains zq");
}

#[gpui_kit::test]
fn completion_keeps_the_auto_closed_quote(cx: &mut TestAppContext) {
    let o = opened(cx, "b.yml");
    cx.update_window(o.win, |_, w, cx| {
        w.click(("section", Section::Body as usize), cx);
        w.render_frame(cx);
        w.click_at("body-code", code_at(), cx);
    })
    .unwrap();
    cx.run_until_parked();
    keys(cx, o.win, r#"{"a": "{{ite"#, &["tab"]);
    let code = o.ed.read_with(cx, |e, cx| e.body().read(cx).code().clone());
    let text = |cx: &mut TestAppContext| code.read_with(cx, |s, _| s.value().to_string());
    assert_eq!(text(cx), r#"{"a": "{{itemId}}"}"#);
    keys(cx, o.win, "\"", &[]);
    assert_eq!(
        text(cx),
        r#"{"a": "{{itemId}}"}"#,
        "the closer is stepped over"
    );
}

#[gpui_kit::test]
fn the_body_editor_does_not_double_auto_closed_braces(cx: &mut TestAppContext) {
    let o = opened(cx, "b.yml");
    cx.update_window(o.win, |_, w, cx| {
        w.click(("section", Section::Body as usize), cx);
        w.render_frame(cx);
        w.click_at("body-code", code_at(), cx);
    })
    .unwrap();
    cx.run_until_parked();
    keys(cx, o.win, "{{h", &[]);
    let code = o.ed.read_with(cx, |e, cx| e.body().read(cx).code().clone());
    assert_eq!(
        code.read_with(cx, |s, _| s.value().to_string()),
        "{{h}}",
        "auto-closed"
    );
    keys(cx, o.win, "", &["enter"]);
    let (text, cursor) = code.read_with(cx, |s, _| (s.value().to_string(), s.cursor()));
    assert_eq!(text, "{{host}}");
    assert_eq!(cursor, 8);
}
