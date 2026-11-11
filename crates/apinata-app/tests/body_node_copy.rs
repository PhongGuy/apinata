//! Right-clicking a JSON node in the response body offers Copy value and Copy path, resolved
//! against the text on screen in the editor and in the rows list.

use crate::support;

use std::sync::Arc;

use apinata_app::body_node::{NodeCopy, node_copies, row_offset};
use apinata_app::body_view::{BodyMode, BodyView, Shown};
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::component::WindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, SharedString, TestAppContext, WindowOptions,
};
use support::*;

const JSON: &str = r#"{"user":{"name":"ann","tags":["a","b"]},"n":1.50}"#;

fn shown(cx: &mut TestAppContext, body: &str, ct: &str) -> (AnyWindowHandle, Entity<BodyView>) {
    support::init(cx);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| BodyView::new(w, cx))
            })
        })
        .unwrap();
    let response = Response::new(200)
        .with_header("content-type", ct)
        .with_text_body(body);
    let r = Arc::new(RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(response),
        Vec::new(),
    ));
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(r), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    (win, view)
}

fn editor_text(cx: &mut TestAppContext, view: &Entity<BodyView>) -> String {
    view.read_with(cx, |v, cx| v.editor().read(cx).value().to_string())
}

fn labels(copies: &[NodeCopy]) -> Vec<&'static str> {
    copies.iter().map(|c| c.label).collect()
}

fn copies_at(cx: &mut TestAppContext, view: &Entity<BodyView>, needle: &str) -> Vec<NodeCopy> {
    let text = editor_text(cx, view);
    let offset = text.find(needle).expect("needle shown");
    view.read_with(cx, |v, cx| v.node_copies(offset, cx))
}

