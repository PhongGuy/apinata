//! The response body's value-actions button: an ellipsis at the viewport edge that opens the
//! node menu for the caret line (editor) or the hovered row (rows), with no right-click.

use crate::support;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use apinata_app::body_node::{ASSERT_VALUE, SET_VARIABLE};
use apinata_app::body_view::{AssertValue, BodyView, SetVariable, Shown};
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::InputEvent as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, WindowOptions};
use support::*;

const JSON: &str = r#"{"user":{"name":"ann","tags":["a","b"]},"n":1.50}"#;
const BUTTON: &str = "body-value-actions";

fn shown(cx: &mut TestAppContext, body: &str, ct: &str) -> (AnyWindowHandle, Entity<BodyView>) {
    support::init(cx);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| BodyView::new(w, cx))
            })
        })
        .unwrap();
    view.update(cx, |v, _| v.set_assert_offered(true));
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
    frame(cx, win);
    (win, view)
}

fn long_json() -> String {
    let items: Vec<String> = (0..30_000).map(|i| format!("{{\"id\":{i}}}")).collect();
    format!("[{}]", items.join(","))
}

fn has_button(cx: &mut TestAppContext, win: AnyWindowHandle) -> bool {
    cx.update_window(win, |_, w, _| w.try_find(BUTTON).is_some())
        .unwrap()
}

fn place_caret(cx: &mut TestAppContext, view: &Entity<BodyView>, offset: usize) {
    let editor = view.read_with(cx, |v, _| v.editor().clone());
    editor.update(cx, |e, cx| e.set_selected_range(offset..offset, cx));
}

fn caret(cx: &mut TestAppContext, view: &Entity<BodyView>) -> usize {
    view.read_with(cx, |v, cx| v.editor().read(cx).cursor())
}

// Opens the button's menu and chooses the entry labelled `label`.
fn choose(cx: &mut TestAppContext, win: AnyWindowHandle, label: &str, labels: &[&str]) {
    cx.update_window(win, |_, w, cx| w.click(BUTTON, cx))
        .unwrap();
    frame(cx, win);
    let ix = labels.iter().position(|l| *l == label).unwrap();
    cx.update_window(win, |_, w, cx| w.within("popup-menu").click(ix, cx))
        .unwrap();
    cx.run_until_parked();
}

const ENTRIES: [&str; 4] = ["Copy value", "Copy path", SET_VARIABLE, ASSERT_VALUE];

type Asked = Rc<RefCell<Vec<(String, String)>>>;

fn watch_set(cx: &mut TestAppContext, view: &Entity<BodyView>) -> Asked {
    let asked: Asked = Default::default();
    let sink = asked.clone();
    let sub = cx.update(|cx| {
        cx.subscribe(view, move |_, e: &SetVariable, _| {
            sink.borrow_mut().push((e.name.clone(), e.value.clone()))
        })
    });
    std::mem::forget(sub);
    asked
}

fn watch_assert(cx: &mut TestAppContext, view: &Entity<BodyView>) -> Rc<RefCell<usize>> {
    let count = Rc::new(RefCell::new(0));
    let sink = count.clone();
    let sub =
        cx.update(|cx| cx.subscribe(view, move |_, _: &AssertValue, _| *sink.borrow_mut() += 1));
    std::mem::forget(sub);
    count
}

#[gpui_kit::test]
fn the_editor_button_sets_the_caret_node_as_a_variable(cx: &mut TestAppContext) {
    let (win, view) = shown(cx, JSON, "application/json");
    let at = view.read_with(cx, |v, cx| v.editor().read(cx).value().to_string());
    let at = at.find("ann").unwrap() + 1;
    place_caret(cx, &view, at);
    frame(cx, win);
    assert!(has_button(cx, win));
    let asked = watch_set(cx, &view);
    choose(cx, win, SET_VARIABLE, &ENTRIES);
    assert_eq!(*asked.borrow(), [("name".to_string(), "ann".to_string())]);
    assert_eq!(caret(cx, &view), at, "the click leaves the caret alone");
}

