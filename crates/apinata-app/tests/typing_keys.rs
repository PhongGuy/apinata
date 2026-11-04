//! Which cell the user is typing in, and the key context of the structured rows.

use crate::support;

use apinata_app::assertions_section::AssertionsSection;
use apinata_app::kv_table::{KvEvent, KvRow, KvTable};
use apinata_core::Assertion;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Context, Entity, Focusable, InteractiveElement, IntoElement,
    KeyBinding, ParentElement, Render, Styled, Subscription, TestAppContext, Window, WindowOptions,
    actions, div,
};
use support::*;

actions!(typing_probe, [InRows, AnyRows]);

struct Host {
    table: Entity<KvTable>,
    in_rows: usize,
    any_rows: usize,
    _sub: Subscription,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .on_action(cx.listener(|h, _: &InRows, _, _| h.in_rows += 1))
            .on_action(cx.listener(|h, _: &AnyRows, _, _| h.any_rows += 1))
            .child(self.table.clone())
    }
}

fn kv(name: &str, value: &str) -> KvRow {
    KvRow {
        name: name.into(),
        value: value.into(),
        enabled: true,
        ..KvRow::default()
    }
}

fn host(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Host>, Entity<KvTable>) {
    support::init(cx);
    cx.update(|cx| {
        cx.bind_keys([
            KeyBinding::new("ctrl-alt-q", InRows, Some("StructuredRows > Input")),
            KeyBinding::new("ctrl-alt-w", AnyRows, Some("StructuredRows")),
        ])
    });
    let (win, host) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| {
                    let table = cx.new(|cx| KvTable::new(w, cx));
                    table.update(cx, |t, cx| t.load(vec![kv("a", "1"), kv("b", "2")], w, cx));
                    let sub = cx.subscribe(&table, |_: &mut Host, _, _: &KvEvent, _| {});
                    Host {
                        table,
                        in_rows: 0,
                        any_rows: 0,
                        _sub: sub,
                    }
                })
            })
        })
        .unwrap();
    frame(cx, win);
    let table = host.read_with(cx, |h, _| h.table.clone());
    (win, host, table)
}

fn typed(cx: &mut TestAppContext, t: &Entity<KvTable>) -> Option<(usize, u8)> {
    t.read_with(cx, |t, _| t.last_typed())
}

