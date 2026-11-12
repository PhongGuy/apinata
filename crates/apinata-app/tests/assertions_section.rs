use crate::support;

use apinata_app::assertions_section::{
    AssertionRow, AssertionsSection, OPERATORS, assertion_rows, assertions_from, is_unary,
};
use apinata_app::command::Edit;
use apinata_app::kv_grid::KvWidth;
use apinata_app::style::{KV_CHECK_WIDTH, OPERATOR_SELECT_WIDTH};
use apinata_app::style::{ROW_HEIGHT, at_zoom};
use apinata_core::{Assertion, RequestFile};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, Focusable as _, TestAppContext};
use support::*;

fn new(e: &str, op: &str, v: Option<&str>, enabled: bool) -> Assertion {
    let mut a = Assertion::new(e, op, v.map(str::to_owned));
    a.set_enabled(enabled);
    a
}

fn section(
    cx: &mut TestAppContext,
    base: Vec<Assertion>,
) -> (
    AnyWindowHandle,
    Entity<SectionHost<AssertionsSection>>,
    Entity<AssertionsSection>,
) {
    let (win, host) = host_section(cx, AssertionsSection::new);
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| s.update(cx, |s, cx| s.load(&base, w, cx)))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    (win, host, s)
}

fn last(cx: &mut TestAppContext, host: &Entity<SectionHost<AssertionsSection>>) -> Vec<Assertion> {
    match host.read_with(cx, |h, _| h.edits.last().cloned()) {
        Some(Edit::Assertions(a)) => a,
        _ => panic!("no assertions edit"),
    }
}

fn click(cx: &mut TestAppContext, win: AnyWindowHandle, id: impl Into<gpui_kit::ElementId>) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(id, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

#[test]
fn bruno_operators_and_their_arity() {
    assert_eq!(&OPERATORS[..3], &["eq", "neq", "gt"]);
    assert_eq!(OPERATORS.iter().filter(|op| is_unary(op)).count(), 12);
    assert!(is_unary("isTruthy") && is_unary("isArray"));
    assert!(!is_unary("eq") && !is_unary("between") && !is_unary("custom"));
}

#[test]
fn unchanged_rows_keep_their_entry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.yml");
    let yaml = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: http://x\nruntime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n      description: kept\n";
    std::fs::write(&path, yaml).unwrap();
    let base = RequestFile::load(&path).unwrap().assertions();
    let mut rows = assertion_rows(&base);
    let want = AssertionRow {
        expression: "res.status".into(),
        operator: "eq".into(),
        value: "200".into(),
        enabled: true,
        source: Some(0),
    };
    assert_eq!(rows[0], want);
    rows[0].enabled = false;
    let out = assertions_from(&base, &rows);
    assert_eq!(
        (out[0].description(), out[0].enabled()),
        (Some("kept"), false)
    );
    rows[0].value = "500".into();
    let out = assertions_from(&base, &rows);
    assert_eq!(
        (
            out[0].operator(),
            out[0].value(),
            out[0].description(),
            out[0].enabled()
        ),
        ("eq", Some("500"), Some("kept"), false)
    );
    rows.push(AssertionRow {
        expression: "res.body.ok".into(),
        operator: "isTruthy".into(),
        value: "x".into(),
        enabled: true,
        source: None,
    });
    assert_eq!(
        assertions_from(&base, &rows)[1],
        new("res.body.ok", "isTruthy", None, true)
    );
}

#[gpui_kit::test]
fn type_toggle_and_delete_through_the_empty_row(cx: &mut TestAppContext) {
    let (win, host, _s) = section(cx, vec![]);
    let present = |cx: &mut TestAppContext, id: (&'static str, usize)| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find(id).is_some()
        })
        .unwrap()
    };
    assert!(present(cx, ("as-expr", 0)) && !present(cx, ("as-delete", 0)));
    assert!(!present(cx, ("as-enabled", 0)));
    assert!(
        cx.update_window(win, |_, w, _| w.try_find("as-add").is_none())
            .unwrap()
    );
    type_at_end(cx, win, None, ("as-expr", 0usize), "res.status");
    cx.run_until_parked();
    assert_eq!(
        last(cx, &host),
        vec![new("res.status", "eq", Some(""), true)]
    );
    type_at_end(cx, win, None, ("as-value", 0usize), "200");
    cx.run_until_parked();
    assert_eq!(
        last(cx, &host),
        vec![new("res.status", "eq", Some("200"), true)]
    );
    assert!(present(cx, ("as-expr", 1)) && !present(cx, ("as-delete", 1)));
    click(cx, win, ("as-enabled", 0usize));
    assert_eq!(
        last(cx, &host),
        vec![new("res.status", "eq", Some("200"), false)]
    );
    click(cx, win, ("as-delete", 0usize));
    assert!(last(cx, &host).is_empty());
}

