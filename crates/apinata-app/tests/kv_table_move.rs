//! Moving key/value rows: the drag handle, the drop line, and Alt+Up and Alt+Down.

use crate::support;

use apinata_app::kv_table::{KvEvent, KvRow, KvTable};
use apinata_app::tabs::Side;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Context, Entity, Focusable as _, IntoElement, ParentElement,
    Render, Styled, Subscription, TestAppContext, Window, WindowOptions, div, point, px,
};
use support::*;

pub(crate) struct Host {
    pub(crate) table: Entity<KvTable>,
    pub(crate) events: Vec<Vec<KvRow>>,
    _sub: Subscription,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.table.clone())
    }
}

fn row(name: &str, enabled: bool, fixed: bool) -> KvRow {
    KvRow {
        name: name.into(),
        value: format!("v-{name}"),
        enabled,
        fixed,
        ..KvRow::default()
    }
}

pub(crate) fn host(cx: &mut TestAppContext, rows: Vec<KvRow>) -> (AnyWindowHandle, Entity<Host>) {
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| {
                    let table = cx.new(|cx| KvTable::new(w, cx));
                    table.update(cx, |t, cx| t.load(rows, w, cx));
                    let sub = cx.subscribe(&table, |h: &mut Host, _, e: &KvEvent, _| {
                        let KvEvent::Changed(rows) = e;
                        h.events.push(rows.clone());
                    });
                    Host {
                        table,
                        events: Vec::new(),
                        _sub: sub,
                    }
                })
            })
        })
        .unwrap();
    cx.run_until_parked();
    frame(cx, win);
    (win, view)
}

pub(crate) fn abc() -> Vec<KvRow> {
    ["a", "b", "c"].map(|n| row(n, true, false)).to_vec()
}

fn names(cx: &mut TestAppContext, h: &Entity<Host>) -> Vec<String> {
    h.read_with(cx, |h, cx| {
        h.table
            .read(cx)
            .rows(cx)
            .into_iter()
            .map(|r| r.name)
            .collect()
    })
}

fn emitted(cx: &mut TestAppContext, h: &Entity<Host>) -> usize {
    h.read_with(cx, |h, _| h.events.len())
}

