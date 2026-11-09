//! "Assert this value" in the response body's node menu: it appends an enabled assertion
//! row to the request, once, and leaves the tab dirty with the new row focused.

use crate::support;

use apinata_app::body_node::{ASSERT_VALUE, NodeCopy, node_entries};
use apinata_app::request_editor::Section;
use gpui_kit::{AppContext as _, Focusable as _, TestAppContext};
use support::*;

const A: &str =
    "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://x/0\"\n";
const JSON: &str = r#"{"user":{"name":"ann","tags":["a"]},"n":1.50,"z":null}"#;

fn entry(needle: &str) -> Option<NodeCopy> {
    node_entries(JSON, JSON.find(needle).unwrap(), true)
        .into_iter()
        .find(|e| e.label == ASSERT_VALUE)
}

#[test]
fn scalars_offer_the_entry_and_containers_do_not() {
    for needle in ["ann", "1.50", "null"] {
        assert!(entry(needle).is_some(), "{needle}");
    }
    assert!(entry("\"tags\"").is_none());
    assert!(entry("{\"name").is_none());
    let off = node_entries(JSON, JSON.find("ann").unwrap(), false);
    assert!(off.iter().all(|e| e.label != ASSERT_VALUE));
}

#[test]
fn the_entry_carries_the_matching_assertion() {
    let a = entry("ann").unwrap().assertion.unwrap();
    assert_eq!(
        (a.expression(), a.operator(), a.value()),
        ("res.body.user.name", "eq", Some("ann"))
    );
    let a = entry("null").unwrap().assertion.unwrap();
    assert_eq!((a.expression(), a.operator()), ("res.body.z", "isNull"));
}

#[gpui_kit::test]
fn choosing_it_adds_one_focused_row_and_a_repeat_adds_none(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", A)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let body = ed.read_with(cx, |e, cx| e.response().read(cx).body().clone());
    let entry = entry("ann").unwrap();
    for _ in 0..2 {
        cx.update_window(win, |_, w, cx| {
            body.update(cx, |b, cx| b.activate_node(&entry, w, cx))
        })
        .unwrap();
        cx.run_until_parked();
    }
    let view = view_of(cx, &ws, &root, "a.yml");
    let rows: Vec<_> = view
        .assertions
        .iter()
        .map(|a| (a.expression(), a.operator(), a.value(), a.enabled()))
        .collect();
    assert_eq!(rows, [("res.body.user.name", "eq", Some("ann"), true)]);
    assert!(view.dirty);
    assert_eq!(ed.read_with(cx, |e, _| e.section()), Section::Assertions);
    let input = ed.read_with(cx, |e, cx| {
        e.assertions()
            .read(cx)
            .expression_input(0)
            .cloned()
            .unwrap()
    });
    let focused = cx
        .update_window(win, |_, w, cx| input.focus_handle(cx).is_focused(w))
        .unwrap();
    assert!(focused, "the new row's expression holds focus");
}

// The app with a sent request whose JSON body is the scalar "ann", in the editor, and the URL
// field focused; returns the request editor and its response body.
fn sent_with_url_focused(
    cx: &mut TestAppContext,
) -> (
    Started,
    wiremock::MockServer,
    gpui_kit::Entity<apinata_app::request_editor::RequestEditor>,
    gpui_kit::Entity<apinata_app::body_view::BodyView>,
) {
    use apinata_app::body_view::Shown;
    use apinata_app::request_editor::SendRequest;
    http_ready(cx);
    let server = mock_server(
        cx,
        wiremock::ResponseTemplate::new(200).set_body_raw("\"ann\"", "application/json"),
    );
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, _| w.activate_window())
        .unwrap();
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let ed = s
        .shell
        .read_with(cx, |v, cx| v.tab_area().read(cx).editor(tab).cloned())
        .unwrap();
    let body = ed.read_with(cx, |e, cx| e.response().read(cx).body().clone());
    cx.update_window(s.win, |_, w, cx| {
        ed.update(cx, |e, cx| e.send(&SendRequest, w, cx))
    })
    .unwrap();
    wait_real(cx, |cx| {
        body.read_with(cx, |b, _| b.shown()) == Shown::Editor
    });
    let url = ed.read_with(cx, |e, _| e.url_input().clone());
    cx.update_window(s.win, |_, w, cx| url.focus_handle(cx).focus(w, cx))
        .unwrap();
    frame(cx, s.win);
    (s, server, ed, body)
}

// Chooses Assert this value from the open menu by its label, then tells whether the new
// row's expression and whether the body editor hold focus.
fn choose_assert(
    cx: &mut TestAppContext,
    s: &Started,
    ed: &gpui_kit::Entity<apinata_app::request_editor::RequestEditor>,
    body: &gpui_kit::Entity<apinata_app::body_view::BodyView>,
) -> (bool, bool) {
    use gpui_kit::test::TestWindowExt as _;
    let ix = (0..8usize)
        .find(|&ix| {
            cx.update_window(s.win, |_, w, _| {
                w.within("popup-menu")
                    .try_find(ix)
                    .and_then(|e| e.label().map(str::to_owned))
            })
            .unwrap()
            .as_deref()
                == Some(ASSERT_VALUE)
        })
        .expect("the menu offers Assert this value");
    cx.update_window(s.win, |_, w, cx| w.within("popup-menu").click(ix, cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    frame(cx, s.win);
    let input = ed.read_with(cx, |e, cx| {
        e.assertions()
            .read(cx)
            .expression_input(0)
            .cloned()
            .unwrap()
    });
    cx.update_window(s.win, |_, w, cx| {
        (
            input.focus_handle(cx).is_focused(w),
            body.read(cx).editor().focus_handle(cx).is_focused(w),
        )
    })
    .unwrap()
}

// A right-click on the body editor through the window pipeline: the press lands past the
// text, on the root scalar.
#[gpui_kit::test]
fn choosing_it_from_the_body_menu_focuses_the_new_row(cx: &mut TestAppContext) {
    use gpui_kit::{InputEvent as _, MouseButton, MouseDownEvent, MouseUpEvent};
    let (s, _server, ed, body) = sent_with_url_focused(cx);
    let at = cx
        .update_window(s.win, |_, w, _| {
            use gpui_kit::test::TestWindowExt as _;
            w.find("body-editor").bounds().center()
        })
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
    let (row, editor) = choose_assert(cx, &s, &ed, &body);
    assert!(!editor, "focus left on the body editor");
    assert!(row, "the new row's expression holds focus");
}

// The value-actions button opens the same entries without moving focus off the URL field.
#[gpui_kit::test]
fn choosing_it_from_the_value_button_focuses_the_new_row(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let (s, _server, ed, body) = sent_with_url_focused(cx);
    cx.update_window(s.win, |_, w, cx| w.click("body-value-actions", cx))
        .unwrap();
    frame(cx, s.win);
    let (row, editor) = choose_assert(cx, &s, &ed, &body);
    assert!(!editor, "focus left on the body editor");
    assert!(row, "the new row's expression holds focus");
}
