//! Keyboard navigation of the rows-mode response body: a current row moved by the arrow,
//! page and home/end keys, kept in view, and followed by the value-actions button.

use crate::support;

use std::sync::Arc;

use apinata_app::body_view::{BodyView, Shown};
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, WindowOptions};
use support::*;

const BUTTON: &str = "body-value-actions";

fn long_json() -> String {
    let items: Vec<String> = (0..30_000).map(|i| format!("{{\"id\":{i}}}")).collect();
    format!("[{}]", items.join(","))
}

// A rows-mode body with the real keymap, focus on the body and one frame drawn.
fn rows(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<BodyView>) {
    open(
        cx,
        Response::new(200)
            .with_header("content-type", "application/json")
            .with_text_body(long_json()),
    )
}

fn open(cx: &mut TestAppContext, response: Response) -> (AnyWindowHandle, Entity<BodyView>) {
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| BodyView::new(w, cx))
            })
        })
        .unwrap();
    view.update(cx, |v, _| v.set_assert_offered(true));
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
    assert!(matches!(
        view.read_with(cx, |v, _| v.shown()),
        Shown::Lines(_) | Shown::Binary(_)
    ));
    cx.update_window(win, |_, w, cx| {
        let focus = view.read(cx).body_focus().clone();
        focus.focus(w, cx);
    })
    .unwrap();
    frame(cx, win);
    (win, view)
}

fn press(cx: &mut TestAppContext, win: AnyWindowHandle, keys: &str) {
    cx.simulate_keystrokes(win, keys);
    frame(cx, win);
    // The deferred scroll lands on the paint after the key, and the button follows it.
    frame(cx, win);
}

fn current(cx: &mut TestAppContext, view: &Entity<BodyView>) -> Option<usize> {
    view.read_with(cx, |v, _| v.current_row())
}

fn first_visible(cx: &mut TestAppContext, view: &Entity<BodyView>) -> usize {
    view.read_with(cx, |v, _| v.visible_row_range().unwrap().start)
}

fn page(cx: &mut TestAppContext, view: &Entity<BodyView>) -> usize {
    view.read_with(cx, |v, _| v.visible_row_range().unwrap().len())
}

#[gpui_kit::test]
fn down_and_up_move_the_current_row_by_one(cx: &mut TestAppContext) {
    let (win, view) = rows(cx);
    assert_eq!(current(cx, &view), None, "nothing is current before a key");
    press(cx, win, "down");
    let start = current(cx, &view).unwrap();
    press(cx, win, "down");
    assert_eq!(current(cx, &view), Some(start + 1));
    press(cx, win, "up");
    assert_eq!(current(cx, &view), Some(start));
}

#[gpui_kit::test]
fn the_current_row_stays_in_view_when_it_walks_off_the_bottom(cx: &mut TestAppContext) {
    let (win, view) = rows(cx);
    let rows_on_screen = page(cx, &view);
    for _ in 0..rows_on_screen + 5 {
        press(cx, win, "down");
    }
    let row = current(cx, &view).unwrap();
    let range = view.read_with(cx, |v, _| v.visible_row_range().unwrap());
    assert!(range.contains(&row), "{row} is outside {range:?}");
    assert!(view.read_with(cx, |v, _| v.rows_scroll_offset().y) < gpui_kit::px(0.));
}

#[gpui_kit::test]
fn page_down_and_page_up_move_by_the_visible_rows(cx: &mut TestAppContext) {
    let (win, view) = rows(cx);
    let n = page(cx, &view);
    press(cx, win, "down");
    let start = current(cx, &view).unwrap();
    press(cx, win, "pagedown");
    assert_eq!(current(cx, &view), Some(start + n));
    press(cx, win, "pageup");
    assert_eq!(current(cx, &view), Some(start));
}

#[gpui_kit::test]
fn end_and_home_go_to_the_last_and_first_rows(cx: &mut TestAppContext) {
    let (win, view) = rows(cx);
    let last = view.read_with(cx, |v, _| v.shown_rows().len()) - 1;
    for (end, home) in [("end", "home"), ("ctrl-end", "ctrl-home")] {
        press(cx, win, end);
        assert_eq!(current(cx, &view), Some(last), "{end}");
        let range = view.read_with(cx, |v, _| v.visible_row_range().unwrap());
        assert!(range.contains(&last), "{end} shows the last row");
        press(cx, win, home);
        assert_eq!(current(cx, &view), Some(0), "{home}");
        assert_eq!(first_visible(cx, &view), 0, "{home} scrolls to the top");
    }
}

