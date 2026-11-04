use crate::support;

use apinata_app::kv_table::{
    KvEvent, KvRow, KvTable, header_rows, headers_from, param_rows, params_from, var_rows,
    vars_from,
};
use apinata_app::style::{KV_ACTION_WIDTH, KV_CHECK_WIDTH, at_zoom};
use apinata_core::{KeyValue, Param, ParamKind, VarValue, Variable};
use gpui_kit::component::input::InputState;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Context, ElementId, Entity, Focusable, IntoElement, ParentElement,
    Render, Styled, Subscription, TestAppContext, Window, WindowOptions, div,
};

fn row(name: &str, value: &str) -> KvRow {
    KvRow {
        name: name.into(),
        value: value.into(),
        enabled: true,
        ..KvRow::default()
    }
}

#[test]
fn params_keep_kind_and_state_through_rows() {
    let mut off = Param::new("q", "1", ParamKind::Query);
    off.enabled = false;
    let base = vec![off, Param::new("id", "", ParamKind::Path)];
    let mut rows = param_rows(&base);
    assert_eq!(rows[0].tag.as_deref(), Some("query"));
    assert!(!rows[0].enabled && !rows[0].fixed);
    assert!(rows[1].fixed);
    assert_eq!(rows[1].tag.as_deref(), Some("path"));
    rows[1].value = "7".into();
    rows.push(row("new", "v"));
    let out = params_from(&base, &rows);
    assert!(!out[0].enabled);
    assert_eq!(out[1], Param::new("id", "7", ParamKind::Path));
    assert_eq!(out[2], Param::new("new", "v", ParamKind::Query));
}

#[test]
fn headers_follow_their_source_rows() {
    let base = vec![KeyValue::new("A", "1"), KeyValue::new("B", "2")];
    let mut rows = header_rows(&base);
    rows.remove(0);
    rows[0].name = "C".into();
    assert_eq!(rows[0].source, Some(1));
    assert_eq!(headers_from(&base, &rows), vec![KeyValue::new("C", "2")]);
}

#[test]
fn typed_vars_keep_their_type() {
    let base = vec![Variable::new(
        "n",
        VarValue::Typed {
            kind: "number".into(),
            data: "1".into(),
        },
    )];
    let mut rows = var_rows(&base);
    assert_eq!(rows[0].value, "1");
    rows[0].value = "2".into();
    let out = vars_from(&base, &rows);
    assert!(
        out[0].value
            == VarValue::Typed {
                kind: "number".into(),
                data: "2".into()
            }
    ); // VarValue has no Debug
}

struct Host {
    table: Entity<KvTable>,
    events: Vec<Vec<KvRow>>,
    _sub: Subscription,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.table.clone())
    }
}

fn host(cx: &mut TestAppContext, rows: Vec<KvRow>) -> (AnyWindowHandle, Entity<Host>) {
    support::init(cx);
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
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    (win, view)
}

fn last(cx: &mut TestAppContext, view: &Entity<Host>) -> Vec<KvRow> {
    view.read_with(cx, |h, _| h.events.last().cloned().expect("an event"))
}

