use crate::support;

use std::ops::Range;

use apinata_app::auth_section::AuthField;
use apinata_app::request_editor::{RequestEditor, Section};
use gpui_kit::AppContext as _;
use gpui_kit::component::input::InputState;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, Entity, InputEvent as _, Modifiers, MouseMoveEvent, Pixels, Point,
    TestAppContext, point, px,
};
use support::*;

const VARS_ROOT: &str = "opencollection: 1.0.0\ninfo:\n  name: c\nrequest:\n  variables:\n    - name: host\n      value: c.test\n";
const REQ: &str = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: \"https://x.test\"\n  headers:\n    - name: \"{{host}}\"\n      value: \"{{nope}}\"\n    - name: X-Off\n      value: \"{{host}}\"\n      disabled: true\n  auth:\n    type: basic\n    username: \"{{host}}\"\n    password: \"{{host}}\"\n";
const TOKEN: Range<usize> = 0..8;

/// The editor for `a.yml` with its variables loaded and `section` shown.
fn editor(
    cx: &mut TestAppContext,
    section: Section,
) -> (tempfile::TempDir, AnyWindowHandle, Entity<RequestEditor>) {
    let col = write_tree(&[("opencollection.yml", VARS_ROOT), ("a.yml", REQ)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    wait_for(cx, "variables", |cx| {
        ed.read_with(cx, |e, _| e.variables().is_some())
    });
    cx.update_window(win, |_, w, cx| w.click(("section", section as usize), cx))
        .unwrap();
    cx.run_until_parked();
    (col, win, ed)
}

fn tip_at(cx: &mut TestAppContext, win: AnyWindowHandle, at: Point<Pixels>) -> Option<String> {
    cx.update_window(win, |_, w, cx| {
        let event = MouseMoveEvent {
            position: at,
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

fn tip(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    input: &Entity<InputState>,
) -> Option<String> {
    let at = input
        .read_with(cx, |s, _| s.range_to_bounds(&TOKEN))
        .expect("laid out")
        .center();
    tip_at(cx, win, at)
}

#[gpui_kit::test]
fn header_cells_explain_placeholders_on_enabled_rows(cx: &mut TestAppContext) {
    let (_col, win, ed) = editor(cx, Section::Headers);
    let table = ed
        .read_with(cx, |e, _| e.table(Section::Headers).cloned())
        .unwrap();
    let input = |cx: &mut TestAppContext, value: bool, ix: usize| {
        table.read_with(cx, |t, _| {
            let found = if value {
                t.value_input(ix)
            } else {
                t.name_input(ix)
            };
            found.cloned().expect("a row")
        })
    };
    let name = input(cx, false, 0);
    assert_eq!(tip(cx, win, &name).as_deref(), Some("Collection · c.test"));
    let value = input(cx, true, 0);
    assert_eq!(tip(cx, win, &value).as_deref(), Some("Undefined variable"));
    let off = input(cx, true, 1);
    assert_eq!(tip(cx, win, &off), None, "a disabled row is not explained");
}

#[gpui_kit::test]
fn auth_explains_the_username_but_never_a_masked_field(cx: &mut TestAppContext) {
    let (_col, win, ed) = editor(cx, Section::Auth);
    let auth = ed.read_with(cx, |e, _| e.auth().clone());
    let username = auth.read_with(cx, |a, _| a.input(AuthField::Username).clone());
    assert_eq!(
        tip(cx, win, &username).as_deref(),
        Some("Collection · c.test")
    );
    let password = cx
        .update_window(win, |_, w, _| w.find("auth-password").bounds())
        .unwrap();
    let inside = point(password.left() + px(20.), password.center().y);
    assert_eq!(tip_at(cx, win, inside), None);
}

// A masked field holding one reference shows it with the URL bar's tint.
#[gpui_kit::test]
fn a_shown_reference_in_a_masked_field_is_tinted(cx: &mut TestAppContext) {
    use apinata_app::theme::{VarKind, var_tint};
    let (_col, win, ed) = editor(cx, Section::Auth);
    let auth = ed.read_with(cx, |e, _| e.auth().clone());
    let password = auth.read_with(cx, |a, _| a.input(AuthField::Password).clone());
    assert!(!password.read_with(cx, |i, _| i.presentation().is_masked()));
    let tinted = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let b = w.find("auth-password").bounds();
            let scale = w.scale_factor();
            let tint = var_tint(VarKind::Defined, cx);
            w.painted_quads().into_iter().any(|q| {
                let x = q.bounds.origin.x.0 / scale;
                let y = q.bounds.origin.y.0 / scale;
                q.background.as_solid() == Some(tint)
                    && x >= f32::from(b.left()) - 0.5
                    && x <= f32::from(b.right())
                    && y >= f32::from(b.top()) - 0.5
                    && y <= f32::from(b.bottom())
            })
        })
        .unwrap();
    assert!(tinted, "the reference carries the variable tint");
}