#[gpui_kit::test]
fn the_editor_button_asserts_the_caret_node(cx: &mut TestAppContext) {
    let (win, view) = shown(cx, JSON, "application/json");
    let text = view.read_with(cx, |v, cx| v.editor().read(cx).value().to_string());
    let at = text.find("1.50").unwrap();
    place_caret(cx, &view, at);
    frame(cx, win);
    let count = watch_assert(cx, &view);
    choose(cx, win, ASSERT_VALUE, &ENTRIES);
    assert_eq!(*count.borrow(), 1);
    assert_eq!(caret(cx, &view), at, "the click leaves the caret alone");
}

#[gpui_kit::test]
fn the_button_is_absent_for_other_kinds_filters_and_edits(cx: &mut TestAppContext) {
    let (win, _view) = shown(cx, "<a>1</a>", "text/html");
    assert!(!has_button(cx, win));

    let (win, view) = shown(cx, JSON, "application/json");
    assert!(has_button(cx, win));
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_filter("$.user.name", w, cx))
    })
    .unwrap();
    frame(cx, win);
    assert!(!has_button(cx, win), "filtered");
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_filter("", w, cx))
    })
    .unwrap();
    frame(cx, win);
    assert!(has_button(cx, win));
    view.update(cx, |v, _| v.set_editable(true));
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_editing(true, w, cx))
    })
    .unwrap();
    frame(cx, win);
    assert!(!has_button(cx, win), "editing");
}

#[gpui_kit::test]
fn the_rows_button_follows_the_hovered_row_and_acts_on_it(cx: &mut TestAppContext) {
    let (win, view) = shown(cx, &long_json(), "application/json");
    assert!(matches!(
        view.read_with(cx, |v, _| v.shown()),
        Shown::Lines(_)
    ));
    assert!(!has_button(cx, win), "nothing hovered yet");
    cx.update_window(win, |_, w, cx| w.hover(("body-line", 3usize), cx))
        .unwrap();
    frame(cx, win);
    assert!(has_button(cx, win));
    let want = view.read_with(cx, |v, _| v.node_copies_at_row(3));
    let asked = watch_set(cx, &view);
    choose(cx, win, SET_VARIABLE, &ENTRIES);
    assert_eq!(asked.borrow().len(), 1);
    assert_eq!(asked.borrow()[0].1, want[0].text);
}

#[gpui_kit::test]
fn a_long_body_draws_one_affordance_not_one_per_row(cx: &mut TestAppContext) {
    let (win, _view) = shown(cx, &long_json(), "application/json");
    cx.update_window(win, |_, w, cx| w.hover(("body-line", 2usize), cx))
        .unwrap();
    frame(cx, win);
    let per_row = cx
        .update_window(win, |_, w, _| {
            (0..40usize)
                .filter(|ix| w.try_find((BUTTON, *ix)).is_some())
                .count()
        })
        .unwrap();
    assert_eq!(per_row, 0);
    assert!(has_button(cx, win));
}

// A body with no focus anywhere: the menu has nothing to hand focus back to, so the view
// must pick the text itself.
fn open_and_dismiss(cx: &mut TestAppContext, win: AnyWindowHandle) {
    cx.update_window(win, |_, w, cx| w.click(BUTTON, cx))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.press("escape", cx))
        .unwrap();
    frame(cx, win);
    cx.run_until_parked();
}

fn focused(cx: &mut TestAppContext, win: AnyWindowHandle, h: &gpui_kit::FocusHandle) -> bool {
    cx.update_window(win, |_, w, _| h.is_focused(w)).unwrap()
}

fn blur(cx: &mut TestAppContext, win: AnyWindowHandle) {
    cx.update_window(win, |_, w, cx| w.blur(cx)).unwrap();
    frame(cx, win);
}

#[gpui_kit::test]
fn closing_the_editor_menu_with_no_prior_focus_focuses_the_editor(cx: &mut TestAppContext) {
    use gpui_kit::Focusable as _;
    let (win, view) = shown(cx, JSON, "application/json");
    let editor = view.read_with(cx, |v, _| v.editor().clone());
    let handle = editor.read_with(cx, |e, cx| e.focus_handle(cx));
    blur(cx, win);
    assert!(!focused(cx, win, &handle));
    open_and_dismiss(cx, win);
    assert!(focused(cx, win, &handle));
}

