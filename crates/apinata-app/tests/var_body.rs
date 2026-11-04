use crate::support;

use std::ops::Range;

use apinata_app::request_editor::{RequestEditor, Section};
use apinata_app::theme::VarKind;
use gpui_kit::AppContext as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, Bounds, Entity, InputEvent as _, Modifiers, MouseMoveEvent, Pixels,
    TestAppContext,
};
use support::*;

const VARS_ROOT: &str = "opencollection: 1.0.0\ninfo:\n  name: c\nrequest:\n  variables:\n    - name: host\n      value: c.test\n";
const JSON: &str = r#"info:
  name: a
  type: http
  seq: 1
http:
  method: POST
  url: "https://x.test"
  body:
    type: json
    data: '{"a":"{{host}}","b":"{{nope}}"}'
"#;
const FORM: &str = "info:\n  name: b\n  type: http\n  seq: 2\nhttp:\n  method: POST\n  url: \"https://x.test\"\n  body:\n    type: form-urlencoded\n    data:\n      - name: \"{{host}}\"\n        value: \"1\"\n";
const HOST: Range<usize> = 6..14;
const NOPE: Range<usize> = 21..29;

/// The editor for `rel` with its variables loaded and the Body section shown.
fn body_editor(
    cx: &mut TestAppContext,
    rel: &str,
) -> (tempfile::TempDir, AnyWindowHandle, Entity<RequestEditor>) {
    let col = write_tree(&[
        ("opencollection.yml", VARS_ROOT),
        ("a.yml", JSON),
        ("b.yml", FORM),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, rel, false);
    wait_for(cx, "variables", |cx| {
        ed.read_with(cx, |e, _| e.variables().is_some())
    });
    cx.update_window(win, |_, w, cx| {
        w.click(("section", Section::Body as usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    (col, win, ed)
}

fn tip_over(cx: &mut TestAppContext, win: AnyWindowHandle, at: Bounds<Pixels>) -> Option<String> {
    cx.update_window(win, |_, w, cx| {
        let event = MouseMoveEvent {
            position: at.center(),
            pressed_button: None,
            modifiers: Modifiers::default(),
        };
        w.dispatch_event(event.to_platform_input(), cx);
        w.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find("var-tooltip")
            .map(|t| t.label().unwrap_or_default().to_owned())
    })
    .unwrap()
}

fn fills(cx: &mut TestAppContext, ed: &Entity<RequestEditor>) -> Vec<(Range<usize>, VarKind)> {
    let body = ed.read_with(cx, |e, _| e.body().clone());
    body.read_with(cx, |b, _| b.token_fills().to_vec())
}

#[gpui_kit::test]
fn body_placeholders_are_tinted_and_follow_typing(cx: &mut TestAppContext) {
    let (_col, win, ed) = body_editor(cx, "a.yml");
    assert_eq!(
        fills(cx, &ed),
        [(HOST, VarKind::Defined), (NOPE, VarKind::Undefined)]
    );
    cx.update_window(win, |_, w, cx| {
        w.click_at("body-code", code_at(), cx);
        w.press("home", cx);
        w.input("x", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        fills(cx, &ed),
        [(7..15, VarKind::Defined), (22..30, VarKind::Undefined)]
    );
}

// A placeholder inside a JSON string keeps the editor's plain text colour over its tint,
// so the string colour never fights the tint.
#[gpui_kit::test]
fn body_placeholders_take_the_plain_text_colour(cx: &mut TestAppContext) {
    let (_col, _win, ed) = body_editor(cx, "a.yml");
    let body = ed.read_with(cx, |e, _| e.body().clone());
    let recoloured = body.read_with(cx, |b, cx| b.token_text_ranges(cx));
    let tinted: Vec<_> = fills(cx, &ed).into_iter().map(|(r, _)| r).collect();
    assert_eq!(recoloured, tinted);
    assert_eq!(recoloured, [HOST, NOPE]);
}

#[gpui_kit::test]
fn hovering_a_body_placeholder_explains_it(cx: &mut TestAppContext) {
    let (_col, win, ed) = body_editor(cx, "a.yml");
    let code = ed.read_with(cx, |e, cx| e.body().read(cx).code().clone());
    let host = code
        .read_with(cx, |s, _| s.range_to_bounds(&HOST))
        .expect("laid out");
    assert_eq!(
        tip_over(cx, win, host).as_deref(),
        Some("Collection · c.test")
    );
    let nope = code
        .read_with(cx, |s, _| s.range_to_bounds(&NOPE))
        .expect("laid out");
    assert_eq!(
        tip_over(cx, win, nope).as_deref(),
        Some("Undefined variable")
    );
}

#[gpui_kit::test]
fn form_body_cells_explain_placeholders(cx: &mut TestAppContext) {
    let (_col, win, ed) = body_editor(cx, "b.yml");
    let table = ed.read_with(cx, |e, cx| e.body().read(cx).table().clone());
    let name = table.read_with(cx, |t, _| t.name_input(0).cloned().expect("a row"));
    let at = name
        .read_with(cx, |s, _| s.range_to_bounds(&(0..8)))
        .expect("laid out");
    assert_eq!(
        tip_over(cx, win, at).as_deref(),
        Some("Collection · c.test")
    );
}
