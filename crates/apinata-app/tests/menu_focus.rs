//! Every right-click menu hands focus back when Escape's press and release land in one update,
//! as a driver delivers them: the app's keys must work with no click in between.

use crate::support;

use apinata_app::body_view::Shown;
use apinata_app::request_editor::SendRequest;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, ElementId, TestAppContext};
use support::*;
use wiremock::ResponseTemplate;

// A right press and release over the element `id` through the whole window pipeline.
fn real_right_click(cx: &mut TestAppContext, s: &Started, id: impl Into<ElementId>) {
    use gpui_kit::{InputEvent as _, MouseButton, MouseDownEvent, MouseUpEvent};
    frame(cx, s.win);
    let id = id.into();
    let at = cx
        .update_window(s.win, |_, w, _| w.find(id).bounds().center())
        .unwrap();
    let down = MouseDownEvent {
        button: MouseButton::Right,
        position: at,
        modifiers: Default::default(),
        click_count: 1,
        first_mouse: false,
    };
    let up = MouseUpEvent {
        button: MouseButton::Right,
        position: at,
        modifiers: Default::default(),
        click_count: 1,
    };
    for event in [down.to_platform_input(), up.to_platform_input()] {
        cx.update_window(s.win, |_, w, cx| {
            w.dispatch_event(event, cx);
        })
        .unwrap();
        cx.run_until_parked();
        frame(cx, s.win);
    }
}

// One keystroke as a driver delivers it: press and release in a single update.
fn press_and_release(cx: &mut TestAppContext, s: &Started, keys: &str) {
    use gpui_kit::{KeyUpEvent, Keystroke, PlatformInput};
    let stroke = Keystroke::parse_with_os(keys, apinata_app::shortcuts::HOST_OS).unwrap();
    cx.update_window(s.win, |_, w, cx| {
        w.dispatch_keystroke(stroke.clone(), cx);
        w.dispatch_event(PlatformInput::KeyUp(KeyUpEvent { keystroke: stroke }), cx);
    })
    .unwrap();
}

// Escape closes the menu, then the palette key must open the palette.
fn escape_then_palette(cx: &mut TestAppContext, s: &Started) {
    press_and_release(cx, s, "escape");
    frame(cx, s.win);
    frame(cx, s.win);
    press_and_release(cx, s, &key("shift-p"));
    frame(cx, s.win);
    let open = cx
        .update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap();
    assert!(open, "the palette opened");
}

fn menu_open(cx: &mut TestAppContext, s: &Started) -> bool {
    cx.update_window(s.win, |_, w, _| w.try_find("popup-menu").is_some())
        .unwrap()
}

fn tree_focus(cx: &mut TestAppContext, s: &Started) {
    let sidebar = s.shell.read_with(cx, |v, _| v.sidebar().clone());
    let tree = sidebar.read_with(cx, |v, _| v.tree().clone());
    cx.update_window(s.win, |_, w, cx| tree.update(cx, |t, cx| t.focus(w, cx)))
        .unwrap();
    frame(cx, s.win);
}

fn two_files() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/a")),
        ("b.yml", &request("b", "http://x/b")),
    ])
}

#[gpui_kit::test]
fn escape_on_a_sidebar_row_menu_keeps_the_keys_working(cx: &mut TestAppContext) {
    let col = two_files();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, _| w.activate_window())
        .unwrap();
    tree_focus(cx, &s);
    real_right_click(cx, &s, row("a.yml"));
    assert!(menu_open(cx, &s), "the row menu opened");
    escape_then_palette(cx, &s);
}

#[gpui_kit::test]
fn escape_on_the_bulk_menu_keeps_the_keys_working(cx: &mut TestAppContext) {
    let col = two_files();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, _| w.activate_window())
        .unwrap();
    tree_focus(cx, &s);
    cx.update_window(s.win, |_, w, cx| w.click(row("a.yml"), cx))
        .unwrap();
    secondary_click(cx, s.win, row("b.yml"));
    real_right_click(cx, &s, row("a.yml"));
    assert!(menu_open(cx, &s), "the bulk menu opened");
    escape_then_palette(cx, &s);
}

#[gpui_kit::test]
fn escape_on_the_url_text_menu_keeps_the_keys_working(cx: &mut TestAppContext) {
    let col = two_files();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, _| w.activate_window())
        .unwrap();
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    real_right_click(cx, &s, "url");
    assert!(menu_open(cx, &s), "the text menu opened");
    escape_then_palette(cx, &s);
}

#[gpui_kit::test]
fn escape_on_the_response_body_menu_keeps_the_keys_working(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(
        cx,
        ResponseTemplate::new(200).set_body_raw("{\"a\":\"x\"}", "application/json"),
    );
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, _| w.activate_window())
        .unwrap();
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
    real_right_click(cx, &s, "body-editor");
    assert!(menu_open(cx, &s), "the body menu opened");
    escape_then_palette(cx, &s);
}

fn long_json() -> String {
    let items: Vec<String> = (0..30_000).map(|i| format!("{{\"id\":{i}}}")).collect();
    format!("[{}]", items.join(","))
}

#[gpui_kit::test]
fn escape_on_the_response_rows_menu_keeps_the_keys_working(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(
        cx,
        ResponseTemplate::new(200).set_body_raw(long_json(), "application/json"),
    );
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, _| w.activate_window())
        .unwrap();
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
        body.read_with(cx, |b, _| matches!(b.shown(), Shown::Lines(_)))
    });
    frame(cx, s.win);
    real_right_click(cx, &s, ("body-line", 3usize));
    assert!(menu_open(cx, &s), "the rows menu opened");
    escape_then_palette(cx, &s);
}