#[gpui_kit::test]
fn the_keys_stop_at_the_ends(cx: &mut TestAppContext) {
    let (win, view) = rows(cx);
    press(cx, win, "home");
    press(cx, win, "up");
    assert_eq!(current(cx, &view), Some(0));
    press(cx, win, "end");
    let last = current(cx, &view).unwrap();
    press(cx, win, "down");
    press(cx, win, "pagedown");
    assert_eq!(current(cx, &view), Some(last));
}

#[gpui_kit::test]
fn the_value_button_follows_the_current_row(cx: &mut TestAppContext) {
    let (win, view) = rows(cx);
    assert!(
        cx.update_window(win, |_, w, _| w.try_find(BUTTON).is_none())
            .unwrap()
    );
    press(cx, win, "down");
    press(cx, win, "down");
    assert!(
        cx.update_window(win, |_, w, _| w.try_find(BUTTON).is_some())
            .unwrap()
    );
    let row = current(cx, &view).unwrap();
    assert_eq!(view.read_with(cx, |v, _| v.hovered_row()), Some(row));
}

#[gpui_kit::test]
fn hovering_a_row_still_moves_the_button(cx: &mut TestAppContext) {
    let (win, view) = rows(cx);
    press(cx, win, "down");
    cx.update_window(win, |_, w, cx| w.hover(("body-line", 4usize), cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(view.read_with(cx, |v, _| v.hovered_row()), Some(4));
}

#[gpui_kit::test]
fn the_keys_do_nothing_while_the_find_field_has_focus(cx: &mut TestAppContext) {
    let (win, view) = rows(cx);
    cx.update_window(win, |_, w, cx| view.update(cx, |v, cx| v.open_find(w, cx)))
        .unwrap();
    frame(cx, win);
    press(cx, win, "down");
    press(cx, win, "pagedown");
    press(cx, win, "end");
    assert_eq!(current(cx, &view), None);
}

#[gpui_kit::test]
fn a_new_body_clears_the_current_row(cx: &mut TestAppContext) {
    let (win, view) = rows(cx);
    press(cx, win, "end");
    assert!(current(cx, &view).is_some());
    let response = Response::new(200)
        .with_header("content-type", "application/json")
        .with_text_body(long_json());
    let r = Arc::new(RequestResult::from_parts(
        ip("b.yml"),
        "b",
        Outcome::Passed,
        Some(response),
        Vec::new(),
    ));
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(r), w, cx))
    })
    .unwrap();
    frame(cx, win);
    assert_eq!(current(cx, &view), None);
}

#[gpui_kit::test]
fn the_first_up_or_down_selects_the_top_visible_row(cx: &mut TestAppContext) {
    for key in ["down", "up"] {
        let (win, view) = rows(cx);
        let top = first_visible(cx, &view);
        press(cx, win, key);
        assert_eq!(current(cx, &view), Some(top), "{key}");
    }
}

#[gpui_kit::test]
fn the_current_row_lands_on_the_edge_it_scrolled_towards(cx: &mut TestAppContext) {
    let (win, view) = rows(cx);
    let n = page(cx, &view);
    for _ in 0..n + 6 {
        press(cx, win, "down");
    }
    let (row, range) = (
        current(cx, &view).unwrap(),
        view.read_with(cx, |v, _| v.visible_row_range().unwrap()),
    );
    assert_eq!(row + 1, range.end, "{row} is the last row of {range:?}");
    for _ in 0..n + 6 {
        press(cx, win, "up");
    }
    let row = current(cx, &view).unwrap();
    assert_eq!(row, first_visible(cx, &view), "up ends at the top edge");
}

#[gpui_kit::test]
fn the_current_row_survives_a_wrap_re_cut_at_the_same_text(cx: &mut TestAppContext) {
    let text = (0..20_500)
        .map(|i| format!("line {i} {}", "w".repeat(300)))
        .collect::<Vec<_>>()
        .join("\n");
    let (win, view) = open(
        cx,
        Response::new(200)
            .with_header("content-type", "text/plain")
            .with_text_body(text),
    );
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_wrap(true, w, cx))
    })
    .unwrap();
    frame(cx, win);
    frame(cx, win);
    press(cx, win, "end");
    let before = view.read_with(cx, |v, _| v.shown_rows().len());
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_wrap(false, w, cx))
    })
    .unwrap();
    frame(cx, win);
    frame(cx, win);
    let after = view.read_with(cx, |v, _| v.shown_rows().len());
    assert_ne!(before, after, "the cut changed");
    // The last wrapped row sat inside the last line, which the new cut holds in its last row.
    assert_eq!(current(cx, &view), Some(after - 1));
    assert_eq!(view.read_with(cx, |v, _| v.hovered_row()), Some(after - 1));
}

