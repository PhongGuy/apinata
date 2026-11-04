//! Moving assertion rows: the drag handle and Alt+Up and Alt+Down.

use crate::support;

use apinata_app::assertions_section::AssertionsSection;
use apinata_app::command::Edit;
use apinata_core::Assertion;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, Focusable as _, TestAppContext, point, px,
};
use support::*;

type Host = SectionHost<AssertionsSection>;

fn three(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Host>, Entity<AssertionsSection>) {
    let mut b = Assertion::new("b", "eq", Some("2".into()));
    b.set_enabled(false);
    let base = vec![
        Assertion::new("a", "eq", Some("1".into())),
        b,
        Assertion::new("c", "isTruthy", None),
    ];
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let (win, host) = host_section(cx, AssertionsSection::new);
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| s.update(cx, |s, cx| s.load(&base, w, cx)))
        .unwrap();
    cx.run_until_parked();
    frame(cx, win);
    (win, host, s)
}

fn edits(cx: &mut TestAppContext, host: &Entity<Host>) -> Vec<Vec<Assertion>> {
    host.read_with(cx, |h, _| {
        h.edits
            .iter()
            .filter_map(|e| match e {
                Edit::Assertions(a) => Some(a.clone()),
                _ => None,
            })
            .collect()
    })
}

fn order(cx: &mut TestAppContext, s: &Entity<AssertionsSection>) -> Vec<String> {
    s.read_with(cx, |s, cx| {
        s.rows(cx).into_iter().map(|r| r.expression).collect()
    })
}

fn focused_row(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    s: &Entity<AssertionsSection>,
) -> Option<usize> {
    let s = s.clone();
    cx.update_window(win, |_, w, cx| {
        (0..3).find(|&i| {
            let input = s.read(cx).expression_input(i).cloned().unwrap();
            input.focus_handle(cx).contains_focused(w, cx)
        })
    })
    .unwrap()
}

#[gpui_kit::test]
fn each_real_row_has_a_handle_and_the_empty_row_does_not(cx: &mut TestAppContext) {
    let (win, _h, _s) = three(cx);
    cx.update_window(win, |_, w, _| {
        for ix in 0..3usize {
            assert!(w.try_find(("as-grip", ix)).is_some(), "row {ix}");
        }
        assert!(w.try_find(("as-grip", 3usize)).is_none());
        let head = w.find("as-head-expr").bounds().left();
        assert_eq!(w.find(("as-expr", 0usize)).bounds().left(), head);
        assert_eq!(w.find(("as-expr", 3usize)).bounds().left(), head);
    })
    .unwrap();
}

#[gpui_kit::test]
fn dragging_a_handle_moves_the_row_and_keeps_its_flags(cx: &mut TestAppContext) {
    let (win, host, s) = three(cx);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let start = w.find(("as-grip", 1usize)).bounds().center();
        let b = w.find(("as-row", 0usize)).bounds();
        w.drag(start, point(b.center().x, b.top() + px(3.)), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(order(cx, &s), ["b", "a", "c"]);
    let all = edits(cx, &host);
    assert_eq!(all.len(), 1, "one edit, so one undo step");
    assert!(!all[0][0].enabled(), "the disabled row moved like any row");
    assert_eq!(all[0][0].expression(), "b");
    assert_eq!(focused_row(cx, win, &s), Some(0));
}

#[gpui_kit::test]
fn dropping_on_the_lower_half_puts_the_row_after(cx: &mut TestAppContext) {
    let (win, _host, s) = three(cx);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let start = w.find(("as-grip", 0usize)).bounds().center();
        let b = w.find(("as-row", 2usize)).bounds();
        w.drag(start, point(b.center().x, b.bottom() - px(3.)), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(order(cx, &s), ["b", "c", "a"]);
}

#[gpui_kit::test]
fn the_drop_line_shows_where_a_drop_would_move_the_row(cx: &mut TestAppContext) {
    let (win, _h, s) = three(cx);
    let shown = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find("as-drop-line").is_some()
        })
        .unwrap()
    };
    assert!(!shown(cx));
    s.update(cx, |s, cx| {
        s.hover_drop(0, 2, Some(apinata_app::tabs::Side::After), cx)
    });
    assert!(shown(cx));
    s.update(cx, |s, cx| s.hover_drop(0, 2, None, cx));
    assert!(!shown(cx));
}

#[gpui_kit::test]
fn alt_keys_move_the_focused_row_and_stop_at_the_ends(cx: &mut TestAppContext) {
    let (win, host, s) = three(cx);
    cx.update_window(win, |_, w, cx| w.click(("as-expr", 0usize), cx))
        .unwrap();
    cx.simulate_keystrokes(win, "alt-up");
    cx.run_until_parked();
    assert_eq!(order(cx, &s), ["a", "b", "c"]);
    cx.simulate_keystrokes(win, "alt-down");
    cx.run_until_parked();
    assert_eq!(order(cx, &s), ["b", "a", "c"]);
    assert_eq!(focused_row(cx, win, &s), Some(1));
    assert_eq!(edits(cx, &host).len(), 1);
    cx.simulate_keystrokes(win, "alt-down");
    cx.simulate_keystrokes(win, "alt-down");
    cx.run_until_parked();
    assert_eq!(order(cx, &s), ["b", "c", "a"], "never into the empty row");
}

#[gpui_kit::test]
fn alt_works_from_the_value_cell(cx: &mut TestAppContext) {
    let (win, _h, s) = three(cx);
    cx.update_window(win, |_, w, cx| w.click(("as-value", 1usize), cx))
        .unwrap();
    cx.simulate_keystrokes(win, "alt-up");
    cx.run_until_parked();
    assert_eq!(order(cx, &s), ["b", "a", "c"]);
}