#[gpui_kit::test]
fn choosing_an_operator_on_the_empty_row_makes_a_row(cx: &mut TestAppContext) {
    let (win, host, s) = section(cx, vec![new("res.status", "eq", Some("200"), true)]);
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.set_operator(1, "isTruthy", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        last(cx, &host),
        vec![
            new("res.status", "eq", Some("200"), true),
            new("", "isTruthy", None, true)
        ]
    );
}

// The row's Remove is the one Tab stop between its value and the empty row.
#[gpui_kit::test]
fn tab_from_the_last_value_reaches_remove_then_the_empty_row(cx: &mut TestAppContext) {
    let (win, _host, s) = section(cx, vec![new("res.status", "eq", Some("200"), true)]);
    let blank = s
        .read_with(cx, |s, _| s.expression_input(1).cloned())
        .unwrap();
    cx.update_window(win, |_, w, cx| w.click(("as-value", 0usize), cx))
        .unwrap();
    cx.simulate_keystrokes(win, "tab");
    cx.run_until_parked();
    assert!(holds_focus(cx, win, ("as-delete", 0usize)));
    cx.simulate_keystrokes(win, "tab");
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        assert!(blank.focus_handle(cx).is_focused(w))
    })
    .unwrap();
}

// A real row's checkbox is its first Tab stop: Remove leads to the next row's checkbox,
// then its expression.
#[gpui_kit::test]
fn tab_from_a_rows_remove_reaches_the_next_rows_checkbox_then_its_expression(
    cx: &mut TestAppContext,
) {
    let (win, _host, s) = section(cx, asserts("ab"));
    let second = s
        .read_with(cx, |s, _| s.expression_input(1).cloned())
        .unwrap();
    cx.update_window(win, |_, w, cx| w.click(("as-value", 0usize), cx))
        .unwrap();
    cx.simulate_keystrokes(win, "tab");
    cx.run_until_parked();
    assert!(holds_focus(cx, win, ("as-delete", 0usize)));
    cx.simulate_keystrokes(win, "tab");
    cx.run_until_parked();
    assert!(holds_focus(cx, win, ("as-enabled", 1usize)));
    cx.simulate_keystrokes(win, "tab");
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        assert!(second.focus_handle(cx).is_focused(w))
    })
    .unwrap();
}

// Shift+Tab from a row's expression lands on its checkbox.
#[gpui_kit::test]
fn shift_tab_from_an_expression_reaches_its_rows_checkbox(cx: &mut TestAppContext) {
    let (win, _host, _s) = section(cx, asserts("ab"));
    cx.update_window(win, |_, w, cx| w.click(("as-expr", 1usize), cx))
        .unwrap();
    cx.simulate_keystrokes(win, "shift-tab");
    cx.run_until_parked();
    assert!(holds_focus(cx, win, ("as-enabled", 1usize)));
}

#[gpui_kit::test]
fn the_empty_assertion_row_lines_up_with_the_real_rows(cx: &mut TestAppContext) {
    let (win, _host, _s) = section(cx, vec![new("res.status", "eq", Some("200"), true)]);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        for id in ["as-expr", "as-value"] {
            let real = w.find((id, 0usize)).bounds();
            let blank = w.find((id, 1usize)).bounds();
            assert_eq!(
                (blank.left(), blank.size.width),
                (real.left(), real.size.width),
                "{id}"
            );
        }
    })
    .unwrap();
    // The checkbox and delete slots only hold space; nothing in them is shown or clickable.
    let hidden = cx
        .update_window(win, |_, w, _| {
            [("as-slot-enabled", 1usize), ("as-slot-delete", 1)].map(|id| !w.find(id).visible())
        })
        .unwrap();
    assert_eq!(hidden, [true, true]);
}