// Drags row `from`'s handle to the upper or lower part of row `onto`.
fn drag(cx: &mut TestAppContext, win: AnyWindowHandle, from: usize, onto: usize, lower: bool) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let start = w.find(("kv-grip", from)).bounds().center();
        let b = w.find(("kv-row", onto)).bounds();
        let y = if lower {
            b.bottom() - px(3.)
        } else {
            b.top() + px(3.)
        };
        w.drag(start, point(b.center().x, y), cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn press(cx: &mut TestAppContext, win: AnyWindowHandle, keys: &str) {
    cx.simulate_keystrokes(win, keys);
    cx.run_until_parked();
}

pub(crate) fn focus_name(cx: &mut TestAppContext, win: AnyWindowHandle, ix: usize) {
    cx.update_window(win, |_, w, cx| w.click(("kv-name", ix), cx))
        .unwrap();
}

fn name_focused(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    h: &Entity<Host>,
    ix: usize,
) -> bool {
    let input = h.read_with(cx, |h, cx| {
        h.table.read(cx).name_input(ix).cloned().unwrap()
    });
    cx.update_window(win, |_, w, cx| {
        input.focus_handle(cx).contains_focused(w, cx)
    })
    .unwrap()
}

#[gpui_kit::test]
fn each_real_row_has_a_handle_but_the_empty_row_does_not(cx: &mut TestAppContext) {
    let (win, _h) = host(cx, abc());
    cx.update_window(win, |_, w, _| {
        for ix in 0..3usize {
            assert!(w.try_find(("kv-grip", ix)).is_some(), "row {ix}");
        }
        assert!(w.try_find(("kv-grip", 3usize)).is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_fixed_row_has_no_handle(cx: &mut TestAppContext) {
    let (win, _h) = host(cx, vec![row("id", true, true), row("a", true, false)]);
    cx.update_window(win, |_, w, _| {
        assert!(w.try_find(("kv-grip", 0usize)).is_none());
        assert!(w.try_find(("kv-grip", 1usize)).is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_handle_slot_keeps_the_columns_and_the_heading_aligned(cx: &mut TestAppContext) {
    let (win, _h) = host(cx, vec![row("id", true, true), row("a", true, false)]);
    cx.update_window(win, |_, w, _| {
        let head = w.find("kv-head-name").bounds().left();
        for ix in 0..3usize {
            assert_eq!(w.find(("kv-name", ix)).bounds().left(), head, "row {ix}");
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn dragging_a_handle_onto_the_lower_half_of_a_row_puts_it_after(cx: &mut TestAppContext) {
    let (win, h) = host(cx, abc());
    drag(cx, win, 0, 1, true);
    assert_eq!(names(cx, &h), ["b", "a", "c"]);
    assert_eq!(emitted(cx, &h), 1, "one edit, so one undo step");
}

#[gpui_kit::test]
fn dragging_onto_the_upper_half_puts_it_before(cx: &mut TestAppContext) {
    let (win, h) = host(cx, abc());
    drag(cx, win, 2, 0, false);
    assert_eq!(names(cx, &h), ["c", "a", "b"]);
}

#[gpui_kit::test]
fn a_drop_that_changes_nothing_emits_nothing(cx: &mut TestAppContext) {
    let (win, h) = host(cx, abc());
    drag(cx, win, 1, 1, true);
    drag(cx, win, 1, 0, true);
    assert_eq!(names(cx, &h), ["a", "b", "c"]);
    assert_eq!(emitted(cx, &h), 0);
}

#[gpui_kit::test]
fn a_dragged_row_keeps_its_text_and_takes_focus(cx: &mut TestAppContext) {
    let (win, h) = host(cx, abc());
    drag(cx, win, 0, 2, true);
    assert_eq!(names(cx, &h), ["b", "c", "a"]);
    assert!(name_focused(cx, win, &h, 2));
    let last = h.read_with(cx, |h, _| h.events.last().unwrap().clone());
    assert_eq!(last[2].value, "v-a");
}

#[gpui_kit::test]
fn a_disabled_row_moves_like_any_row(cx: &mut TestAppContext) {
    let (win, h) = host(
        cx,
        vec![
            row("a", true, false),
            row("b", false, false),
            row("c", true, false),
        ],
    );
    drag(cx, win, 1, 0, false);
    let last = h.read_with(cx, |h, _| h.events.last().unwrap().clone());
    let seen: Vec<_> = last.iter().map(|r| (r.name.as_str(), r.enabled)).collect();
    assert_eq!(seen, [("b", false), ("a", true), ("c", true)]);
}

#[gpui_kit::test]
fn a_fixed_row_has_no_real_neighbour_to_trade_places_with(cx: &mut TestAppContext) {
    let (win, h) = host(cx, vec![row("id", true, true), row("a", true, false)]);
    drag(cx, win, 1, 0, false);
    assert_eq!(names(cx, &h), ["id", "a"]);
    assert_eq!(emitted(cx, &h), 0);
}

// Rows `q1, id (fixed), q2`: the fixed row is derived from the URL, so it stays at index 1.
fn around_a_path_row() -> Vec<KvRow> {
    vec![
        row("q1", true, false),
        row("id", true, true),
        row("q2", true, false),
    ]
}

fn fixed_flags(cx: &mut TestAppContext, h: &Entity<Host>) -> Vec<bool> {
    h.read_with(cx, |h, cx| {
        h.table
            .read(cx)
            .rows(cx)
            .into_iter()
            .map(|r| r.fixed)
            .collect()
    })
}

#[gpui_kit::test]
fn a_drag_across_a_fixed_row_leaves_it_where_it_was(cx: &mut TestAppContext) {
    let (win, h) = host(cx, around_a_path_row());
    drag(cx, win, 0, 2, true);
    assert_eq!(names(cx, &h), ["q2", "id", "q1"]);
    assert_eq!(fixed_flags(cx, &h), [false, true, false]);
    drag(cx, win, 2, 0, false);
    assert_eq!(names(cx, &h), ["q1", "id", "q2"]);
}

#[gpui_kit::test]
fn alt_on_the_row_below_a_fixed_row_moves_past_it(cx: &mut TestAppContext) {
    let (win, h) = host(cx, around_a_path_row());
    focus_name(cx, win, 2);
    press(cx, win, "alt-up");
    assert_eq!(names(cx, &h), ["q2", "id", "q1"]);
    assert_eq!(fixed_flags(cx, &h), [false, true, false]);
    assert!(name_focused(cx, win, &h, 0));
    press(cx, win, "alt-down");
    assert_eq!(names(cx, &h), ["q1", "id", "q2"]);
}

#[gpui_kit::test]
fn a_drop_line_never_marks_a_fixed_row(cx: &mut TestAppContext) {
    let (win, h) = host(cx, around_a_path_row());
    let table = h.read_with(cx, |h, _| h.table.clone());
    table.update(cx, |t, cx| t.hover_drop(0, 1, Some(Side::After), cx));
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("kv-drop-line").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn alt_on_a_fixed_row_does_nothing(cx: &mut TestAppContext) {
    let (win, h) = host(cx, vec![row("id", true, true), row("a", true, false)]);
    cx.update_window(win, |_, w, cx| w.click(("kv-value", 0usize), cx))
        .unwrap();
    press(cx, win, "alt-down");
    assert_eq!(names(cx, &h), ["id", "a"]);
    focus_name(cx, win, 1);
    press(cx, win, "alt-up");
    assert_eq!(names(cx, &h), ["id", "a"]);
    assert_eq!(emitted(cx, &h), 0);
}

#[gpui_kit::test]
fn the_drop_line_shows_only_where_a_drop_would_move_the_row(cx: &mut TestAppContext) {
    let (win, h) = host(cx, abc());
    let table = h.read_with(cx, |h, _| h.table.clone());
    let shown = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find("kv-drop-line").is_some()
        })
        .unwrap()
    };
    assert!(!shown(cx));
    table.update(cx, |t, cx| t.hover_drop(0, 2, Some(Side::After), cx));
    assert!(shown(cx), "after the last row");
    table.update(cx, |t, cx| t.hover_drop(0, 2, None, cx));
    assert!(!shown(cx), "cleared when the pointer leaves");
    table.update(cx, |t, cx| t.hover_drop(1, 1, Some(Side::After), cx));
    assert!(!shown(cx), "on the row itself");
}

#[gpui_kit::test]
fn alt_down_moves_the_focused_row_down_and_keeps_focus_on_it(cx: &mut TestAppContext) {
    let (win, h) = host(cx, abc());
    focus_name(cx, win, 0);
    press(cx, win, "alt-down");
    assert_eq!(names(cx, &h), ["b", "a", "c"]);
    assert!(name_focused(cx, win, &h, 1));
    assert_eq!(emitted(cx, &h), 1);
    assert_eq!(h.read_with(cx, |h, cx| h.table.read(cx).last_typed()), None);
}

// From the Remove button, the move hands focus to the moved row's name, never leaving it on
// the Remove button that now belongs to the neighbour.
#[gpui_kit::test]
fn alt_down_from_remove_moves_the_row_and_focuses_its_name(cx: &mut TestAppContext) {
    let (win, h) = host(cx, abc());
    cx.update_window(win, |_, w, cx| w.click(("kv-value", 0usize), cx))
        .unwrap();
    press(cx, win, "tab");
    assert!(holds_focus(cx, win, ("kv-delete", 0usize)));
    press(cx, win, "alt-down");
    assert_eq!(names(cx, &h), ["b", "a", "c"]);
    assert!(
        name_focused(cx, win, &h, 1),
        "the moved row's name has focus"
    );
}

#[gpui_kit::test]
fn alt_up_moves_the_focused_row_up(cx: &mut TestAppContext) {
    let (win, h) = host(cx, abc());
    focus_name(cx, win, 2);
    press(cx, win, "alt-up");
    assert_eq!(names(cx, &h), ["a", "c", "b"]);
    assert!(name_focused(cx, win, &h, 1));
}

#[gpui_kit::test]
fn alt_works_from_the_value_cell_too(cx: &mut TestAppContext) {
    let (win, h) = host(cx, abc());
    cx.update_window(win, |_, w, cx| w.click(("kv-value", 1usize), cx))
        .unwrap();
    press(cx, win, "alt-up");
    assert_eq!(names(cx, &h), ["b", "a", "c"]);
}

#[gpui_kit::test]
fn alt_at_the_ends_does_nothing_and_never_enters_the_empty_row(cx: &mut TestAppContext) {
    let (win, h) = host(cx, abc());
    focus_name(cx, win, 0);
    press(cx, win, "alt-up");
    focus_name(cx, win, 2);
    press(cx, win, "alt-down");
    assert_eq!(names(cx, &h), ["a", "b", "c"]);
    assert_eq!(emitted(cx, &h), 0);
}

#[gpui_kit::test]
fn a_locked_table_has_no_handles_and_ignores_alt(cx: &mut TestAppContext) {
    let (win, h) = host(cx, abc());
    focus_name(cx, win, 0);
    let table = h.read_with(cx, |h, _| h.table.clone());
    table.update(cx, |t, cx| t.set_locked(true, cx));
    frame(cx, win);
    cx.update_window(win, |_, w, _| {
        assert!(w.try_find(("kv-grip", 0usize)).is_none());
    })
    .unwrap();
    press(cx, win, "alt-down");
    assert_eq!(names(cx, &h), ["a", "b", "c"]);
}

#[gpui_kit::test]
fn the_move_commands_are_registered_with_alt_chords(cx: &mut TestAppContext) {
    let _ = cx;
    let cmds = apinata_app::commands::registry();
    for (id, label, chord) in [
        ("request.move_row_up", "Move row up", "alt-up"),
        ("request.move_row_down", "Move row down", "alt-down"),
    ] {
        let c = cmds.iter().find(|c| c.id == id).expect(id);
        assert_eq!(c.label, label);
        assert_eq!(c.default_chords, [chord]);
    }
}

#[gpui_kit::test]
fn alt_works_with_the_row_checkbox_focused(cx: &mut TestAppContext) {
    let (win, h) = host(cx, abc());
    cx.update_window(win, |_, w, cx| w.click(("kv-enabled", 0usize), cx))
        .unwrap();
    press(cx, win, "alt-down");
    assert_eq!(names(cx, &h), ["b", "a", "c"]);
}

#[gpui_kit::test]
fn the_drag_preview_label_falls_back_when_the_name_is_empty(cx: &mut TestAppContext) {
    let _ = cx;
    assert_eq!(apinata_app::row_move::preview_label(""), "Row");
    assert_eq!(apinata_app::row_move::preview_label("x"), "x");
}

#[gpui_kit::test]
fn focus_follows_a_moved_row_whose_checkbox_was_reached_by_tab(cx: &mut TestAppContext) {
    let (win, h) = host(cx, abc());
    focus_name(cx, win, 0);
    cx.update_window(win, |_, w, cx| {
        w.focus_prev(cx);
        w.render_frame(cx);
        assert_eq!(w.find(("kv-enabled", 0usize)).focused(), Some(true));
    })
    .unwrap();
    press(cx, win, "alt-down");
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        assert_eq!(w.find(("kv-enabled", 1usize)).focused(), Some(true));
        w.press("space", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let last = h.read_with(cx, |h, _| h.events.last().unwrap().clone());
    let seen: Vec<_> = last.iter().map(|r| (r.name.as_str(), r.enabled)).collect();
    assert_eq!(seen, [("b", true), ("a", false), ("c", true)]);
}

#[gpui_kit::test]
fn a_drop_on_the_empty_row_leaves_focus_on_the_dragged_row(cx: &mut TestAppContext) {
    let (win, h) = host(cx, abc());
    drag(cx, win, 0, 3, false);
    assert_eq!(names(cx, &h), ["a", "b", "c"]);
    assert!(name_focused(cx, win, &h, 0));
}

// The history's undo reaches the table as a load of the earlier rows.
fn load_rows(cx: &mut TestAppContext, win: AnyWindowHandle, h: &Entity<Host>, rows: Vec<KvRow>) {
    let table = h.read_with(cx, |h, _| h.table.clone());
    cx.update_window(win, |_, w, cx| {
        table.update(cx, |t, cx| t.load(rows, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn undoing_a_move_across_a_path_row_keeps_focus_on_the_moved_row(cx: &mut TestAppContext) {
    let (win, h) = host(cx, around_a_path_row());
    focus_name(cx, win, 2);
    press(cx, win, "alt-up");
    assert_eq!(names(cx, &h), ["q2", "id", "q1"]);
    assert!(name_focused(cx, win, &h, 0));
    load_rows(cx, win, &h, around_a_path_row());
    assert_eq!(names(cx, &h), ["q1", "id", "q2"]);
    assert!(name_focused(cx, win, &h, 2));
}