#[gpui_kit::test]
fn a_load_that_shows_the_move_undone_takes_focus_with_the_row(cx: &mut TestAppContext) {
    let (win, _host, s) = three(cx);
    cx.update_window(win, |_, w, cx| w.click(("as-expr", 0usize), cx))
        .unwrap();
    cx.simulate_keystrokes(win, "alt-down");
    cx.run_until_parked();
    assert_eq!(focused_row(cx, win, &s), Some(1));
    let original = vec![
        Assertion::new("a", "eq", Some("1".into())),
        {
            let mut b = Assertion::new("b", "eq", Some("2".into()));
            b.set_enabled(false);
            b
        },
        Assertion::new("c", "isTruthy", None),
    ];
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&original, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(order(cx, &s), ["a", "b", "c"]);
    assert_eq!(focused_row(cx, win, &s), Some(0));
}

#[gpui_kit::test]
fn alt_works_with_the_row_checkbox_focused(cx: &mut TestAppContext) {
    let (win, _h, s) = three(cx);
    cx.update_window(win, |_, w, cx| w.click(("as-enabled", 0usize), cx))
        .unwrap();
    cx.simulate_keystrokes(win, "alt-down");
    cx.run_until_parked();
    assert_eq!(order(cx, &s), ["b", "a", "c"]);
}

#[gpui_kit::test]
fn a_drop_on_the_empty_row_leaves_focus_on_the_dragged_row(cx: &mut TestAppContext) {
    let (win, _h, s) = three(cx);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let start = w.find(("as-grip", 0usize)).bounds().center();
        let b = w.find(("as-row", 3usize)).bounds();
        w.drag(start, b.center(), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(order(cx, &s), ["a", "b", "c"]);
    assert_eq!(focused_row(cx, win, &s), Some(0));
}

#[gpui_kit::test]
fn focus_follows_a_row_moved_from_a_clicked_checkbox(cx: &mut TestAppContext) {
    let (win, _h, s) = three(cx);
    cx.update_window(win, |_, w, cx| w.click(("as-enabled", 2usize), cx))
        .unwrap();
    cx.simulate_keystrokes(win, "alt-up");
    cx.run_until_parked();
    frame(cx, win);
    cx.simulate_keystrokes(win, "alt-up");
    cx.run_until_parked();
    assert_eq!(order(cx, &s), ["c", "a", "b"]);
}

#[gpui_kit::test]
fn alt_works_with_the_operator_select_focused(cx: &mut TestAppContext) {
    let (win, _h, s) = three(cx);
    cx.update_window(win, |_, w, cx| w.click(("as-expr", 1usize), cx))
        .unwrap();
    cx.simulate_keystrokes(win, "tab");
    cx.run_until_parked();
    cx.simulate_keystrokes(win, "alt-up");
    cx.run_until_parked();
    assert_eq!(order(cx, &s), ["b", "a", "c"]);
}

#[gpui_kit::test]
fn the_delete_button_stays_shown_while_only_the_checkbox_has_focus(cx: &mut TestAppContext) {
    let (win, _h, _s) = three(cx);
    let shown = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, _| {
            w.within(("as-row", 0usize)).find("kv-action").visible()
        })
        .unwrap()
    };
    assert!(!shown(cx));
    // The click leaves the pointer over the row, so move it off before looking.
    cx.update_window(win, |_, w, cx| {
        w.click(("as-enabled", 0usize), cx);
        w.hover(("as-row", 2usize), cx);
    })
    .unwrap();
    frame(cx, win);
    assert!(shown(cx));
}

/// Load `rows` into the section, as a reload from the snapshot shows them.
fn reload(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    s: &Entity<AssertionsSection>,
    rows: Vec<Assertion>,
) {
    cx.update_window(win, |_, w, cx| s.update(cx, |s, cx| s.load(&rows, w, cx)))
        .unwrap();
    cx.run_until_parked();
}

// A reload that drops the row whose checkbox holds focus: focus goes to the expression of the
// row now in that slot, as a delete sends it.
#[gpui_kit::test]
fn a_reload_dropping_the_row_whose_checkbox_holds_focus_focuses_the_next_expression(
    cx: &mut TestAppContext,
) {
    let (win, _h, s) = three(cx);
    cx.update_window(win, |_, w, cx| w.click(("as-enabled", 1usize), cx))
        .unwrap();
    cx.run_until_parked();
    let rows = vec![
        Assertion::new("a", "eq", Some("1".into())),
        Assertion::new("c", "isTruthy", None),
    ];
    reload(cx, win, &s, rows);
    assert_eq!(order(cx, &s), ["a", "c"]);
    assert!(holds_focus(cx, win, ("as-expr", 1usize)));
}

// A reload that drops another row while a later row's checkbox holds focus: that row survives
// one slot up, and its checkbox keeps focus there.
#[gpui_kit::test]
fn a_reload_keeps_focus_on_a_surviving_rows_checkbox(cx: &mut TestAppContext) {
    let (win, _h, s) = three(cx);
    cx.update_window(win, |_, w, cx| w.click(("as-enabled", 2usize), cx))
        .unwrap();
    cx.run_until_parked();
    // The click turned c off; the reload echoes that.
    let mut c = Assertion::new("c", "isTruthy", None);
    c.set_enabled(false);
    reload(
        cx,
        win,
        &s,
        vec![Assertion::new("a", "eq", Some("1".into())), c],
    );
    assert_eq!(order(cx, &s), ["a", "c"]);
    assert!(holds_focus(cx, win, ("as-enabled", 1usize)));
}