fn set_mode(cx: &mut TestAppContext, win: AnyWindowHandle, view: &Entity<BodyView>, m: BodyMode) {
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_mode(m, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

#[test]
fn the_menu_model_for_a_scalar_an_object_and_an_array() {
    let scalar = node_copies(JSON, JSON.find("ann").unwrap());
    assert_eq!(scalar.len(), 3);
    assert_eq!(
        (scalar[0].label, scalar[0].text.as_str()),
        ("Copy value", "ann")
    );
    assert_eq!(scalar[0].toast, "Copied the value");
    assert_eq!(
        (scalar[1].label, scalar[1].text.as_str()),
        ("Copy path", "res.body.user.name")
    );
    assert_eq!(scalar[1].toast, "Copied the path");
    let object = node_copies(JSON, JSON.find("\"user\"").unwrap());
    assert_eq!(object[0].text, r#"{"name":"ann","tags":["a","b"]}"#);
    assert_eq!(object[1].text, "res.body.user");
    let array = node_copies(JSON, JSON.find("[\"a").unwrap());
    assert_eq!(array[0].text, r#"["a","b"]"#);
    assert_eq!(array[1].text, "res.body.user.tags");
    assert_eq!(
        node_copies(JSON, JSON.find("1.50").unwrap())[0].text,
        "1.50"
    );
}

#[test]
fn a_key_without_a_path_offers_only_copy_value() {
    let text = r#"{"it's":1}"#;
    let copies = node_copies(text, text.find("it").unwrap());
    assert_eq!(labels(&copies), ["Copy value", "Set as variable..."]);
    assert!(node_copies("not json", 1).is_empty());
}

#[test]
fn rows_map_back_to_text_offsets() {
    let text = "{\n  \"a\": 1,\r\n  \"b\": \"xyzxyz\"\n}";
    let rows: Vec<SharedString> = apinata_app::body_text::rows(text, 7);
    // Rows: "{", "  \"a\": ", "1,", "  \"b\": ", "\"xyzxyz", "\"", "}".
    let starts: Vec<Option<usize>> = (0..rows.len())
        .map(|i| row_offset(text, &rows, i))
        .collect();
    assert_eq!(starts[0], Some(0));
    assert_eq!(starts[1], text.find("\"a\""));
    assert_eq!(starts[2], text.find("1,"));
    assert_eq!(starts[4], text.find("\"xyz"));
    assert_eq!(row_offset(text, &rows, rows.len()), None);
}

#[gpui_kit::test]
fn the_editor_offers_the_node_in_pretty_and_raw(cx: &mut TestAppContext) {
    let (win, view) = shown(cx, JSON, "application/json");
    let name = copies_at(cx, &view, "ann");
    assert_eq!(
        labels(&name),
        ["Copy value", "Copy path", "Set as variable..."]
    );
    assert_eq!(name[1].text, "res.body.user.name");
    set_mode(cx, win, &view, BodyMode::Raw);
    let raw = copies_at(cx, &view, "1.50");
    assert_eq!(
        (raw[0].text.as_str(), raw[1].text.as_str()),
        ("1.50", "res.body.n")
    );
}

#[gpui_kit::test]
fn nothing_is_offered_for_other_kinds_filters_or_edits(cx: &mut TestAppContext) {
    let (win, view) = shown(cx, "<a>1</a>", "text/html");
    assert!(view.read_with(cx, |v, cx| v.node_copies(1, cx)).is_empty());

    let (win2, json) = shown(cx, JSON, "application/json");
    cx.update_window(win2, |_, w, cx| {
        json.update(cx, |v, cx| v.set_filter("$.user.name", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(json.read_with(cx, |v, cx| v.node_copies(1, cx)).is_empty());
    cx.update_window(win2, |_, w, cx| {
        json.update(cx, |v, cx| v.set_filter("", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!json.read_with(cx, |v, cx| v.node_copies(1, cx)).is_empty());

    json.update(cx, |v, _| v.set_editable(true));
    cx.update_window(win2, |_, w, cx| {
        json.update(cx, |v, cx| v.set_editing(true, w, cx))
    })
    .unwrap();
    assert!(json.read_with(cx, |v, cx| v.node_copies(1, cx)).is_empty());
    let _ = win;
}

#[gpui_kit::test]
fn writing_a_copy_fills_the_clipboard_and_toasts(cx: &mut TestAppContext) {
    let (win, view) = shown(cx, JSON, "application/json");
    let path = copies_at(cx, &view, "ann").remove(1);
    let before = cx
        .update_window(win, |_, w, cx| w.notifications(cx).len())
        .unwrap();
    cx.update_window(win, |_, w, cx| path.write(w, cx)).unwrap();
    cx.run_until_parked();
    assert_eq!(
        cx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
        Some("res.body.user.name")
    );
    let after = cx
        .update_window(win, |_, w, cx| w.notifications(cx).len())
        .unwrap();
    assert_eq!(after, before + 1);
}

fn long_json() -> String {
    let items: Vec<String> = (0..30_000).map(|i| format!("{{\"id\":{i}}}")).collect();
    format!("[{}]", items.join(","))
}

#[gpui_kit::test]
fn a_long_body_resolves_a_clicked_row(cx: &mut TestAppContext) {
    let (win, view) = shown(cx, &long_json(), "application/json");
    assert!(matches!(
        view.read_with(cx, |v, _| v.shown()),
        Shown::Lines(_)
    ));
    let rows = view.read_with(cx, |v, _| v.shown_rows().to_vec());
    let row = rows
        .iter()
        .position(|r| r.contains("\"id\": 12345"))
        .unwrap();
    let copies = view.read_with(cx, |v, _| v.node_copies_at_row(row));
    assert_eq!(copies[0].text, "12345");
    assert_eq!(copies[1].text, "res.body[12345].id");

    // Raw is one very long line cut into chunks; a chunk must still land on its own node.
    set_mode(cx, win, &view, BodyMode::Raw);
    let rows = view.read_with(cx, |v, _| v.shown_rows().to_vec());
    assert!(rows.len() > 2);
    let row = rows.len() / 2;
    let text = long_json();
    let offset: usize = rows[..row].iter().map(|r| r.len()).sum();
    let expected = apinata_core::json_node_at(&text, offset).unwrap();
    let got = view.read_with(cx, |v, _| v.node_copies_at_row(row));
    assert_eq!(got[0].text, expected.value);
}

// Right-clicks the editor and chooses the menu entry at `ix`; the press lands past the text,
// so the node under it is the root.
fn choose_from_editor_menu(cx: &mut TestAppContext, win: AnyWindowHandle, ix: usize) {
    use gpui_kit::test::TestWindowExt as _;
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.right_click("body-editor", cx))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.within("popup-menu").click(ix, cx))
        .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_editor_menu_lists_copy_value_and_copy_path(cx: &mut TestAppContext) {
    let (win, view) = shown(cx, JSON, "application/json");
    choose_from_editor_menu(cx, win, 3);
    let text = editor_text(cx, &view);
    let root = apinata_core::json_node_at(&text, 0).unwrap();
    assert_eq!(
        cx.read_from_clipboard().and_then(|c| c.text()),
        Some(root.value)
    );
    choose_from_editor_menu(cx, win, 4);
    assert_eq!(
        cx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
        Some("res.body")
    );
}

#[gpui_kit::test]
fn the_editor_menu_stays_plain_for_html(cx: &mut TestAppContext) {
    let (win, _view) = shown(cx, "<a>1</a>", "text/html");
    // Entries 3 and 4 would be the node entries on JSON; here they are the default menu's.
    choose_from_editor_menu(cx, win, 3);
    choose_from_editor_menu(cx, win, 4);
    assert_eq!(cx.read_from_clipboard().and_then(|c| c.text()), None);
}

#[gpui_kit::test]
fn a_rows_menu_copies_the_clicked_rows_node(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let (win, view) = shown(cx, &long_json(), "application/json");
    frame(cx, win);
    let want = view.read_with(cx, |v, _| v.node_copies_at_row(3));
    assert_eq!(want[0].text, "{\n  \"id\": 0\n}");
    cx.update_window(win, |_, w, cx| w.right_click(("body-line", 3usize), cx))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.within("popup-menu").click(1, cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        cx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
        Some(want[1].text.as_str())
    );
}

#[gpui_kit::test]
fn a_filtered_rows_view_has_no_node_menu(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let (win, view) = shown(cx, &long_json(), "application/json");
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_filter("$[*].id", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.right_click(("body-line", 2usize), cx))
        .unwrap();
    frame(cx, win);
    assert!(
        cx.update_window(win, |_, w, _| w.try_find("popup-menu").is_none())
            .unwrap()
    );
}

// The rows strip draws its own always-visible scrollbar while it overflows, with no scroll
// first, in the gutter along the frame's right edge and a frame after the body is shown.
#[gpui_kit::test]
fn a_long_body_shows_a_vertical_scrollbar_without_scrolling(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let (win, _view) = shown(cx, &long_json(), "application/json");
    frame(cx, win);
    frame(cx, win);
    assert!(present(cx, win, "body-lines-scrollbar-y"));
    let (bar, frame_b) = cx
        .update_window(win, |_, w, _| {
            (
                w.find("body-lines-scrollbar-y").bounds(),
                w.find("body-lines-frame").bounds(),
            )
        })
        .unwrap();
    assert_eq!(bar.size.width, apinata_app::style::SCROLLBAR_GUTTER);
    assert!(bar.size.height > gpui_kit::px(50.));
    assert!(
        bar.right() <= frame_b.right()
            && bar.left() >= frame_b.right() - bar.size.width - gpui_kit::px(2.)
    );
}

// One rendered frame after the body is shown is enough: the marker asks for the frame that
// has the layout the bar needs.
#[gpui_kit::test]
fn the_scrollbar_follows_layout_within_one_extra_frame(cx: &mut TestAppContext) {
    let (win, _view) = shown(cx, &long_json(), "application/json");
    frame(cx, win);
    cx.run_until_parked();
    frame(cx, win);
    assert!(present(cx, win, "body-lines-scrollbar-y"));
}

#[gpui_kit::test]
fn a_press_at_the_frame_edge_targets_the_row_under_it_not_the_previous_one(
    cx: &mut TestAppContext,
) {
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{InputEvent as _, MouseButton, MouseDownEvent, MouseUpEvent};
    let (win, view) = shown(cx, &long_json(), "application/json");
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.right_click(("body-line", 3usize), cx))
        .unwrap();
    frame(cx, win);
    assert!(
        !view
            .read_with(cx, |v, _| v.pending_node_copies())
            .is_empty()
    );
    // Dismiss, then press on the frame's right edge. Rows span the viewer, so that press
    // lands on the row at its height and must offer that row's entries, never row 3's.
    cx.update_window(win, |_, w, cx| w.press("escape", cx))
        .unwrap();
    let edge = cx
        .update_window(win, |_, w, _| {
            let b = w.find("body-lines-frame").bounds();
            let row5 = w.find(("body-line", 5usize)).bounds();
            gpui_kit::point(
                b.right() - apinata_app::style::SCROLLBAR_GUTTER - gpui_kit::px(2.),
                row5.center().y,
            )
        })
        .unwrap();
    let press_at = |cx: &mut TestAppContext, position| {
        let down = MouseDownEvent {
            button: MouseButton::Right,
            position,
            modifiers: Default::default(),
            click_count: 1,
            first_mouse: false,
        };
        let up = MouseUpEvent {
            button: MouseButton::Right,
            position,
            modifiers: Default::default(),
            click_count: 1,
        };
        for event in [down.to_platform_input(), up.to_platform_input()] {
            cx.update_window(win, |_, w, cx| {
                w.dispatch_event(event, cx);
            })
            .unwrap();
            cx.run_until_parked();
        }
    };
    press_at(cx, edge);
    let want = view.read_with(cx, |v, _| v.node_copies_at_row(5));
    let got = view.read_with(cx, |v, _| v.pending_node_copies());
    assert_eq!(got.len(), want.len());
    assert_eq!(got[0].text, want[0].text);
    // The scrollbar gutter owns the strip beside it: a press there offers no node menu.
    cx.update_window(win, |_, w, cx| w.press("escape", cx))
        .unwrap();
    let gutter = cx
        .update_window(win, |_, w, _| {
            let b = w.find("body-lines-frame").bounds();
            let row5 = w.find(("body-line", 5usize)).bounds();
            gpui_kit::point(b.right() - gpui_kit::px(2.), row5.center().y)
        })
        .unwrap();
    press_at(cx, gutter);
    assert!(
        view.read_with(cx, |v, _| v.pending_node_copies())
            .is_empty()
    );
}

#[gpui_kit::test]
fn showing_a_new_body_drops_pending_entries(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let (win, view) = shown(cx, &long_json(), "application/json");
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.right_click(("body-line", 3usize), cx))
        .unwrap();
    assert!(
        !view
            .read_with(cx, |v, _| v.pending_node_copies())
            .is_empty()
    );
    let next = Arc::new(RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(Response::new(200).with_text_body("x")),
        Vec::new(),
    ));
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(next), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        view.read_with(cx, |v, _| v.pending_node_copies())
            .is_empty()
    );
}

// Puts the editor caret at byte `offset` without any press, as arrow keys or a click would.
fn place_caret(cx: &mut TestAppContext, view: &Entity<BodyView>, offset: usize) {
    let editor = view.read_with(cx, |v, _| v.editor().clone());
    editor.update(cx, |e, cx| e.set_selected_range(offset..offset, cx));
}

fn copy_at_caret(cx: &mut TestAppContext, win: AnyWindowHandle, view: &Entity<BodyView>, l: &str) {
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.copy_caret_node(l, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

fn clipboard(cx: &mut TestAppContext) -> Option<String> {
    cx.read_from_clipboard().and_then(|c| c.text())
}

#[gpui_kit::test]
fn the_caret_actions_copy_the_node_under_a_keyboard_caret(cx: &mut TestAppContext) {
    let (win, view) = shown(cx, JSON, "application/json");
    let text = editor_text(cx, &view);
    place_caret(cx, &view, text.find("ann").unwrap() + 1);
    copy_at_caret(cx, win, &view, "Copy value");
    assert_eq!(clipboard(cx).as_deref(), Some("ann"));
    copy_at_caret(cx, win, &view, "Copy path");
    assert_eq!(clipboard(cx).as_deref(), Some("res.body.user.name"));
}

#[gpui_kit::test]
fn a_caret_on_the_empty_trailing_line_copies_nothing(cx: &mut TestAppContext) {
    let body = "{\"a\": [\n  1.50\n]}\n";
    let (win, view) = shown(cx, body, "application/json");
    set_mode(cx, win, &view, BodyMode::Raw);
    let text = editor_text(cx, &view);
    assert!(text.ends_with('\n'));
    place_caret(cx, &view, text.len());
    copy_at_caret(cx, win, &view, "Copy value");
    assert_eq!(clipboard(cx), None);
    // Right of the `[` the array is copied exactly as written, not the root.
    place_caret(cx, &view, text.find('[').unwrap() + 1);
    copy_at_caret(cx, win, &view, "Copy value");
    assert_eq!(clipboard(cx).as_deref(), Some("[\n  1.50\n]"));
}

#[gpui_kit::test]
fn the_caret_actions_ignore_rows_mode_and_other_kinds(cx: &mut TestAppContext) {
    let (win, view) = shown(cx, &long_json(), "application/json");
    assert!(matches!(
        view.read_with(cx, |v, _| v.shown()),
        Shown::Lines(_)
    ));
    copy_at_caret(cx, win, &view, "Copy value");
    copy_at_caret(cx, win, &view, "Copy path");
    assert_eq!(clipboard(cx), None);
    let (win, html) = shown(cx, "<a>1</a>", "text/html");
    copy_at_caret(cx, win, &html, "Copy value");
    assert_eq!(clipboard(cx), None);
}

#[test]
fn the_menu_model_copies_a_container_exactly_as_written() {
    let text = r#"{"n":[1.50,{"b":2,"b":3}]}"#;
    let copies = node_copies(text, text.find("[1").unwrap());
    assert_eq!(copies[0].text, r#"[1.50,{"b":2,"b":3}]"#);
}

#[gpui_kit::test]
fn the_palette_actions_copy_from_the_active_tabs_body(cx: &mut TestAppContext) {
    use apinata_app::palette::palette_items;
    let col = two_requests();
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    let editor = editor_of(cx, &s, tab);
    let result = Arc::new(RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(
            Response::new(200)
                .with_header("content-type", "application/json")
                .with_text_body(JSON),
        ),
        Vec::new(),
    ));
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    model.update(cx, |m, cx| m.show(result, cx));
    frame(cx, s.win);
    let body = editor.read_with(cx, |e, cx| e.response().read(cx).body().clone());
    let text = editor_text(cx, &body);
    place_caret(cx, &body, text.find("ann").unwrap());
    for (label, want) in [
        ("Copy JSON value at caret", "ann"),
        ("Copy JSON path at caret", "res.body.user.name"),
    ] {
        let entry = palette_items()
            .into_iter()
            .find(|e| e.label == label)
            .unwrap_or_else(|| panic!("{label} is in the palette"));
        cx.update_window(s.win, |_, w, cx| {
            w.dispatch_action(entry.action.boxed_clone(), cx)
        })
        .unwrap();
        frame(cx, s.win);
        assert_eq!(clipboard(cx).as_deref(), Some(want), "{label}");
    }
}

#[gpui_kit::test]
fn the_editor_menu_set_as_variable_asks_the_shell_with_the_key_and_value(cx: &mut TestAppContext) {
    let (win, view) = shown(cx, JSON, "application/json");
    let asked: std::rc::Rc<std::cell::RefCell<Vec<(String, String)>>> = Default::default();
    let sink = asked.clone();
    let _sub = cx.update(|cx| {
        cx.subscribe(
            &view,
            move |_, e: &apinata_app::body_view::SetVariable, _| {
                sink.borrow_mut().push((e.name.clone(), e.value.clone()))
            },
        )
    });
    // The press lands past the text, so the node is the root object: no key to prefill.
    choose_from_editor_menu(cx, win, 5);
    let root = apinata_core::json_node_at(&editor_text(cx, &view), 0).unwrap();
    assert_eq!(*asked.borrow(), [(String::new(), root.value)]);
}

#[gpui_kit::test]
fn a_rows_menu_set_as_variable_asks_with_the_clicked_rows_key(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let (win, view) = shown(cx, &long_json(), "application/json");
    frame(cx, win);
    let asked: std::rc::Rc<std::cell::RefCell<Vec<(String, String)>>> = Default::default();
    let sink = asked.clone();
    let _sub = cx.update(|cx| {
        cx.subscribe(
            &view,
            move |_, e: &apinata_app::body_view::SetVariable, _| {
                sink.borrow_mut().push((e.name.clone(), e.value.clone()))
            },
        )
    });
    let want = view.read_with(cx, |v, _| v.node_copies_at_row(3));
    cx.update_window(win, |_, w, cx| w.right_click(("body-line", 3usize), cx))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.within("popup-menu").click(want.len() - 1, cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(asked.borrow().len(), 1);
    assert_eq!(asked.borrow()[0].1, want[0].text);
}

type Asked = std::rc::Rc<std::cell::RefCell<Vec<(String, String)>>>;

// Records every SetVariable the body view emits.
fn watch_set_variable(cx: &mut TestAppContext, view: &Entity<BodyView>) -> Asked {
    let asked: Asked = Default::default();
    let sink = asked.clone();
    let sub = cx.update(|cx| {
        cx.subscribe(
            view,
            move |_, e: &apinata_app::body_view::SetVariable, _| {
                sink.borrow_mut().push((e.name.clone(), e.value.clone()))
            },
        )
    });
    std::mem::forget(sub);
    asked
}

fn set_variable_at_caret(cx: &mut TestAppContext, win: AnyWindowHandle, view: &Entity<BodyView>) {
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_caret_node_variable(w, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_caret_node_opens_set_as_variable_with_its_key_and_value(cx: &mut TestAppContext) {
    let (win, view) = shown(cx, JSON, "application/json");
    let asked = watch_set_variable(cx, &view);
    let text = editor_text(cx, &view);
    place_caret(cx, &view, text.find("ann").unwrap() + 1);
    set_variable_at_caret(cx, win, &view);
    assert_eq!(*asked.borrow(), [("name".to_string(), "ann".to_string())]);
}

#[gpui_kit::test]
fn set_variable_at_the_caret_is_inert_where_the_node_menu_is(cx: &mut TestAppContext) {
    let (win, rows) = shown(cx, &long_json(), "application/json");
    let asked = watch_set_variable(cx, &rows);
    set_variable_at_caret(cx, win, &rows);
    let (win, html) = shown(cx, "<a>1</a>", "text/html");
    let more = watch_set_variable(cx, &html);
    set_variable_at_caret(cx, win, &html);
    let (win, json) = shown(cx, JSON, "application/json");
    let filtered = watch_set_variable(cx, &json);
    let text = editor_text(cx, &json);
    place_caret(cx, &json, text.find("ann").unwrap() + 1);
    cx.update_window(win, |_, w, cx| {
        json.update(cx, |v, cx| v.set_filter("$.user.name", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    set_variable_at_caret(cx, win, &json);
    cx.update_window(win, |_, w, cx| {
        json.update(cx, |v, cx| v.set_filter("", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        json.update(cx, |v, cx| v.set_editing(true, w, cx))
    })
    .unwrap();
    set_variable_at_caret(cx, win, &json);
    assert!(asked.borrow().is_empty());
    assert!(more.borrow().is_empty());
    assert!(filtered.borrow().is_empty());
}

#[gpui_kit::test]
fn set_variable_with_no_node_at_the_caret_emits_nothing(cx: &mut TestAppContext) {
    let body = "{\"a\": [\n  1.50\n]}\n";
    let (win, view) = shown(cx, body, "application/json");
    set_mode(cx, win, &view, BodyMode::Raw);
    let asked = watch_set_variable(cx, &view);
    let text = editor_text(cx, &view);
    place_caret(cx, &view, text.len());
    set_variable_at_caret(cx, win, &view);
    assert!(asked.borrow().is_empty());
}

#[gpui_kit::test]
fn the_palette_sets_the_caret_node_as_a_variable(cx: &mut TestAppContext) {
    use apinata_app::palette::palette_items;
    let col = two_requests();
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    let editor = editor_of(cx, &s, tab);
    let result = Arc::new(RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(
            Response::new(200)
                .with_header("content-type", "application/json")
                .with_text_body(JSON),
        ),
        Vec::new(),
    ));
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    model.update(cx, |m, cx| m.show(result, cx));
    frame(cx, s.win);
    let body = editor.read_with(cx, |e, cx| e.response().read(cx).body().clone());
    let asked = watch_set_variable(cx, &body);
    let text = editor_text(cx, &body);
    place_caret(cx, &body, text.find("ann").unwrap());
    let entry = palette_items()
        .into_iter()
        .find(|e| e.label == "Set JSON value at caret as variable…")
        .expect("in the palette");
    cx.update_window(s.win, |_, w, cx| {
        w.dispatch_action(entry.action.boxed_clone(), cx)
    })
    .unwrap();
    frame(cx, s.win);
    assert_eq!(*asked.borrow(), [("name".to_string(), "ann".to_string())]);
}