#[gpui_kit::test]
fn a_unary_operator_hides_the_value(cx: &mut TestAppContext) {
    let (win, host, s) = section(cx, vec![new("res.body", "eq", Some("1"), true)]);
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.set_operator(0, "isEmpty", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        last(cx, &host),
        vec![new("res.body", "isEmpty", None, true)]
    );
    let found = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            (
                w.try_find(("as-value", 0usize)).is_some(),
                w.try_find(("as-op", 0usize)).is_some(),
            )
        })
        .unwrap();
    assert_eq!(found, (false, true));
}

#[gpui_kit::test]
fn an_unknown_operator_from_the_file_is_kept(cx: &mut TestAppContext) {
    let (win, host, s) = section(cx, vec![new("res.body", "custom", Some("x"), true)]);
    assert_eq!(s.read_with(cx, |s, cx| s.rows(cx))[0].operator, "custom");
    click(cx, win, ("as-enabled", 0usize));
    assert_eq!(
        last(cx, &host),
        vec![new("res.body", "custom", Some("x"), false)]
    );
}

#[gpui_kit::test]
fn load_keeps_the_caret_when_the_echo_matches(cx: &mut TestAppContext) {
    let (win, _host, s) = section(cx, vec![new("res.status", "eq", Some("200"), true)]);
    type_at_end(cx, win, None, ("as-expr", 0usize), "X");
    cx.update_window(win, |_, w, cx| w.press("left", cx))
        .unwrap();
    let input = s
        .read_with(cx, |s, _| s.expression_input(0).cloned())
        .unwrap();
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| {
            s.load(&[new("res.statusX", "eq", Some("200"), true)], w, cx)
        })
    })
    .unwrap();
    assert_eq!(input.read_with(cx, |i, _| i.cursor()), "res.status".len());
}

#[test]
fn a_toggle_keeps_a_missing_value_missing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.yml");
    let yaml = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: http://x\nruntime:\n  assertions:\n    - expression: res.status\n      operator: eq\n    - expression: res.body\n      operator: custom\n";
    std::fs::write(&path, yaml).unwrap();
    let base = RequestFile::load(&path).unwrap().assertions();
    let mut rows = assertion_rows(&base);
    rows.iter_mut().for_each(|r| r.enabled = false);
    let out = assertions_from(&base, &rows);
    assert_eq!((out[0].value(), out[1].value()), (None, None));
    assert_eq!((out[0].enabled(), out[1].enabled()), (false, false));
}