fn click(cx: &mut TestAppContext, win: AnyWindowHandle, id: impl Into<gpui_kit::ElementId>) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(id, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

fn type_in(cx: &mut TestAppContext, win: AnyWindowHandle, id: (&'static str, usize), text: &str) {
    cx.update_window(win, |_, w, cx| {
        w.click(id, cx);
        w.input(text, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn typing_in_a_cell_records_its_row_and_column(cx: &mut TestAppContext) {
    let (win, _h, table) = host(cx);
    assert_eq!(typed(cx, &table), None);
    type_in(cx, win, ("kv-name", 0), "x");
    assert_eq!(typed(cx, &table), Some((0, 0)));
    type_in(cx, win, ("kv-value", 1), "y");
    assert_eq!(typed(cx, &table), Some((1, 1)));
}

#[gpui_kit::test]
fn the_first_keystroke_in_the_empty_row_names_the_new_row(cx: &mut TestAppContext) {
    let (win, _h, table) = host(cx);
    type_in(cx, win, ("kv-name", 2), "n");
    assert_eq!(typed(cx, &table), Some((2, 0)));
}

#[gpui_kit::test]
fn delete_toggle_and_toggle_all_clear_it(cx: &mut TestAppContext) {
    let (win, _h, table) = host(cx);
    type_in(cx, win, ("kv-name", 0), "x");
    click(cx, win, ("kv-enabled", 1usize));
    assert_eq!(typed(cx, &table), None);
    type_in(cx, win, ("kv-name", 0), "y");
    assert!(typed(cx, &table).is_some());
    click(cx, win, "kv-head-enabled");
    assert_eq!(typed(cx, &table), None);
    type_in(cx, win, ("kv-name", 0), "z");
    click(cx, win, ("kv-delete", 1usize));
    assert_eq!(typed(cx, &table), None);
}

#[gpui_kit::test]
fn setting_files_clears_it(cx: &mut TestAppContext) {
    let (win, _h, table) = host(cx);
    type_in(cx, win, ("kv-name", 0), "x");
    cx.update_window(win, |_, w, cx| {
        table.update(cx, |t, cx| t.set_files(0, Some(vec!["f".into()]), w, cx))
    })
    .unwrap();
    assert_eq!(typed(cx, &table), None);
}

#[gpui_kit::test]
fn the_rows_carry_the_structured_rows_context_but_not_the_bulk_editor(cx: &mut TestAppContext) {
    let (win, h, table) = host(cx);
    cx.update_window(win, |_, w, cx| {
        w.click(("kv-name", 0usize), cx);
        w.press("ctrl-alt-q", cx);
    })
    .unwrap();
    assert_eq!(h.read_with(cx, |h, _| h.in_rows), 1);
    cx.update_window(win, |_, w, cx| {
        table.update(cx, |t, cx| t.set_bulk(true, w, cx));
        let editor = table.read(cx).bulk_editor().unwrap().clone();
        editor.read(cx).focus_handle(cx).focus(w, cx);
    })
    .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.press("ctrl-alt-q", cx);
        w.press("ctrl-alt-w", cx);
    })
    .unwrap();
    assert_eq!(h.read_with(cx, |h, _| (h.in_rows, h.any_rows)), (1, 0));
}

fn assertion_host(
    cx: &mut TestAppContext,
) -> (
    AnyWindowHandle,
    Entity<SectionHost<AssertionsSection>>,
    Entity<AssertionsSection>,
) {
    let (win, host) = host_section(cx, AssertionsSection::new);
    let s = host.read_with(cx, |h, _| h.view.clone());
    let base = vec![Assertion::new("res.status", "eq", Some("200".into()))];
    cx.update_window(win, |_, w, cx| s.update(cx, |s, cx| s.load(&base, w, cx)))
        .unwrap();
    frame(cx, win);
    (win, host, s)
}

#[gpui_kit::test]
fn assertion_typing_is_keyed_and_sent_with_the_edit(cx: &mut TestAppContext) {
    let (win, host, s) = assertion_host(cx);
    type_in(cx, win, ("as-expr", 0), "x");
    assert_eq!(s.read_with(cx, |s, _| s.last_typed()), Some((0, 0)));
    type_in(cx, win, ("as-value", 0), "1");
    assert_eq!(s.read_with(cx, |s, _| s.last_typed()), Some((0, 1)));
    let last = host.read_with(cx, |h, _| h.typed.last().copied());
    assert_eq!(last, Some(Some((0, 1))));
}

#[gpui_kit::test]
fn assertion_toggle_delete_operator_and_add_clear_the_key(cx: &mut TestAppContext) {
    let (win, host, s) = assertion_host(cx);
    let typed = |cx: &mut TestAppContext| s.read_with(cx, |s, _| s.last_typed());
    type_in(cx, win, ("as-expr", 0), "x");
    click(cx, win, ("as-enabled", 0usize));
    assert_eq!(typed(cx), None);
    assert_eq!(
        host.read_with(cx, |h, _| h.typed.last().copied()),
        Some(None)
    );
    type_in(cx, win, ("as-expr", 0), "y");
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.set_operator(0, "neq", w, cx))
    })
    .unwrap();
    assert_eq!(typed(cx), None);
    type_in(cx, win, ("as-expr", 0), "z");
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| {
            s.add_assertion(&Assertion::new("a", "eq", Some("b".into())), w, cx)
        })
    })
    .unwrap();
    assert_eq!(typed(cx), None);
    type_in(cx, win, ("as-expr", 0), "q");
    click(cx, win, ("as-delete", 1usize));
    assert_eq!(typed(cx), None);
}

#[gpui_kit::test]
fn assertion_rows_carry_the_structured_rows_context(cx: &mut TestAppContext) {
    support::init(cx);
    cx.update(|cx| {
        cx.bind_keys([KeyBinding::new(
            "ctrl-alt-q",
            InRows,
            Some("StructuredRows > Input"),
        )])
    });
    let (win, _host, _s) = assertion_host(cx);
    // The probe action has no handler here, so the check is that the context exists at all.
    cx.update_window(win, |_, w, cx| {
        w.click(("as-expr", 0usize), cx);
        assert!(
            w.context_stack()
                .iter()
                .any(|c| c.contains("StructuredRows")),
            "{:?}",
            w.context_stack()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_form_body_forwards_the_tables_typing_key(cx: &mut TestAppContext) {
    use apinata_app::body_section::BodySection;
    use apinata_core::{BodySpec, KeyValue};
    let (win, host) = host_section(cx, |w, cx| {
        BodySection::new(std::path::PathBuf::from("/c"), w, cx)
    });
    let s = host.read_with(cx, |h, _| h.view.clone());
    let spec = BodySpec::FormUrlEncoded(vec![KeyValue::new("a", "1")]);
    cx.update_window(win, |_, w, cx| s.update(cx, |s, cx| s.load(&spec, w, cx)))
        .unwrap();
    frame(cx, win);
    type_in(cx, win, ("kv-value", 0), "x");
    assert_eq!(
        host.read_with(cx, |h, _| h.typed.last().copied()),
        Some(Some((0, 1)))
    );
}
