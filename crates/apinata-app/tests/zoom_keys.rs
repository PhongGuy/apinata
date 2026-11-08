use crate::support;

use apinata_app::app::AppState;
use apinata_app::body_view::{BodyView, Shown};
use apinata_app::request_editor::{RequestEditor, SendRequest};
use apinata_app::settings::Settings;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, ElementId, Entity, Focusable, Modifiers, ScrollDelta, ScrollEvent,
    TestAppContext, VisualTestContext, point, px,
};
use support::*;
use wiremock::ResponseTemplate;

fn zoom_and_offset(cx: &mut TestAppContext) -> (f32, f32) {
    let s: Settings = cx.update(|cx| cx.global::<AppState>().settings.read(cx).get().clone());
    (s.zoom, s.response_font_offset)
}

struct Responded {
    s: Started,
    body: Entity<BodyView>,
    editor: Entity<RequestEditor>,
    server: wiremock::MockServer,
    _col: tempfile::TempDir,
}

/// The real app with `a.yml` open and a JSON response from a local server in its body.
fn responded(cx: &mut TestAppContext) -> Responded {
    http_ready(cx);
    let server = mock_server(
        cx,
        ResponseTemplate::new(200).set_body_raw("{\"a\":1}", "application/json"),
    );
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let editor = s
        .shell
        .read_with(cx, |v, cx| v.tab_area().read(cx).editor(tab).cloned())
        .unwrap();
    let body = editor.read_with(cx, |e, cx| e.response().read(cx).body().clone());
    cx.update_window(s.win, |_, w, cx| {
        editor.update(cx, |e, cx| e.send(&SendRequest, w, cx))
    })
    .unwrap();
    wait_real(cx, |cx| {
        body.read_with(cx, |b, _| b.shown()) == Shown::Editor
    });
    frame(cx, s.win);
    Responded {
        s,
        body,
        editor,
        server,
        _col: col,
    }
}

/// Presses secondary-`k` with focus on the shell root; the pointer stays where it was.
fn press_on_shell(cx: &mut TestAppContext, s: &Started, k: &str) {
    let shell = s.shell.clone();
    let mut v = VisualTestContext::from_window(s.win, cx);
    v.update(|w, cx| shell.read(cx).focus_handle(cx).focus(w, cx));
    v.simulate_keystrokes(&secondary(k));
    v.run_until_parked();
}

/// Presses secondary-`k` with focus in the response body's editor.
fn press_in_body(cx: &mut TestAppContext, r: &Responded, k: &str) {
    let ed = r.body.read_with(cx, |b, _| b.editor().clone());
    let mut v = VisualTestContext::from_window(r.s.win, cx);
    v.update(|w, cx| ed.read(cx).focus_handle(cx).focus(w, cx));
    v.simulate_keystrokes(&secondary(k));
    v.run_until_parked();
}