#[gpui_kit::test]
fn assertion_rows_match_the_key_value_rows(cx: &mut TestAppContext) {
    let (win, _host, _s) = section(
        cx,
        vec![
            new("res.status", "eq", Some("200"), true),
            new("res.body", "isDefined", None, false),
        ],
    );
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let first = w.find(("as-row", 0usize)).bounds();
        assert_eq!(first.size.height, at_zoom(ROW_HEIGHT, 1.0));
        assert_eq!(
            w.find(("as-row", 1usize)).bounds().size.height,
            at_zoom(ROW_HEIGHT, 1.0)
        );
        let blank = w.find(("as-row", 2usize)).bounds();
        assert_eq!(blank.size.height, at_zoom(ROW_HEIGHT, 1.0));
        assert_eq!(blank.left(), first.left());
        let delete = w.find(("as-delete", 0usize)).bounds();
        assert!(
            delete.size.width <= at_zoom(ROW_HEIGHT, 1.0),
            "an icon, not a labelled button: {delete:?}"
        );
        for id in [("as-expr", 0usize), ("as-value", 0usize)] {
            let b = w.find(id).bounds();
            assert!(
                b.top() >= first.top() && b.bottom() <= first.bottom(),
                "{id:?}: {b:?}"
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn assertion_inputs_are_the_small_size(cx: &mut TestAppContext) {
    let (win, _host, _s) = section(cx, vec![new("res.status", "eq", Some("200"), true)]);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let small = w.find(("as-op", 0usize)).bounds().size.height;
        for id in [("as-expr", 0usize), ("as-value", 0usize)] {
            assert_eq!(w.find(id).bounds().size.height, small, "{id:?}");
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn assertion_columns_have_headings(cx: &mut TestAppContext) {
    assert_eq!(
        apinata_app::assertions_section::HEADINGS,
        ["Expression", "Operator", "Value"]
    );
    let (win, _host, _s) = section(cx, vec![new("res.status", "eq", Some("200"), true)]);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        for (head, cell) in [
            ("as-head-expr", "as-expr"),
            ("as-head-op", "as-op"),
            ("as-head-value", "as-value"),
        ] {
            let h = w.find(head).bounds();
            let c = w.find((cell, 0usize)).bounds();
            assert!(
                (f32::from(h.left()) - f32::from(c.left())).abs() < 0.5,
                "{head} {h:?} vs {cell} {c:?}"
            );
        }
    })
    .unwrap();
}

#[test]
fn assertion_columns_name_expression_operator_value_with_a_fixed_operator() {
    let cols = apinata_app::assertions_section::columns();
    let heads: Vec<String> = cols.iter().map(|c| c.heading.to_string()).collect();
    assert_eq!(heads, apinata_app::assertions_section::HEADINGS);
    let cells: Vec<&str> = cols.iter().map(|c| c.cell_id).collect();
    assert_eq!(cells, ["as-expr", "as-op", "as-value"]);
    let widths: Vec<KvWidth> = cols.iter().map(|c| c.width).collect();
    assert_eq!(
        widths,
        [
            KvWidth::Flex,
            KvWidth::Fixed(OPERATOR_SELECT_WIDTH),
            KvWidth::Flex
        ]
    );
}

fn hover_on(cx: &mut TestAppContext, win: AnyWindowHandle, target: impl Into<gpui_kit::ElementId>) {
    let target = target.into();
    cx.update_window(win, |_, w, cx| {
        w.hover(target, cx);
        w.render_frame(cx);
    })
    .unwrap();
}

fn shown_at(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
) -> bool {
    let id = id.into();
    cx.update_window(win, |_, w, _| w.find(id).visible())
        .unwrap()
}

fn action_shown(cx: &mut TestAppContext, win: AnyWindowHandle, ix: usize) -> bool {
    cx.update_window(win, |_, w, _| {
        w.within(("as-row", ix)).find("kv-action").visible()
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_assertion_remove_shows_on_hover_or_focus(cx: &mut TestAppContext) {
    let (win, _host, _s) = section(cx, vec![new("res.status", "eq", Some("200"), true)]);
    // Hover-only edges are the dark themes' behaviour; light ones rest visible.
    support::use_theme(cx, apinata_app::theme::DARK);
    hover_on(cx, win, "as-head");
    assert!(!action_shown(cx, win, 0));
    hover_on(cx, win, ("as-row", 0usize));
    assert!(action_shown(cx, win, 0));
    click(cx, win, ("as-expr", 0usize));
    hover_on(cx, win, "as-head");
    assert!(action_shown(cx, win, 0));
    assert!(shown_at(cx, win, ("as-expr-edge", 0usize)));
    assert!(!shown_at(cx, win, ("as-value-edge", 0usize)));
}

#[gpui_kit::test]
fn the_assertion_heading_matches_the_key_value_heading(cx: &mut TestAppContext) {
    let (win, _host, _s) = section(cx, vec![new("res.status", "eq", Some("200"), true)]);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let head = w.within("as-head").find("kv-check").bounds();
        let check = w.within(("as-row", 0usize)).find("kv-check").bounds();
        assert_eq!(
            (head.left(), head.size.width),
            (check.left(), at_zoom(KV_CHECK_WIDTH, 1.0))
        );
        let head_action = w.within("as-head").find("kv-action").bounds();
        let action = w.within(("as-row", 0usize)).find("kv-action").bounds();
        assert_eq!(head_action.right(), action.right());
    })
    .unwrap();
}

// The assertions heading has no bulk toggle but the band height of a params or headers table
// rendered beside it, which is `KV_HEAD_HEIGHT`.
#[gpui_kit::test]
fn the_heading_band_matches_the_key_value_tables(cx: &mut TestAppContext) {
    use apinata_app::kv_table::{KvRow, KvTable};
    use apinata_app::style::KV_HEAD_HEIGHT;
    let (win, _host, _s) = section(cx, vec![new("res.status", "eq", Some("200"), true)]);
    support::init(cx);
    let (kv_win, _table) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                let table = cx.new(|cx| KvTable::new(w, cx));
                let rows = vec![KvRow {
                    name: "a".into(),
                    value: "1".into(),
                    enabled: true,
                    ..KvRow::default()
                }];
                table.update(cx, |t, cx| t.load(rows, w, cx));
                table
            })
        })
        .unwrap();
    cx.run_until_parked();
    let height = |cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find(id).bounds().size.height
        })
        .unwrap()
    };
    let params = height(cx, kv_win, "kv-head");
    let assertions = height(cx, win, "as-head");
    assert_eq!(
        assertions, params,
        "assertions band against the params band"
    );
    assert_eq!(params, at_zoom(KV_HEAD_HEIGHT, 1.0));
}