#[gpui_kit::test]
fn the_hex_view_navigates_too(cx: &mut TestAppContext) {
    let bytes: Vec<u8> = (0..4000).map(|i| (i % 251) as u8).collect();
    let (win, view) = open(
        cx,
        Response::new(200)
            .with_header("content-type", "application/octet-stream")
            .with_body(bytes, ""),
    );
    assert!(matches!(
        view.read_with(cx, |v, _| v.shown()),
        Shown::Binary(_)
    ));
    press(cx, win, "down");
    press(cx, win, "down");
    assert_eq!(current(cx, &view), Some(1));
    press(cx, win, "end");
    let last = view.read_with(cx, |v, _| v.shown_rows().len()) - 1;
    assert_eq!(current(cx, &view), Some(last));
}

#[gpui_kit::test]
fn find_next_leaves_the_keyboard_row_alone(cx: &mut TestAppContext) {
    let (win, view) = rows(cx);
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| {
            v.open_find(w, cx);
            v.set_find_query("id", w, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        let focus = view.read(cx).body_focus().clone();
        focus.focus(w, cx);
    })
    .unwrap();
    press(cx, win, "down");
    press(cx, win, "down");
    let row = current(cx, &view).unwrap();
    cx.update_window(win, |_, w, cx| view.update(cx, |v, cx| v.find_next(w, cx)))
        .unwrap();
    frame(cx, win);
    assert_eq!(current(cx, &view), Some(row), "find does not move it");
    assert!(!view.read_with(cx, |v, cx| v.find_label(cx)).is_empty());
}

fn long_lines() -> Response {
    let text = (0..20_500)
        .map(|i| format!("line {i} {}", "w".repeat(300)))
        .collect::<Vec<_>>()
        .join("\n");
    Response::new(200)
        .with_header("content-type", "text/plain")
        .with_text_body(text)
}

fn wrap(cx: &mut TestAppContext, win: AnyWindowHandle, view: &Entity<BodyView>, on: bool) {
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_wrap(on, w, cx))
    })
    .unwrap();
    frame(cx, win);
    frame(cx, win);
}

#[gpui_kit::test]
fn a_continuation_row_maps_to_its_line_across_a_re_cut(cx: &mut TestAppContext) {
    let (win, view) = open(cx, long_lines());
    wrap(cx, win, &view, true);
    press(cx, win, "down");
    press(cx, win, "down");
    let row = current(cx, &view).unwrap();
    assert_eq!(row, 1);
    assert_eq!(
        view.read_with(cx, |v, _| v.line_numbers()[row]),
        0,
        "a continuation"
    );
    wrap(cx, win, &view, false);
    assert_eq!(current(cx, &view), Some(0), "the first line's only row");
}

#[gpui_kit::test]
fn a_key_pressed_during_a_re_cut_wins_over_the_old_place(cx: &mut TestAppContext) {
    let (win, view) = open(cx, long_lines());
    wrap(cx, win, &view, true);
    press(cx, win, "down");
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_wrap(false, w, cx));
        // The cut is under way and has not landed; the key arrives in between.
        w.render_frame(cx);
        w.dispatch_action(
            gpui_kit::Action::boxed_clone(&apinata_app::body_find::RowLast),
            cx,
        );
    })
    .unwrap();
    frame(cx, win);
    frame(cx, win);
    let last = view.read_with(cx, |v, _| v.shown_rows().len()) - 1;
    assert_eq!(current(cx, &view), Some(last));
}

#[gpui_kit::test]
fn clicking_a_row_makes_it_the_current_row(cx: &mut TestAppContext) {
    let (win, view) = rows(cx);
    click_with(cx, win, ("body-line", 3usize), Default::default());
    assert_eq!(current(cx, &view), Some(3));
    assert_eq!(view.read_with(cx, |v, _| v.hovered_row()), Some(3));
    press(cx, win, "down");
    assert_eq!(current(cx, &view), Some(4));
}

#[gpui_kit::test]
fn clicking_a_hex_row_makes_it_the_current_row(cx: &mut TestAppContext) {
    let bytes: Vec<u8> = (0..4000).map(|i| (i % 251) as u8).collect();
    let (win, view) = open(
        cx,
        Response::new(200)
            .with_header("content-type", "application/octet-stream")
            .with_body(bytes, ""),
    );
    click_with(cx, win, ("body-line", 2usize), Default::default());
    assert_eq!(current(cx, &view), Some(2));
    press(cx, win, "down");
    assert_eq!(current(cx, &view), Some(3));
}

#[gpui_kit::test]
fn hovering_after_a_click_still_moves_the_hovered_row(cx: &mut TestAppContext) {
    let (win, view) = rows(cx);
    click_with(cx, win, ("body-line", 3usize), Default::default());
    cx.update_window(win, |_, w, cx| w.hover(("body-line", 6usize), cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(view.read_with(cx, |v, _| v.hovered_row()), Some(6));
    assert_eq!(current(cx, &view), Some(3));
}