fn hover(cx: &mut TestAppContext, s: &Started, id: impl Into<gpui_kit::ElementId>) {
    cx.update_window(s.win, |_, w, cx| w.hover(id, cx)).unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_keys_zoom_the_app_when_the_response_is_neither_focused_nor_hovered(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    press_on_shell(cx, &s, "=");
    assert_eq!(zoom_and_offset(cx), (1.1, 0.0));
    press_on_shell(cx, &s, "+");
    assert_eq!(zoom_and_offset(cx).0, 1.2);
    for _ in 0..3 {
        press_on_shell(cx, &s, "-");
    }
    assert_eq!(zoom_and_offset(cx).0, 0.9);
    press_on_shell(cx, &s, "0");
    assert_eq!(zoom_and_offset(cx), (1.0, 0.0));
}

#[gpui_kit::test]
fn the_keys_size_the_focused_response_body_only(cx: &mut TestAppContext) {
    let r = responded(cx);
    press_in_body(cx, &r, "=");
    press_in_body(cx, &r, "=");
    assert_eq!(zoom_and_offset(cx), (1.0, 2.0));
    press_in_body(cx, &r, "-");
    assert_eq!(zoom_and_offset(cx), (1.0, 1.0));
    press_on_shell(cx, &r.s, "=");
    assert_eq!(
        zoom_and_offset(cx),
        (1.1, 1.0),
        "the app zoom keeps the response offset"
    );
    press_in_body(cx, &r, "0");
    assert_eq!(
        zoom_and_offset(cx),
        (1.1, 0.0),
        "reset acts on the targeted level only"
    );
}

#[gpui_kit::test]
fn the_keys_size_the_hovered_response_body_with_focus_elsewhere(cx: &mut TestAppContext) {
    let r = responded(cx);
    hover(cx, &r.s, "body-editor");
    press_on_shell(cx, &r.s, "=");
    assert_eq!(zoom_and_offset(cx), (1.0, 1.0));
    hover(cx, &r.s, row("a.yml"));
    press_on_shell(cx, &r.s, "=");
    assert_eq!(zoom_and_offset(cx), (1.1, 1.0));
}

#[gpui_kit::test]
fn the_keys_zoom_the_app_over_an_error_that_replaced_the_body(cx: &mut TestAppContext) {
    let r = responded(cx);
    hover(cx, &r.s, "body-editor");
    // A stopped server refuses the resend, so the pane shows an error where the body was.
    let Responded {
        s, editor, server, ..
    } = r;
    drop(server);
    cx.update_window(s.win, |_, w, cx| {
        editor.update(cx, |e, cx| e.send(&SendRequest, w, cx))
    })
    .unwrap();
    wait_real(cx, |cx| {
        editor.read_with(cx, |e, cx| e.response().read(cx).error_view(cx).is_some())
    });
    frame(cx, s.win);
    press_on_shell(cx, &s, "=");
    assert_eq!(zoom_and_offset(cx), (1.1, 0.0));
}

fn ctrl_scroll(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    id: impl Into<ElementId>,
    delta: ScrollDelta,
) {
    use gpui_kit::InputEvent as _;
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        w.hover(id.clone(), cx);
        let position = w.find(id).bounds().center();
        w.dispatch_event(
            ScrollEvent {
                position,
                delta,
                modifiers: Modifiers::secondary_key(cx),
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        w.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn lines(y: f32) -> ScrollDelta {
    ScrollDelta::Lines(point(0., y))
}

#[gpui_kit::test]
fn ctrl_scroll_over_the_response_sizes_it_and_elsewhere_zooms_the_app(cx: &mut TestAppContext) {
    let r = responded(cx);
    ctrl_scroll(cx, r.s.win, "body-editor", lines(1.));
    assert_eq!(
        zoom_and_offset(cx),
        (1.0, 1.0),
        "wheel up over the body grows it"
    );
    ctrl_scroll(cx, r.s.win, "body-editor", lines(-1.));
    ctrl_scroll(cx, r.s.win, "body-editor", lines(-1.));
    assert_eq!(zoom_and_offset(cx), (1.0, -1.0));
    ctrl_scroll(cx, r.s.win, row("a.yml"), lines(1.));
    assert_eq!(zoom_and_offset(cx), (1.1, -1.0));
    ctrl_scroll(cx, r.s.win, row("a.yml"), lines(-1.));
    assert_eq!(zoom_and_offset(cx), (1.0, -1.0));
}

#[gpui_kit::test]
fn pixel_scrolls_step_once_per_50_px(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let px30 = || ScrollDelta::Pixels(point(px(0.), px(30.)));
    ctrl_scroll(cx, s.win, row("a.yml"), px30());
    assert_eq!(zoom_and_offset(cx).0, 1.0);
    ctrl_scroll(cx, s.win, row("a.yml"), px30());
    assert_eq!(zoom_and_offset(cx).0, 1.1);
}

#[gpui_kit::test]
fn a_plain_scroll_never_zooms(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| w.scroll(row("a.yml"), lines(1.), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(zoom_and_offset(cx), (1.0, 0.0));
}