// After a row's Remove is clicked, focus lands on a neighbour's expression so the editor's
// keys (Save among them) still reach their handlers.
fn delete_focus(cx: &mut TestAppContext, count: usize, ix: usize, target: usize) {
    let base = (0..count)
        .map(|n| new(&format!("e{n}"), "eq", Some("1"), true))
        .collect();
    let (win, _host, s) = section(cx, base);
    let want = s
        .read_with(cx, |s, _| s.expression_input(target).cloned())
        .unwrap();
    click(cx, win, ("as-delete", ix));
    let on_target = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            want.focus_handle(cx).is_focused(w)
        })
        .unwrap();
    assert!(
        on_target,
        "deleting row {ix} of {count} focuses row {target}"
    );
}

#[gpui_kit::test]
fn deleting_a_middle_assertion_focuses_the_next_row(cx: &mut TestAppContext) {
    delete_focus(cx, 3, 1, 2);
}

#[gpui_kit::test]
fn deleting_the_last_assertion_focuses_the_previous_row(cx: &mut TestAppContext) {
    delete_focus(cx, 3, 2, 1);
}

#[gpui_kit::test]
fn deleting_the_only_assertion_focuses_the_empty_row(cx: &mut TestAppContext) {
    delete_focus(cx, 1, 0, 1);
}

#[gpui_kit::test]
fn a_press_in_the_edge_of_an_expression_cell_focuses_it(cx: &mut TestAppContext) {
    let (win, _host, s) = section(cx, vec![new("res.status", "eq", Some("200"), true)]);
    let input = s
        .read_with(cx, |s, _| s.expression_input(0).cloned())
        .unwrap();
    press_in(cx, win, ("as-expr", 0usize), 4.);
    cx.update_window(win, |_, w, cx| {
        assert!(
            input.focus_handle(cx).is_focused(w),
            "4px press did not focus"
        )
    })
    .unwrap();
}

/// One `eq 1` assertion per letter of `names`, its expression the letter; an upper-case
/// letter is the lower-case one turned off.
fn asserts(names: &str) -> Vec<Assertion> {
    (names.chars())
        .map(|c| {
            new(
                &c.to_ascii_lowercase().to_string(),
                "eq",
                Some("1"),
                c.is_lowercase(),
            )
        })
        .collect()
}

fn expressions(cx: &mut TestAppContext, s: &Entity<AssertionsSection>) -> String {
    s.read_with(cx, |s, cx| {
        s.rows(cx).into_iter().map(|r| r.expression).collect()
    })
}

/// Load `rows` `loads` times in one update, as the app's notices can before its next frame.
fn load_before_a_draw(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    s: &Entity<AssertionsSection>,
    rows: Vec<Assertion>,
    loads: usize,
) {
    cx.update_window(win, |_, w, cx| {
        for _ in 0..loads {
            s.update(cx, |s, cx| s.load(&rows, w, cx));
        }
    })
    .unwrap();
    cx.run_until_parked();
}