#[gpui_kit::test]
fn typing_in_a_value_emits_the_rows(cx: &mut TestAppContext) {
    let (win, view) = host(cx, vec![row("a", "1")]);
    cx.update_window(win, |_, w, cx| {
        w.click(("kv-value", 0usize), cx);
        w.press("end", cx);
        w.input("2", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(last(cx, &view)[0].value, "12");
}

fn table_of(cx: &mut TestAppContext, view: &Entity<Host>) -> Entity<KvTable> {
    view.read_with(cx, |h, _| h.table.clone())
}

fn shown<const N: usize>(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    ids: [(&'static str, usize); N],
) -> [bool; N] {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        ids.map(|id| w.try_find(id).is_some())
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_table_ends_with_one_empty_row_that_is_never_reported(cx: &mut TestAppContext) {
    let (win, view) = host(cx, vec![row("a", "1")]);
    let ids = [
        ("kv-name", 1),
        ("kv-value", 1),
        ("kv-enabled", 1),
        ("kv-delete", 1),
        ("kv-name", 2),
    ];
    assert_eq!(shown(cx, win, ids), [true, true, false, false, false]);
    assert!(
        cx.update_window(win, |_, w, _| w.try_find("kv-add").is_none())
            .unwrap()
    );
    let table = table_of(cx, &view);
    assert_eq!(table.read_with(cx, |t, cx| t.rows(cx)), vec![row("a", "1")]);
    assert!(view.read_with(cx, |h, _| h.events.is_empty()));
}

#[gpui_kit::test]
fn an_empty_table_still_shows_the_empty_row(cx: &mut TestAppContext) {
    let (win, _view) = host(cx, vec![]);
    assert_eq!(
        shown(cx, win, [("kv-name", 0), ("kv-value", 0), ("kv-delete", 0)]),
        [true, true, false]
    );
}

#[gpui_kit::test]
fn typing_in_the_empty_row_makes_it_real_and_adds_another(cx: &mut TestAppContext) {
    let (win, view) = host(cx, vec![row("a", "1")]);
    let table = table_of(cx, &view);
    let typed = table
        .read_with(cx, |t, _| t.name_input(1).cloned())
        .unwrap();
    let hint = |i: &Entity<InputState>, cx: &mut TestAppContext| {
        i.read_with(cx, |i, _| i.presentation().placeholder().to_string())
    };
    assert_eq!(hint(&typed, cx), "Key");
    let value = table
        .read_with(cx, |t, _| t.value_input(1).cloned())
        .unwrap();
    assert_eq!(hint(&value, cx), "Value");
    cx.update_window(win, |_, w, cx| {
        w.click(("kv-name", 1usize), cx);
        w.input("b", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(last(cx, &view), vec![row("a", "1"), row("b", "")]);
    // The same input stays focused and keeps receiving the typing.
    cx.update_window(win, |_, w, cx| {
        assert!(typed.focus_handle(cx).is_focused(w));
        w.input("c", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(last(cx, &view), vec![row("a", "1"), row("bc", "")]);
    assert_eq!(
        shown(
            cx,
            win,
            [
                ("kv-delete", 1),
                ("kv-enabled", 1),
                ("kv-name", 2),
                ("kv-delete", 2),
                ("kv-name", 3)
            ]
        ),
        [true, true, true, false, false]
    );
}

#[gpui_kit::test]
fn typing_a_value_first_also_makes_a_row(cx: &mut TestAppContext) {
    let (win, view) = host(cx, vec![]);
    cx.update_window(win, |_, w, cx| {
        w.click(("kv-value", 0usize), cx);
        w.input("v", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(last(cx, &view), vec![row("", "v")]);
}

#[gpui_kit::test]
fn clearing_a_row_keeps_it(cx: &mut TestAppContext) {
    let (win, view) = host(cx, vec![row("a", "")]);
    cx.update_window(win, |_, w, cx| {
        w.click(("kv-name", 0usize), cx);
        w.press("end", cx);
        w.press("backspace", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(last(cx, &view), vec![row("", "")]);
    assert_eq!(
        shown(cx, win, [("kv-delete", 0), ("kv-name", 1), ("kv-name", 2)]),
        [true, true, false]
    );
}

#[gpui_kit::test]
fn delete_and_toggle_emit_the_rows(cx: &mut TestAppContext) {
    let (win, view) = host(cx, vec![row("a", "1"), row("b", "2")]);
    cx.update_window(win, |_, w, cx| w.click(("kv-delete", 0usize), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(last(cx, &view), vec![row("b", "2")]);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(("kv-enabled", 0usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!last(cx, &view)[0].enabled);
}

#[gpui_kit::test]
fn a_reload_keeps_the_empty_row(cx: &mut TestAppContext) {
    let (win, view) = host(cx, vec![row("a", "1"), row("b", "2")]);
    let table = table_of(cx, &view);
    let blank = table
        .read_with(cx, |t, _| t.name_input(2).cloned())
        .unwrap();
    cx.update_window(win, |_, w, cx| {
        table.update(cx, |t, cx| t.load(vec![row("z", "9")], w, cx))
    })
    .unwrap();
    assert_eq!(table.read_with(cx, |t, cx| t.rows(cx)), vec![row("z", "9")]);
    assert_eq!(
        table.read_with(cx, |t, _| t.name_input(1).cloned()),
        Some(blank)
    );
    assert_eq!(
        shown(cx, win, [("kv-name", 1), ("kv-delete", 1)]),
        [true, false]
    );
}

#[gpui_kit::test]
fn fixed_rows_have_no_toggle_or_delete(cx: &mut TestAppContext) {
    let fixed = KvRow {
        fixed: true,
        tag: Some("path".into()),
        ..row("id", "")
    };
    let (win, _view) = host(cx, vec![fixed]);
    let found = cx
        .update_window(win, |_, w, _| {
            [
                ("kv-enabled", 0usize),
                ("kv-delete", 0),
                ("kv-tag", 0),
                ("kv-name", 0),
            ]
            .map(|id| w.try_find(id).is_some())
        })
        .unwrap();
    assert_eq!(found, [false, false, true, true]);
}

#[gpui_kit::test]
fn load_keeps_the_caret_when_values_match_and_replaces_them_otherwise(cx: &mut TestAppContext) {
    let (win, view) = host(cx, vec![row("a", "1"), row("b", "2")]);
    cx.update_window(win, |_, w, cx| {
        w.click(("kv-value", 0usize), cx);
        w.press("end", cx);
        w.input("2", cx);
        w.press("left", cx)
    })
    .unwrap();
    cx.run_until_parked();
    let table = view.read_with(cx, |h, _| h.table.clone());
    let input = table
        .read_with(cx, |t, _| t.value_input(0).cloned())
        .unwrap();
    let events = view.read_with(cx, |h, _| h.events.len());
    cx.update_window(win, |_, w, cx| {
        table.update(cx, |t, cx| {
            t.load(vec![row("a", "12"), row("b", "2")], w, cx)
        })
    })
    .unwrap();
    assert_eq!(input.read_with(cx, |s, _| s.cursor()), 1);
    cx.update_window(win, |_, w, cx| {
        table.update(cx, |t, cx| t.load(vec![row("a", "99")], w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        table.read_with(cx, |t, _| t.value_input(0).cloned()),
        Some(input.clone())
    );
    assert_eq!(input.read_with(cx, |s, _| s.value().to_string()), "99");
    assert_eq!(table.read_with(cx, |t, cx| t.rows(cx).len()), 1);
    assert_eq!(view.read_with(cx, |h, _| h.events.len()), events);
}

#[gpui_kit::test]
fn a_locked_table_refuses_input(cx: &mut TestAppContext) {
    let (win, view) = host(cx, vec![row("a", "1")]);
    let table = view.read_with(cx, |h, _| h.table.clone());
    cx.update_window(win, |_, w, cx| w.click(("kv-value", 0usize), cx))
        .unwrap();
    table.update(cx, |t, cx| t.set_locked(true, cx));
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.input("9", cx);
        w.click(("kv-name", 1usize), cx);
        w.input("x", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert!(view.read_with(cx, |h, _| h.events.is_empty()));
    assert_eq!(table.read_with(cx, |t, cx| t.rows(cx)), vec![row("a", "1")]);
    table.update(cx, |t, cx| t.set_locked(false, cx));
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(("kv-value", 0usize), cx);
        w.press("end", cx);
        w.input("9", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(last(cx, &view)[0].value, "19");
}

#[gpui_kit::test]
fn an_error_shows_under_the_table(cx: &mut TestAppContext) {
    let (win, view) = host(cx, vec![row("a", "1")]);
    let table = view.read_with(cx, |h, _| h.table.clone());
    table.update(cx, |t, cx| {
        t.set_error(Some("`headers[0].name` cannot be written".into()), None, cx)
    });
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    assert!(
        cx.update_window(win, |_, w, _| w.try_find("kv-error").is_some())
            .unwrap()
    );
}

#[gpui_kit::test]
fn the_empty_row_lines_up_with_the_real_rows(cx: &mut TestAppContext) {
    let (win, _view) = host(cx, vec![row("a", "1")]);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        for id in ["kv-name", "kv-value"] {
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
            [("kv-slot-enabled", 1usize), ("kv-slot-delete", 1)].map(|id| !w.find(id).visible())
        })
        .unwrap();
    assert_eq!(hidden, [true, true]);
}

#[gpui_kit::test]
fn the_empty_row_lines_up_with_tagged_rows(cx: &mut TestAppContext) {
    let tagged = KvRow {
        tag: Some("query".into()),
        ..row("a", "1")
    };
    let (win, _view) = host(cx, vec![tagged]);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        for id in ["kv-name", "kv-value"] {
            let real = w.find((id, 0usize)).bounds();
            let blank = w.find((id, 1usize)).bounds();
            assert_eq!(
                (blank.left(), blank.size.width),
                (real.left(), real.size.width),
                "{id}"
            );
        }
        assert!(w.try_find(("kv-tag", 1usize)).is_none());
        assert!(!w.find(("kv-slot-tag", 1usize)).visible());
    })
    .unwrap();
}

// Tab from a row's last field reaches its Remove control, then the empty row's key.
#[gpui_kit::test]
fn tab_from_the_last_field_reaches_remove_then_the_empty_row(cx: &mut TestAppContext) {
    let (win, view) = host(cx, vec![row("a", "1")]);
    let table = table_of(cx, &view);
    let blank = table
        .read_with(cx, |t, _| t.name_input(1).cloned())
        .unwrap();
    cx.update_window(win, |_, w, cx| w.click(("kv-value", 0usize), cx))
        .unwrap();
    cx.simulate_keystrokes(win, "tab");
    cx.run_until_parked();
    let on_delete = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find(("kv-delete", 0usize)).focused()
        })
        .unwrap();
    assert_eq!(on_delete, Some(true), "Remove is the row's last stop");
    cx.update_window(win, |_, w, cx| w.focus_next(cx)).unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        assert!(blank.focus_handle(cx).is_focused(w))
    })
    .unwrap();
}

#[gpui_kit::test]
fn column_headings_sit_over_their_columns(cx: &mut TestAppContext) {
    let (win, view) = host(cx, vec![row("a", "1")]);
    let table = table_of(cx, &view);
    assert_eq!(table.read_with(cx, |t, _| t.headings()), ["Key", "Value"]);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let (hn, hv) = (
            w.find("kv-head-name").bounds(),
            w.find("kv-head-value").bounds(),
        );
        let (n, v) = (
            w.find(("kv-name", 0usize)).bounds(),
            w.find(("kv-value", 0usize)).bounds(),
        );
        assert_eq!(hn.left(), n.left(), "Key over the key column");
        assert_eq!(hv.left(), v.left(), "Value over the value column");
        assert!(hn.bottom() <= n.top(), "headings above the first row");
        assert!(w.try_find("kv-head-ctype").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn an_empty_table_still_has_headings(cx: &mut TestAppContext) {
    let (win, _view) = host(cx, vec![]);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("kv-head-name").is_some());
        assert!(w.try_find("kv-head-value").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn headings_line_up_over_tagged_rows(cx: &mut TestAppContext) {
    let (win, _view) = host(cx, param_rows(&[Param::new("q", "1", ParamKind::Query)]));
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let hv = w.find("kv-head-value").bounds();
        let v = w.find(("kv-value", 0usize)).bounds();
        assert_eq!(hv.left(), v.left(), "Value over a tagged row's value");
        assert_eq!(hv.right(), v.right(), "the tag slot keeps the widths equal");
    })
    .unwrap();
}

#[gpui_kit::test]
fn fixed_rows_keep_the_columns_of_other_rows_and_the_headings(cx: &mut TestAppContext) {
    let fixed = KvRow {
        fixed: true,
        tag: Some("path".into()),
        ..row("id", "")
    };
    let query = KvRow {
        tag: Some("query".into()),
        ..row("q", "1")
    };
    let (win, _view) = host(cx, vec![fixed, query]);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        for (head, id) in [("kv-head-name", "kv-name"), ("kv-head-value", "kv-value")] {
            let fixed = w.find((id, 0usize)).bounds();
            let other = w.find((id, 1usize)).bounds();
            let heading = w.find(head).bounds();
            assert_eq!(
                (fixed.left(), fixed.size.width),
                (other.left(), other.size.width),
                "{id}"
            );
            assert_eq!(fixed.left(), heading.left(), "{head}");
        }
        let hidden = [("kv-enabled", 0usize), ("kv-delete", 0)].map(|id| w.try_find(id).is_some());
        assert_eq!(
            hidden,
            [false, false],
            "a fixed row still has no toggle or delete"
        );
    })
    .unwrap();
}

// Moves the pointer onto `target` and draws the frame the hover shows in.
fn hover(cx: &mut TestAppContext, win: AnyWindowHandle, target: impl Into<ElementId>) {
    let target = target.into();
    cx.update_window(win, |_, w, cx| {
        w.hover(target, cx);
        w.render_frame(cx);
    })
    .unwrap();
}

fn shows(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    scope: (&'static str, usize),
    id: &'static str,
) -> bool {
    cx.update_window(win, |_, w, _| w.within(scope).find(id).visible())
        .unwrap()
}

fn edge(cx: &mut TestAppContext, win: AnyWindowHandle, id: (&'static str, usize)) -> bool {
    cx.update_window(win, |_, w, _| w.find(id).visible())
        .unwrap()
}

#[gpui_kit::test]
fn the_heading_row_leaves_the_check_column_empty(cx: &mut TestAppContext) {
    let (win, _view) = host(cx, vec![row("a", "1")]);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let head = w.within("kv-head").find("kv-check").bounds();
        let check = w.within(("kv-row", 0usize)).find("kv-check").bounds();
        assert_eq!(
            (head.left(), head.size.width),
            (check.left(), check.size.width)
        );
        assert_eq!(check.size.width, at_zoom(KV_CHECK_WIDTH, 1.0));
        let toggle = w.find(("kv-enabled", 0usize)).bounds();
        assert!(
            toggle.size.width <= check.size.width,
            "{toggle:?} in {check:?}"
        );
        let head_action = w.within("kv-head").find("kv-action").bounds();
        let action = w.within(("kv-row", 0usize)).find("kv-action").bounds();
        assert_eq!(head_action.right(), action.right());
        assert_eq!(action.size.width, at_zoom(KV_ACTION_WIDTH, 1.0));
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_remove_control_shows_on_row_hover(cx: &mut TestAppContext) {
    let (win, _view) = host(cx, vec![row("a", "1"), row("b", "2")]);
    hover(cx, win, "kv-head");
    assert!(!shows(cx, win, ("kv-row", 0), "kv-action"));
    hover(cx, win, ("kv-row", 0usize));
    assert!(shows(cx, win, ("kv-row", 0), "kv-action"));
    assert!(!shows(cx, win, ("kv-row", 1), "kv-action"));
}

#[gpui_kit::test]
fn a_focused_row_keeps_its_remove_control_and_cell_edge(cx: &mut TestAppContext) {
    let (win, _view) = host(cx, vec![row("a", "1")]);
    // Hover-only edges are the dark themes' behaviour; light ones rest visible.
    support::use_theme(cx, apinata_app::theme::DARK);
    cx.update_window(win, |_, w, cx| w.click(("kv-value", 0usize), cx))
        .unwrap();
    hover(cx, win, "kv-head");
    assert!(shows(cx, win, ("kv-row", 0), "kv-action"));
    assert!(edge(cx, win, ("kv-value-edge", 0)));
    assert!(!edge(cx, win, ("kv-name-edge", 0)));
}

#[gpui_kit::test]
fn a_cell_shows_its_edge_only_under_the_pointer(cx: &mut TestAppContext) {
    let (win, _view) = host(cx, vec![row("a", "1")]);
    // Hover-only edges are the dark themes' behaviour; light ones rest visible.
    support::use_theme(cx, apinata_app::theme::DARK);
    hover(cx, win, "kv-head");
    assert!(!edge(cx, win, ("kv-name-edge", 0)));
    hover(cx, win, ("kv-name", 0usize));
    assert!(edge(cx, win, ("kv-name-edge", 0)));
    assert!(!edge(cx, win, ("kv-value-edge", 0)));
}

#[gpui_kit::test]
fn a_click_beside_the_remove_control_leaves_no_focus_in_its_slot(cx: &mut TestAppContext) {
    let (win, _view) = host(cx, vec![row("a", "1")]);
    cx.update_window(win, |_, w, cx| w.click(("kv-name", 0usize), cx))
        .unwrap();
    hover(cx, win, ("kv-row", 0usize));
    cx.update_window(win, |_, w, cx| {
        let slot = w.within(("kv-row", 0usize)).find("kv-action").bounds();
        let button = w.find(("kv-delete", 0usize)).bounds();
        assert!(button.left() > slot.left(), "{button:?} fills {slot:?}");
        // The button is centred in a slot a pixel wider on each side; aim at that margin.
        let beside = gpui_kit::point(gpui_kit::px(0.5), slot.size.height / 2.);
        w.within(("kv-row", 0usize))
            .click_at("kv-action", beside, cx);
    })
    .unwrap();
    cx.run_until_parked();
    // The click lands where any click on the table's empty space does, not in the slot,
    // so the slot hides again once the pointer leaves.
    hover(cx, win, "kv-head");
    assert!(!shows(cx, win, ("kv-row", 0), "kv-action"));
}

// The params' kind column carries a muted heading like the other columns, lined up with it.
#[gpui_kit::test]
fn the_tag_column_has_a_heading_over_it(cx: &mut TestAppContext) {
    let tagged = KvRow {
        tag: Some("query".into()),
        ..row("q", "1")
    };
    let (win, _view) = host(cx, vec![tagged]);
    let (head, cell, label) = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let head = w.find("kv-head-tag");
            (
                head.bounds(),
                w.find(("kv-tag", 0usize)).bounds(),
                head.label().map(str::to_owned),
            )
        })
        .unwrap();
    assert_eq!(label.as_deref(), Some("Type"));
    assert_eq!(head.left(), cell.left());
    assert_eq!(head.size.width, cell.size.width);
}

fn focused_on(cx: &mut TestAppContext, win: AnyWindowHandle, id: impl Into<ElementId>) -> bool {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).focused() == Some(true)
    })
    .unwrap()
}

fn next(cx: &mut TestAppContext, win: AnyWindowHandle) {
    cx.update_window(win, |_, w, cx| w.focus_next(cx)).unwrap();
    cx.run_until_parked();
}

// Tab starts at the heading checkbox, then the heading's bulk toggle; a row's order is
// checkbox, key, value, Remove; the focused Remove shows without hover.
#[gpui_kit::test]
fn tab_walks_a_row_from_its_checkbox_to_remove(cx: &mut TestAppContext) {
    let (win, view) = host(cx, vec![row("a", "1")]);
    let table = table_of(cx, &view);
    let (name, value) = table.read_with(cx, |t, _| {
        (
            t.name_input(0).cloned().unwrap(),
            t.value_input(0).cloned().unwrap(),
        )
    });
    next(cx, win);
    assert!(
        focused_on(cx, win, "kv-head-enabled"),
        "the heading checkbox is first"
    );
    next(cx, win);
    assert!(
        focused_on(cx, win, "kv-bulk-toggle"),
        "then the heading's bulk toggle"
    );
    next(cx, win);
    assert!(
        focused_on(cx, win, ("kv-enabled", 0usize)),
        "then the row's checkbox"
    );
    next(cx, win);
    cx.update_window(win, |_, w, cx| assert!(name.focus_handle(cx).is_focused(w)))
        .unwrap();
    next(cx, win);
    cx.update_window(win, |_, w, cx| {
        assert!(value.focus_handle(cx).is_focused(w))
    })
    .unwrap();
    next(cx, win);
    assert!(focused_on(cx, win, ("kv-delete", 0usize)));
    hover(cx, win, "kv-head");
    assert!(
        shows(cx, win, ("kv-row", 0), "kv-action"),
        "focus keeps Remove shown"
    );
}

#[gpui_kit::test]
fn space_on_the_checkbox_turns_the_row_off(cx: &mut TestAppContext) {
    let (win, view) = host(cx, vec![row("a", "1")]);
    support::tab_to(cx, win, ("kv-enabled", 0usize));
    cx.update_window(win, |_, w, cx| w.press("space", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(!last(cx, &view)[0].enabled);
}

#[gpui_kit::test]
fn enter_on_remove_deletes_the_row(cx: &mut TestAppContext) {
    let (win, view) = host(cx, vec![row("a", "1"), row("b", "2")]);
    support::tab_to(cx, win, ("kv-delete", 0usize));
    cx.update_window(win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    let names: Vec<String> = last(cx, &view).iter().map(|r| r.name.clone()).collect();
    assert_eq!(names, ["b"]);
}

#[gpui_kit::test]
fn a_tab_focused_checkbox_draws_the_keyboard_ring(cx: &mut TestAppContext) {
    use apinata_app::focus_mode::{self, Modality};
    let (win, _view) = host(cx, vec![row("a", "1")]);
    cx.update(|cx| focus_mode::set(Modality::Keyboard, cx));
    support::tab_to(cx, win, ("kv-enabled", 0usize));
    let ring = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.within(("kv-enabled", 0usize))
                .try_find("checkbox-focus")
                .is_some()
        })
        .unwrap();
    assert!(ring);
}

// A row whose checkbox holds keyboard focus shows its Remove without hover.
#[gpui_kit::test]
fn a_focused_checkbox_keeps_its_row_remove_shown(cx: &mut TestAppContext) {
    let (win, _view) = host(cx, vec![row("a", "1"), row("b", "2")]);
    support::tab_to(cx, win, ("kv-enabled", 0usize));
    hover(cx, win, "kv-head");
    assert!(focused_on(cx, win, ("kv-enabled", 0usize)));
    assert!(shows(cx, win, ("kv-row", 0usize), "kv-action"));
    assert!(!shows(cx, win, ("kv-row", 1), "kv-action"));
}

// After a row's Remove is clicked, focus lands on a neighbour's first field so the editor's
// keys (Save among them) still reach their handlers.
fn delete_focus(cx: &mut TestAppContext, names: &[&str], ix: usize, target: usize) {
    let (win, view) = host(cx, names.iter().map(|n| row(n, "v")).collect());
    let table = table_of(cx, &view);
    let want = table
        .read_with(cx, |t, _| t.name_input(target).cloned())
        .unwrap();
    cx.update_window(win, |_, w, cx| w.click(("kv-delete", ix), cx))
        .unwrap();
    cx.run_until_parked();
    let on_target = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            want.focus_handle(cx).is_focused(w)
        })
        .unwrap();
    assert!(
        on_target,
        "deleting row {ix} of {names:?} focuses row {target}"
    );
}

#[gpui_kit::test]
fn deleting_a_middle_row_focuses_the_next_row(cx: &mut TestAppContext) {
    delete_focus(cx, &["a", "b", "c"], 1, 2);
}

#[gpui_kit::test]
fn deleting_the_last_row_focuses_the_previous_row(cx: &mut TestAppContext) {
    delete_focus(cx, &["a", "b", "c"], 2, 1);
}

#[gpui_kit::test]
fn deleting_the_only_row_focuses_the_empty_row(cx: &mut TestAppContext) {
    delete_focus(cx, &["a"], 0, 1);
}

/// A locked table cell is told apart by the muted fill and is read-only in every theme, so a
/// click focuses it and its text selects; the high-contrast themes keep full-strength text.
#[gpui_kit::test]
fn a_locked_cell_is_read_only_in_every_theme(cx: &mut TestAppContext) {
    for (theme, high_contrast) in [
        (apinata_app::theme::HC_DARK, true),
        (apinata_app::theme::HC_LIGHT, true),
        (apinata_app::theme::DARK, false),
        (apinata_app::theme::LIGHT, false),
    ] {
        let (win, view) = host(cx, vec![row("a", "1")]);
        support::use_theme(cx, theme);
        let table = view.read_with(cx, |h, _| h.table.clone());
        table.update(cx, |t, cx| t.set_locked(true, cx));
        cx.update_window(win, |_, w, cx| w.render_frame(cx))
            .unwrap();
        let input = table
            .read_with(cx, |t, _| t.value_input(0).cloned())
            .unwrap();
        let (readonly, disabled) = input.read_with(cx, |i, _| {
            let p = i.presentation();
            (p.is_readonly(), p.is_disabled())
        });
        assert_eq!((readonly, disabled), (true, false), "{theme}");
        if high_contrast {
            let (text, fill) = cx.update(|cx| {
                use gpui_kit::component::ActiveTheme;
                (cx.theme().foreground, cx.theme().muted)
            });
            assert!(support::contrast(text, fill) >= 7.0, "{theme}");
        }
    }
}

// Mouse helpers: a press in the padding of a cell is a different hit from one in its text.
mod edge {
    use super::*;
    use gpui_kit::{InputEvent as _, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent};
    use gpui_kit::{Pixels, Point, point};

    pub fn event_at(p: Point<Pixels>, held: Option<MouseButton>) -> gpui_kit::PlatformInput {
        MouseMoveEvent {
            position: p,
            pressed_button: held,
            modifiers: Default::default(),
        }
        .to_platform_input()
    }

    /// Presses `count` times at `dx` pixels inside the left edge of `id`'s cell.
    pub fn click_in(
        cx: &mut TestAppContext,
        win: AnyWindowHandle,
        id: (&'static str, usize),
        dx: f32,
        count: usize,
    ) {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let b = w.find(id).bounds();
            let p = point(b.left() + gpui_kit::px(dx), b.center().y);
            w.dispatch_event(event_at(p, None), cx);
            for n in 1..=count {
                let down = MouseDownEvent {
                    button: MouseButton::Left,
                    position: p,
                    modifiers: Default::default(),
                    click_count: n,
                    first_mouse: false,
                };
                w.dispatch_event(down.to_platform_input(), cx);
                let up = MouseUpEvent {
                    button: MouseButton::Left,
                    position: p,
                    modifiers: Default::default(),
                    click_count: n,
                };
                w.dispatch_event(up.to_platform_input(), cx);
            }
        })
        .unwrap();
        cx.run_until_parked();
    }

    /// Presses `dx` inside the left edge, drags 8 steps right, releases.
    pub fn drag_in(
        cx: &mut TestAppContext,
        win: AnyWindowHandle,
        id: (&'static str, usize),
        dx: f32,
    ) {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let b = w.find(id).bounds();
            let y = b.center().y;
            let start = point(b.left() + gpui_kit::px(dx), y);
            w.dispatch_event(event_at(start, None), cx);
            let down = MouseDownEvent {
                button: MouseButton::Left,
                position: start,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            };
            w.dispatch_event(down.to_platform_input(), cx);
        })
        .unwrap();
        // Each platform event is its own turn, so deferred work from the press runs first.
        cx.run_until_parked();
        cx.update_window(win, |_, w, cx| {
            let b = w.find(id).bounds();
            let y = b.center().y;
            let start = point(b.left() + gpui_kit::px(dx), y);
            for step in 1..=8 {
                let p = point(start.x + gpui_kit::px(12. * step as f32), y);
                w.dispatch_event(event_at(p, Some(MouseButton::Left)), cx);
                w.render_frame(cx);
            }
            let up = MouseUpEvent {
                button: MouseButton::Left,
                position: start,
                modifiers: Default::default(),
                click_count: 1,
            };
            w.dispatch_event(up.to_platform_input(), cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    pub fn focused(cx: &mut TestAppContext, win: AnyWindowHandle, s: &Entity<InputState>) -> bool {
        cx.update_window(win, |_, w, cx| s.read(cx).focus_handle(cx).is_focused(w))
            .unwrap()
    }
}

fn fixed_id_row() -> KvRow {
    KvRow {
        fixed: true,
        tag: Some("path".into()),
        ..row("identifier", "")
    }
}

#[gpui_kit::test]
fn a_press_in_the_edge_of_a_locked_fixed_name_focuses_it(cx: &mut TestAppContext) {
    let (win, view) = host(cx, vec![fixed_id_row()]);
    let table = table_of(cx, &view);
    let input = table
        .read_with(cx, |t, _| t.name_input(0).cloned())
        .unwrap();
    edge::click_in(cx, win, ("kv-name", 0), 4., 1);
    assert!(edge::focused(cx, win, &input), "a 4px press did not focus");
}

#[gpui_kit::test]
fn a_press_and_drag_from_the_edge_selects_in_a_locked_fixed_name(cx: &mut TestAppContext) {
    let (win, view) = host(cx, vec![fixed_id_row()]);
    let table = table_of(cx, &view);
    let input = table
        .read_with(cx, |t, _| t.name_input(0).cloned())
        .unwrap();
    edge::drag_in(cx, win, ("kv-name", 0), 4.);
    assert!(edge::focused(cx, win, &input), "drag lost focus");
    let range = input.read_with(cx, |s, _| s.selected_range());
    assert!(
        !range.is_empty(),
        "drag from the edge selected nothing: {range:?}"
    );
}

#[gpui_kit::test]
fn a_double_click_in_the_edge_selects_the_word_in_a_locked_fixed_name(cx: &mut TestAppContext) {
    let (win, view) = host(cx, vec![fixed_id_row()]);
    let table = table_of(cx, &view);
    let input = table
        .read_with(cx, |t, _| t.name_input(0).cloned())
        .unwrap();
    edge::click_in(cx, win, ("kv-name", 0), 4., 2);
    assert_eq!(input.read_with(cx, |s, _| s.selected_range()), 0..10);
}

#[gpui_kit::test]
fn edge_presses_behave_the_same_in_an_editable_cell(cx: &mut TestAppContext) {
    let (win, view) = host(cx, vec![row("a", "identifier")]);
    let table = table_of(cx, &view);
    let input = table
        .read_with(cx, |t, _| t.value_input(0).cloned())
        .unwrap();
    edge::click_in(cx, win, ("kv-value", 0), 4., 1);
    assert!(edge::focused(cx, win, &input), "a 4px press did not focus");
    edge::click_in(cx, win, ("kv-value", 0), 4., 2);
    assert_eq!(
        input.read_with(cx, |s, _| s.selected_range()),
        0..10,
        "double click"
    );
    edge::drag_in(cx, win, ("kv-value", 0), 4.);
    assert!(edge::focused(cx, win, &input), "drag lost focus");
    assert!(
        !input.read_with(cx, |s, _| s.selected_range()).is_empty(),
        "drag selected nothing"
    );
}

#[gpui_kit::test]
fn a_press_in_the_middle_still_focuses_a_locked_cell(cx: &mut TestAppContext) {
    let (win, view) = host(cx, vec![fixed_id_row()]);
    let table = table_of(cx, &view);
    let input = table
        .read_with(cx, |t, _| t.name_input(0).cloned())
        .unwrap();
    edge::click_in(cx, win, ("kv-name", 0), 30., 1);
    assert!(edge::focused(cx, win, &input));
}

#[gpui_kit::test]
fn a_press_in_the_right_padding_of_a_long_value_focuses_it(cx: &mut TestAppContext) {
    let long = "x".repeat(400);
    let (win, view) = host(cx, vec![row("a", &long)]);
    let table = table_of(cx, &view);
    let input = table
        .read_with(cx, |t, _| t.value_input(0).cloned())
        .unwrap();
    let width = cx
        .update_window(win, |_, w, _| {
            f32::from(w.find(("kv-value", 0usize)).bounds().size.width)
        })
        .unwrap();
    edge::click_in(cx, win, ("kv-value", 0), width - 4., 1);
    assert!(
        edge::focused(cx, win, &input),
        "a right-edge press did not focus"
    );
}

#[gpui_kit::test]
fn a_marked_name_cell_shows_its_danger_edge_unhovered_and_focused(cx: &mut TestAppContext) {
    let (win, view) = host(cx, vec![row("a", "1"), row("b", "2")]);
    let table = table_of(cx, &view);
    table.update(cx, |t, cx| {
        t.set_error(
            Some("`headers[1].name` cannot be written".into()),
            Some(1),
            cx,
        )
    });
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    assert!(edge(cx, win, ("kv-name-danger-edge", 1)), "unhovered");
    assert!(!shown(cx, win, [("kv-name-danger-edge", 0)])[0]);
    cx.update_window(win, |_, w, cx| {
        w.click(("kv-name", 1usize), cx);
        w.render_frame(cx)
    })
    .unwrap();
    assert!(edge(cx, win, ("kv-name-danger-edge", 1)), "focused");
    // A row that does not exist marks nothing.
    table.update(cx, |t, cx| t.set_error(Some("x".into()), Some(9), cx));
    assert!(!shown(cx, win, [("kv-name-danger-edge", 1)])[0]);
}

// A load that drops the focused row puts focus where a delete would: the last row left when
// several trailing rows go at once, as an undone multi-row paste does.
#[gpui_kit::test]
fn a_load_dropping_focused_trailing_rows_focuses_the_last_row_left(cx: &mut TestAppContext) {
    let (win, view) = host(cx, vec![row("a", "1"), row("b", "2"), row("c", "3")]);
    let table = table_of(cx, &view);
    cx.update_window(win, |_, w, cx| w.click(("kv-name", 2usize), cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        table.update(cx, |t, cx| t.load(vec![row("a", "1")], w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(support::holds_focus(cx, win, ("kv-name", 0usize)));
}

// A focused checkbox counts as focus in its row, so a load dropping that row keeps focus too.
#[gpui_kit::test]
fn a_load_dropping_a_row_with_its_checkbox_focused_keeps_focus(cx: &mut TestAppContext) {
    let (win, view) = host(cx, vec![row("a", "1"), row("b", "2")]);
    let table = table_of(cx, &view);
    cx.update_window(win, |_, w, cx| w.click(("kv-enabled", 1usize), cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        table.update(cx, |t, cx| t.load(vec![row("a", "1")], w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(support::holds_focus(cx, win, ("kv-name", 0usize)));
}

fn load_rows(cx: &mut TestAppContext, win: AnyWindowHandle, view: &Entity<Host>, rows: Vec<KvRow>) {
    let table = view.read_with(cx, |h, _| h.table.clone());
    cx.update_window(win, |_, w, cx| {
        table.update(cx, |t, cx| t.load(rows, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
}

// A reload leaves a focused Remove's surviving row fixed, so no Remove there can take focus.
// The pending focus must not outlive that draw and steal a later cell focus.
#[gpui_kit::test]
fn a_remove_focus_a_reload_could_not_place_never_fires_later(cx: &mut TestAppContext) {
    let (win, view) = host(cx, vec![row("a", "1"), row("x", "2"), row("y", "3")]);
    cx.update_window(win, |_, w, cx| w.click(("kv-name", 0usize), cx))
        .unwrap();
    for _ in 0..40 {
        if support::holds_focus(cx, win, ("kv-delete", 2usize)) {
            break;
        }
        cx.simulate_keystrokes(win, "tab");
        cx.run_until_parked();
    }
    assert!(support::holds_focus(cx, win, ("kv-delete", 2usize)));
    let fixed_y = KvRow {
        fixed: true,
        ..row("y", "3")
    };
    load_rows(cx, win, &view, vec![row("a", "1"), fixed_y]);
    cx.update_window(win, |_, w, cx| w.click(("kv-name", 0usize), cx))
        .unwrap();
    cx.run_until_parked();
    load_rows(cx, win, &view, vec![row("a", "1"), row("y", "3")]);
    assert!(support::holds_focus(cx, win, ("kv-name", 0usize)));
}

/// Load `rows` `loads` times in one update, as the app's notices can before its next frame.
fn load_before_a_draw(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    view: &Entity<Host>,
    rows: Vec<KvRow>,
    loads: usize,
) {
    let table = table_of(cx, view);
    cx.update_window(win, |_, w, cx| {
        for _ in 0..loads {
            table.update(cx, |t, cx| t.load(rows.clone(), w, cx));
        }
    })
    .unwrap();
    cx.run_until_parked();
}

fn tab_to(cx: &mut TestAppContext, win: AnyWindowHandle, id: (&'static str, usize)) {
    for _ in 0..40 {
        if support::holds_focus(cx, win, id) {
            return;
        }
        cx.simulate_keystrokes(win, "tab");
        cx.run_until_parked();
    }
    panic!("Tab never reached {id:?}");
}

// An undone delete of a row above brings it back: the row whose `control` held focus moves
// one slot down, and the control keeps focus there.
fn insert_above_keeps(cx: &mut TestAppContext, control: &'static str, loads: usize) {
    let (win, view) = host(cx, vec![row("a", "1"), row("c", "3")]);
    cx.update_window(win, |_, w, cx| w.click(("kv-name", 0usize), cx))
        .unwrap();
    tab_to(cx, win, (control, 1));
    let abc = vec![row("a", "1"), row("b", "2"), row("c", "3")];
    load_before_a_draw(cx, win, &view, abc, loads);
    assert!(support::holds_focus(cx, win, (control, 2usize)));
}

#[gpui_kit::test]
fn a_reload_inserting_a_row_above_keeps_focus_on_the_shifted_rows_remove(cx: &mut TestAppContext) {
    insert_above_keeps(cx, "kv-delete", 1);
}

#[gpui_kit::test]
fn two_reloads_inserting_a_row_above_keep_focus_on_the_shifted_rows_remove(
    cx: &mut TestAppContext,
) {
    insert_above_keeps(cx, "kv-delete", 2);
}

#[gpui_kit::test]
fn a_reload_inserting_a_row_above_keeps_focus_on_the_shifted_rows_checkbox(
    cx: &mut TestAppContext,
) {
    insert_above_keeps(cx, "kv-enabled", 1);
}

#[gpui_kit::test]
fn two_reloads_inserting_a_row_above_keep_focus_on_the_shifted_rows_checkbox(
    cx: &mut TestAppContext,
) {
    insert_above_keeps(cx, "kv-enabled", 2);
}

// An undone move puts row a back last: its Remove keeps focus at the new place.
fn move_keeps_remove(cx: &mut TestAppContext, loads: usize) {
    let (win, view) = host(cx, vec![row("a", "1"), row("b", "2"), row("c", "3")]);
    cx.update_window(win, |_, w, cx| w.click(("kv-name", 0usize), cx))
        .unwrap();
    tab_to(cx, win, ("kv-delete", 0));
    let bca = vec![row("b", "2"), row("c", "3"), row("a", "1")];
    load_before_a_draw(cx, win, &view, bca, loads);
    assert!(support::holds_focus(cx, win, ("kv-delete", 2usize)));
}

#[gpui_kit::test]
fn a_reload_moving_the_row_keeps_focus_on_its_remove(cx: &mut TestAppContext) {
    move_keeps_remove(cx, 1);
}

#[gpui_kit::test]
fn two_reloads_moving_the_row_keep_focus_on_its_remove(cx: &mut TestAppContext) {
    move_keeps_remove(cx, 2);
}

// A disk write that inserts n above b and edits c, read by one load or by two before a
// draw: b's Remove keeps focus, one slot down.
fn insert_and_edit_keeps_remove(cx: &mut TestAppContext, edited: Vec<KvRow>, loads: usize) {
    let (win, view) = host(
        cx,
        vec![row("a", "1"), row("m", "2"), row("b", "3"), row("c", "4")],
    );
    cx.update_window(win, |_, w, cx| w.click(("kv-name", 0usize), cx))
        .unwrap();
    tab_to(cx, win, ("kv-delete", 2));
    load_before_a_draw(cx, win, &view, edited, loads);
    assert!(support::holds_focus(cx, win, ("kv-delete", 3usize)));
}

fn amnbc_edited_below() -> Vec<KvRow> {
    let c = row("c", "changed");
    vec![
        row("a", "1"),
        row("m", "2"),
        row("n", "9"),
        row("b", "3"),
        c,
    ]
}

fn amnbc_edited_above() -> Vec<KvRow> {
    let m = row("m", "changed");
    vec![
        row("a", "1"),
        m,
        row("n", "9"),
        row("b", "3"),
        row("c", "4"),
    ]
}

#[gpui_kit::test]
fn a_reload_inserting_above_and_editing_below_keeps_focus_on_the_rows_remove(
    cx: &mut TestAppContext,
) {
    insert_and_edit_keeps_remove(cx, amnbc_edited_below(), 1);
}

#[gpui_kit::test]
fn two_reloads_inserting_above_and_editing_below_keep_focus_on_the_rows_remove(
    cx: &mut TestAppContext,
) {
    insert_and_edit_keeps_remove(cx, amnbc_edited_below(), 2);
}

#[gpui_kit::test]
fn a_reload_inserting_and_editing_above_keeps_focus_on_the_rows_remove(cx: &mut TestAppContext) {
    insert_and_edit_keeps_remove(cx, amnbc_edited_above(), 1);
}

#[gpui_kit::test]
fn two_reloads_inserting_and_editing_above_keep_focus_on_the_rows_remove(cx: &mut TestAppContext) {
    insert_and_edit_keeps_remove(cx, amnbc_edited_above(), 2);
}

// After an insert above moves focus from c's Remove to its new slot, the frames that follow
// draw the focus ring there only, not on the Remove now at c's old index.
fn insert_above_moves_the_ring(cx: &mut TestAppContext, loads: usize) {
    let (win, view) = host(cx, vec![row("a", "1"), row("c", "3")]);
    cx.update_window(win, |_, w, cx| w.click(("kv-name", 0usize), cx))
        .unwrap();
    tab_to(cx, win, ("kv-delete", 1));
    cx.update(|cx| apinata_app::focus_mode::set(apinata_app::focus_mode::Modality::Keyboard, cx));
    support::frame(cx, win);
    let abc = vec![row("a", "1"), row("b", "2"), row("c", "3")];
    load_before_a_draw(cx, win, &view, abc, loads);
    assert!(support::ring_drawn(cx, win, ("kv-delete", 2usize)));
    assert!(!support::ring_drawn(cx, win, ("kv-delete", 1usize)));
}

#[gpui_kit::test]
fn a_reload_inserting_above_leaves_no_ring_on_the_old_remove(cx: &mut TestAppContext) {
    insert_above_moves_the_ring(cx, 1);
}

#[gpui_kit::test]
fn two_reloads_inserting_above_leave_no_ring_on_the_old_remove(cx: &mut TestAppContext) {
    insert_above_moves_the_ring(cx, 2);
}

// Twin rows: a reload turning [A, A, B] into [A, B, A] while row 0's Remove holds focus keeps
// it on row 0, the alignment nearest its old place, through one load or two before a draw.
fn twin_rows_keep_remove(cx: &mut TestAppContext, loads: usize) {
    let (win, view) = host(cx, vec![row("A", "1"), row("A", "1"), row("B", "2")]);
    cx.update_window(win, |_, w, cx| w.click(("kv-name", 0usize), cx))
        .unwrap();
    tab_to(cx, win, ("kv-delete", 0));
    let aba = vec![row("A", "1"), row("B", "2"), row("A", "1")];
    load_before_a_draw(cx, win, &view, aba, loads);
    assert!(support::holds_focus(cx, win, ("kv-delete", 0usize)));
}

#[gpui_kit::test]
fn a_reload_reordering_twin_rows_keeps_focus_on_the_first_rows_remove(cx: &mut TestAppContext) {
    twin_rows_keep_remove(cx, 1);
}

#[gpui_kit::test]
fn two_reloads_reordering_twin_rows_keep_focus_on_the_first_rows_remove(cx: &mut TestAppContext) {
    twin_rows_keep_remove(cx, 2);
}

fn typed(kind: &str) -> Variable {
    Variable::new(
        "n",
        VarValue::Typed {
            kind: kind.into(),
            data: "1".into(),
        },
    )
}

// A reload that changes only what the table does not show (here a variable's declared type)
// still makes rows built before it lag, so an edit from them is never taken as current.
#[gpui_kit::test]
fn rows_with_a_different_entry_lag_though_every_shown_field_matches(cx: &mut TestAppContext) {
    let (_, view) = host(cx, var_rows(&[typed("number")]));
    let table = view.read_with(cx, |h, _| h.table.clone());
    let older = var_rows(&[typed("boolean")]);
    let same = var_rows(&[typed("number")]);
    assert_eq!(older, same, "the rows match in every shown field");
    cx.update(|cx| {
        assert!(apinata_app::kv_table::lags(&table, &older, cx));
        assert!(!apinata_app::kv_table::lags(&table, &same, cx));
    });
}

#[test]
fn a_row_entry_is_the_same_only_for_an_equal_entry_of_the_same_type() {
    use apinata_app::kv_table::RowEntry;
    let number = RowEntry::new(typed("number"));
    assert!(number.same(&RowEntry::new(typed("number"))));
    assert!(!number.same(&RowEntry::new(typed("boolean"))));
    assert!(!number.same(&RowEntry::new(KeyValue::new("n", "1"))));
    assert!(
        number == RowEntry::new(typed("boolean")),
        "PartialEq still ignores the entry"
    );
}

#[gpui_kit::test]
fn a_row_with_an_entry_lags_a_row_without_one(cx: &mut TestAppContext) {
    let (_, view) = host(cx, var_rows(&[typed("number")]));
    let table = view.read_with(cx, |h, _| h.table.clone());
    let mut bare = var_rows(&[typed("number")]);
    bare[0].entry = None;
    cx.update(|cx| assert!(apinata_app::kv_table::lags(&table, &bare, cx)));
}