#[gpui_kit::test]
fn closing_the_rows_menu_with_no_prior_focus_focuses_the_body(cx: &mut TestAppContext) {
    let (win, view) = shown(cx, &long_json(), "application/json");
    cx.update_window(win, |_, w, cx| w.hover(("body-line", 3usize), cx))
        .unwrap();
    frame(cx, win);
    let handle = view.read_with(cx, |v, _| v.body_focus().clone());
    blur(cx, win);
    assert!(!focused(cx, win, &handle));
    open_and_dismiss(cx, win);
    assert!(focused(cx, win, &handle));
}

fn tall_json() -> String {
    let items: Vec<String> = (0..400).map(|i| i.to_string()).collect();
    format!("[{}]", items.join(","))
}

#[gpui_kit::test]
fn the_editor_button_hides_when_the_caret_line_scrolls_out_of_view(cx: &mut TestAppContext) {
    let (win, view) = shown(cx, &tall_json(), "application/json");
    assert!(matches!(
        view.read_with(cx, |v, _| v.shown()),
        Shown::Editor
    ));
    place_caret(cx, &view, 0);
    frame(cx, win);
    frame(cx, win);
    assert!(has_button(cx, win));
    let editor = view.read_with(cx, |v, _| v.editor().clone());
    editor.update(cx, |e, cx| {
        e.set_scroll_offset(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-4000.)), cx)
    });
    for _ in 0..3 {
        frame(cx, win);
    }
    assert!(!has_button(cx, win), "the caret line is off screen");
}

#[gpui_kit::test]
fn the_rows_button_hides_when_the_hovered_row_scrolls_out_of_view(cx: &mut TestAppContext) {
    let (win, _view) = shown(cx, &long_json(), "application/json");
    cx.update_window(win, |_, w, cx| w.hover(("body-line", 3usize), cx))
        .unwrap();
    frame(cx, win);
    assert!(has_button(cx, win));
    cx.update_window(win, |_, w, cx| {
        w.scroll(
            "body-lines-frame",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-2000.))),
            cx,
        )
    })
    .unwrap();
    frame(cx, win);
    assert!(!has_button(cx, win), "row 3 is off screen");
}