fn tab_to(cx: &mut TestAppContext, win: AnyWindowHandle, id: (&'static str, usize)) {
    click(cx, win, ("as-expr", 0usize));
    for _ in 0..40 {
        if holds_focus(cx, win, id) {
            return;
        }
        cx.simulate_keystrokes(win, "tab");
        cx.run_until_parked();
    }
    panic!("Tab never reached {id:?}");
}

// An undone delete of a row above brings it back: c's Remove keeps focus one slot down.
fn insert_above_keeps_remove(cx: &mut TestAppContext, loads: usize) {
    let (win, _host, s) = section(cx, asserts("ac"));
    tab_to(cx, win, ("as-delete", 1));
    load_before_a_draw(cx, win, &s, asserts("abc"), loads);
    assert_eq!(expressions(cx, &s), "abc");
    assert!(holds_focus(cx, win, ("as-delete", 2usize)));
}

#[gpui_kit::test]
fn a_reload_inserting_a_row_above_keeps_focus_on_the_shifted_assertions_remove(
    cx: &mut TestAppContext,
) {
    insert_above_keeps_remove(cx, 1);
}

#[gpui_kit::test]
fn two_reloads_inserting_a_row_above_keep_focus_on_the_shifted_assertions_remove(
    cx: &mut TestAppContext,
) {
    insert_above_keeps_remove(cx, 2);
}

// The same with c's checkbox focused by a click, which turned c off.
fn insert_above_keeps_checkbox(cx: &mut TestAppContext, loads: usize) {
    let (win, _host, s) = section(cx, asserts("ac"));
    click(cx, win, ("as-enabled", 1usize));
    load_before_a_draw(cx, win, &s, asserts("abC"), loads);
    assert_eq!(expressions(cx, &s), "abc");
    assert!(holds_focus(cx, win, ("as-enabled", 2usize)));
}

#[gpui_kit::test]
fn a_reload_inserting_a_row_above_keeps_focus_on_the_shifted_assertions_checkbox(
    cx: &mut TestAppContext,
) {
    insert_above_keeps_checkbox(cx, 1);
}

#[gpui_kit::test]
fn two_reloads_inserting_a_row_above_keep_focus_on_the_shifted_assertions_checkbox(
    cx: &mut TestAppContext,
) {
    insert_above_keeps_checkbox(cx, 2);
}

// An undone move puts a back last: its Remove keeps focus at the new place.
fn move_keeps_remove(cx: &mut TestAppContext, loads: usize) {
    let (win, _host, s) = section(cx, asserts("abc"));
    tab_to(cx, win, ("as-delete", 0));
    load_before_a_draw(cx, win, &s, asserts("bca"), loads);
    assert_eq!(expressions(cx, &s), "bca");
    assert!(holds_focus(cx, win, ("as-delete", 2usize)));
}

#[gpui_kit::test]
fn a_reload_moving_the_row_keeps_focus_on_its_assertions_remove(cx: &mut TestAppContext) {
    move_keeps_remove(cx, 1);
}

#[gpui_kit::test]
fn two_reloads_moving_the_row_keep_focus_on_its_assertions_remove(cx: &mut TestAppContext) {
    move_keeps_remove(cx, 2);
}

// A reload dropping the row whose Remove holds focus focuses the next row's expression.
fn drop_from_remove(cx: &mut TestAppContext, loads: usize) {
    let (win, _host, s) = section(cx, asserts("abc"));
    tab_to(cx, win, ("as-delete", 1));
    load_before_a_draw(cx, win, &s, asserts("ac"), loads);
    assert_eq!(expressions(cx, &s), "ac");
    assert!(holds_focus(cx, win, ("as-expr", 1usize)));
}

#[gpui_kit::test]
fn a_reload_dropping_the_row_whose_remove_holds_focus_focuses_the_next_expression(
    cx: &mut TestAppContext,
) {
    drop_from_remove(cx, 1);
}

#[gpui_kit::test]
fn two_reloads_dropping_the_row_whose_remove_holds_focus_focus_the_next_expression(
    cx: &mut TestAppContext,
) {
    drop_from_remove(cx, 2);
}

// A reload moves c's Remove and the section hides before the next draw; focus then moves
// elsewhere (and `twice`, another load lands while hidden). Showing it leaves focus there.
fn hidden_load_keeps_focus_elsewhere(cx: &mut TestAppContext, twice: bool) {
    let (win, host, s) = section(cx, asserts("ac"));
    tab_to(cx, win, ("as-delete", 1));
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&asserts("abc"), w, cx));
        host.update(cx, |h, cx| {
            h.shown = false;
            cx.notify()
        });
    })
    .unwrap();
    frame(cx, win);
    let elsewhere = host.read_with(cx, |h, _| h.elsewhere.clone());
    cx.update_window(win, |_, w, cx| elsewhere.focus(w, cx))
        .unwrap();
    frame(cx, win);
    let moved = cx
        .update_window(win, |_, w, _| elsewhere.is_focused(w))
        .unwrap();
    assert!(moved, "focus moved elsewhere");
    if twice {
        load_before_a_draw(cx, win, &s, asserts("nabc"), 1);
        frame(cx, win);
    }
    host.update(cx, |h, cx| {
        h.shown = true;
        cx.notify()
    });
    frame(cx, win);
    let kept = cx
        .update_window(win, |_, w, _| elsewhere.is_focused(w))
        .unwrap();
    assert!(kept, "showing the section leaves focus where it was moved");
}