#[gpui_kit::test]
fn an_open_menu_freezes_the_hovered_row_and_closing_it_releases_the_row(cx: &mut TestAppContext) {
    let (win, view) = shown(cx, &long_json(), "application/json");
    cx.update_window(win, |_, w, cx| w.hover(("body-line", 3usize), cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(view.read_with(cx, |v, _| v.hovered_row()), Some(3));
    cx.update_window(win, |_, w, cx| w.click(BUTTON, cx))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.hover(("body-line", 5usize), cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(
        view.read_with(cx, |v, _| v.hovered_row()),
        Some(3),
        "frozen"
    );
    cx.update_window(win, |_, w, cx| w.press("escape", cx))
        .unwrap();
    frame(cx, win);
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, _| v.hovered_row()), None);
}

#[gpui_kit::test]
fn leaving_the_frame_clears_the_hovered_row_and_the_button(cx: &mut TestAppContext) {
    let (win, view) = shown(cx, &long_json(), "application/json");
    cx.update_window(win, |_, w, cx| w.hover(("body-line", 3usize), cx))
        .unwrap();
    frame(cx, win);
    assert!(has_button(cx, win));
    cx.update_window(win, |_, w, cx| w.hover("body-copy", cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(view.read_with(cx, |v, _| v.hovered_row()), None);
    assert!(!has_button(cx, win));
}

// Many short rows: each is far narrower than the viewer.
fn short_rows() -> String {
    let rows: Vec<String> = (0..30_000).map(|i| format!("{{\"i\":{i}}}")).collect();
    format!("[\n{}\n]", rows.join(",\n"))
}

#[gpui_kit::test]
fn a_short_row_spans_the_viewer_so_its_empty_right_side_selects_it(cx: &mut TestAppContext) {
    let (win, view) = shown(cx, &short_rows(), "application/json");
    assert!(matches!(
        view.read_with(cx, |v, _| v.shown()),
        Shown::Lines(_)
    ));
    let (row, frame_w) = cx
        .update_window(win, |_, w, _| {
            (
                w.find(("body-line", 3usize)).bounds(),
                w.find("body-lines-frame").bounds(),
            )
        })
        .unwrap();
    assert!(
        row.right() >= frame_w.right() - apinata_app::style::SCROLLBAR_GUTTER - gpui_kit::px(4.),
        "row {row:?} frame {frame_w:?}"
    );
    // A pointer past the text, inside the row's box, picks the row up.
    let pos = gpui_kit::point(row.right() - gpui_kit::px(8.), row.center().y);
    cx.update_window(win, |_, w, cx| {
        w.dispatch_event(
            gpui_kit::MouseMoveEvent {
                position: pos,
                pressed_button: None,
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
    })
    .unwrap();
    frame(cx, win);
    assert_eq!(view.read_with(cx, |v, _| v.hovered_row()), Some(3));
    assert!(has_button(cx, win));
}

// Wires the body's SetVariable event to a stand-in dialog with Set and Cancel buttons, opened
// the way the real one is (deferred), so focus restore is exercised on every close path.
fn dialog_on_event(cx: &mut TestAppContext, win: AnyWindowHandle, view: &Entity<BodyView>) {
    use gpui_kit::ParentElement as _;
    use gpui_kit::component::WindowExt as _;
    use gpui_kit::component::button::Button;
    let sub = cx
        .update_window(win, |_, w, cx| {
            let me = view.clone();
            view.update(cx, |_, cx| {
                cx.subscribe_in(&me, w, |_, _, _: &SetVariable, window, cx| {
                    window.defer(cx, |window, cx| {
                        window.open_dialog(cx, |dialog, _, _| {
                            dialog.child(
                                gpui_kit::div()
                                    .child(
                                        Button::new("dlg-set")
                                            .label("Set")
                                            .on_click(|_, window, cx| window.close_dialog(cx)),
                                    )
                                    .child(
                                        Button::new("dlg-cancel")
                                            .label("Cancel")
                                            .on_click(|_, window, cx| window.close_dialog(cx)),
                                    ),
                            )
                        });
                    });
                })
            })
        })
        .unwrap();
    std::mem::forget(sub);
}

fn close_dialog_by(cx: &mut TestAppContext, win: AnyWindowHandle, how: &'static str) {
    cx.update_window(win, |_, w, cx| match how {
        "escape" => w.press("escape", cx),
        id => w.click(id, cx),
    })
    .unwrap();
    frame(cx, win);
    cx.run_until_parked();
    frame(cx, win);
}

const CLOSE_PATHS: [&str; 3] = ["dlg-set", "dlg-cancel", "escape"];

#[gpui_kit::test]
fn closing_the_set_variable_dialog_returns_focus_to_the_editor(cx: &mut TestAppContext) {
    use gpui_kit::Focusable as _;
    for how in CLOSE_PATHS {
        let (win, view) = shown(cx, JSON, "application/json");
        dialog_on_event(cx, win, &view);
        let handle = view.read_with(cx, |v, cx| v.editor().read(cx).focus_handle(cx));
        place_caret(cx, &view, 10);
        blur(cx, win);
        choose(cx, win, SET_VARIABLE, &ENTRIES);
        frame(cx, win);
        close_dialog_by(cx, win, how);
        assert!(focused(cx, win, &handle), "editor after {how}");
    }
}

#[gpui_kit::test]
fn closing_the_set_variable_dialog_returns_focus_to_the_rows_body(cx: &mut TestAppContext) {
    for how in CLOSE_PATHS {
        let (win, view) = shown(cx, &long_json(), "application/json");
        dialog_on_event(cx, win, &view);
        cx.update_window(win, |_, w, cx| w.hover(("body-line", 3usize), cx))
            .unwrap();
        frame(cx, win);
        let handle = view.read_with(cx, |v, _| v.body_focus().clone());
        blur(cx, win);
        choose(cx, win, SET_VARIABLE, &ENTRIES);
        frame(cx, win);
        close_dialog_by(cx, win, how);
        assert!(focused(cx, win, &handle), "rows body after {how}");
    }
}