#[gpui_kit::test]
fn a_reload_hidden_before_its_draw_leaves_focus_where_it_was_moved(cx: &mut TestAppContext) {
    hidden_load_keeps_focus_elsewhere(cx, false);
}

#[gpui_kit::test]
fn a_second_reload_while_hidden_leaves_focus_where_it_was_moved(cx: &mut TestAppContext) {
    hidden_load_keeps_focus_elsewhere(cx, true);
}

// Opens row 0's operator picker and types `query` into its search.
fn search_operators(cx: &mut TestAppContext, query: &str) -> AnyWindowHandle {
    let (win, _host, _s) = section(cx, vec![new("res.status", "eq", Some("200"), true)]);
    click(cx, win, ("as-op", 0usize));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.input(query, cx))
        .unwrap();
    frame(cx, win);
    win
}

#[gpui_kit::test]
fn operators_read_as_plain_words_with_the_raw_operator_beside(cx: &mut TestAppContext) {
    let (win, host, _s) = section(cx, vec![new("res.status", "eq", Some("200"), true)]);
    let shown = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find(("as-op", 0usize)).value().map(str::to_owned)
        })
        .unwrap()
    };
    assert_eq!(shown(cx).as_deref(), Some("equals"));
    click(cx, win, ("as-op", 0usize));
    let option = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let o = w.find("as-op-option-gte");
            (
                o.label().map(str::to_owned),
                o.description().map(str::to_owned),
            )
        })
        .unwrap();
    assert_eq!(
        option,
        (Some("greater than or equal".into()), Some("gte".into()))
    );
    click(cx, win, "as-op-option-gte");
    assert_eq!(
        last(cx, &host),
        vec![new("res.status", "gte", Some("200"), true)],
        "the file keeps the raw operator"
    );
    assert_eq!(shown(cx).as_deref(), Some("greater than or equal"));
}

#[gpui_kit::test]
fn the_operator_search_matches_plain_words(cx: &mut TestAppContext) {
    let win = search_operators(cx, "greater");
    assert!(present(cx, win, "as-op-option-gt"));
    assert!(present(cx, win, "as-op-option-gte"));
    assert!(!present(cx, win, "as-op-option-eq"));
    assert!(!present(cx, win, "as-op-option-lt"));
}

#[gpui_kit::test]
fn the_operator_search_matches_the_raw_operator(cx: &mut TestAppContext) {
    let win = search_operators(cx, "notCon");
    assert!(present(cx, win, "as-op-option-notContains"));
    assert!(!present(cx, win, "as-op-option-contains"));
    assert!(!present(cx, win, "as-op-option-eq"));
}

#[gpui_kit::test]
fn the_expression_shows_an_example(cx: &mut TestAppContext) {
    let (_win, _host, s) = section(cx, vec![new("res.status", "eq", Some("200"), true)]);
    for ix in [0usize, 1] {
        let input = s.read_with(cx, |s, _| s.expression_input(ix).cloned().unwrap());
        let hint = input.read_with(cx, |i, _| i.presentation().placeholder().to_string());
        assert_eq!(hint, "res.status", "row {ix}");
    }
}
