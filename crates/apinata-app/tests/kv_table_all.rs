//! The heading checkbox that enables or disables every row at once.

use crate::support;

use apinata_app::kv_table::{KvEvent, KvRow, KvTable, Tri, enabled_state};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled,
    Subscription, TestAppContext, Window, WindowOptions, div,
};
use support::*;

struct Host {
    table: Entity<KvTable>,
    events: Vec<Vec<KvRow>>,
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
        value: "v".into(),
        enabled,
        fixed,
        ..KvRow::default()
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
    frame(cx, win);
    (win, view)
}

fn flags(cx: &mut TestAppContext, h: &Entity<Host>) -> Vec<bool> {
    h.read_with(cx, |h, _| {
        h.events
            .last()
            .expect("an event")
            .iter()
            .map(|r| r.enabled)
            .collect()
    })
}

#[test]
fn the_state_is_over_the_togglable_rows_only() {
    assert_eq!(enabled_state(&[]), Tri::Off);
    assert_eq!(
        enabled_state(&[row("a", true, false), row("b", true, false)]),
        Tri::On
    );
    assert_eq!(
        enabled_state(&[row("a", false, false), row("b", false, false)]),
        Tri::Off
    );
    assert_eq!(
        enabled_state(&[row("a", true, false), row("b", false, false)]),
        Tri::Mixed
    );
    // A fixed row has no toggle, so its flag does not count.
    assert_eq!(
        enabled_state(&[row("a", true, false), row("b", false, true)]),
        Tri::On
    );
}

#[gpui_kit::test]
fn clicking_the_heading_flips_all_rows_in_one_edit(cx: &mut TestAppContext) {
    let (win, h) = host(cx, vec![row("a", true, false), row("b", true, false)]);
    cx.update_window(win, |_, w, cx| w.click("kv-head-enabled", cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(h.read_with(cx, |h, _| h.events.len()), 1);
    assert_eq!(flags(cx, &h), [false, false]);
    cx.update_window(win, |_, w, cx| w.click("kv-head-enabled", cx))
        .unwrap();
    assert_eq!(h.read_with(cx, |h, _| h.events.len()), 2);
    assert_eq!(flags(cx, &h), [true, true]);
}

#[gpui_kit::test]
fn clicking_mixed_turns_everything_on_but_fixed_rows(cx: &mut TestAppContext) {
    let (win, h) = host(
        cx,
        vec![
            row("a", true, false),
            row("b", false, false),
            row("id", false, true),
        ],
    );
    cx.update_window(win, |_, w, cx| w.click("kv-head-enabled", cx))
        .unwrap();
    assert_eq!(flags(cx, &h), [true, true, false]);
}

#[gpui_kit::test]
fn a_locked_table_ignores_the_heading(cx: &mut TestAppContext) {
    let (win, h) = host(cx, vec![row("a", true, false), row("b", true, false)]);
    let t = h.read_with(cx, |h, _| h.table.clone());
    // Positive control: the same table unlocked answers the click.
    cx.update_window(win, |_, w, cx| w.click("kv-head-enabled", cx))
        .unwrap();
    assert_eq!(h.read_with(cx, |h, _| h.events.len()), 1);
    t.update(cx, |t, cx| t.set_locked(true, cx));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("kv-head-enabled", cx))
        .unwrap();
    assert_eq!(
        h.read_with(cx, |h, _| h.events.len()),
        1,
        "locked: no event"
    );
    assert_eq!(flags(cx, &h), [false, false]);
}

#[gpui_kit::test]
fn a_table_without_togglable_rows_emits_nothing(cx: &mut TestAppContext) {
    for rows in [vec![], vec![row("id", false, true)]] {
        let (win, h) = host(cx, rows);
        cx.update_window(win, |_, w, cx| w.click("kv-head-enabled", cx))
            .unwrap();
        assert!(h.read_with(cx, |h, _| h.events.is_empty()));
    }
}

#[gpui_kit::test]
fn the_empty_row_is_left_out(cx: &mut TestAppContext) {
    let (win, h) = host(cx, vec![row("a", false, false), row("b", false, false)]);
    let t = h.read_with(cx, |h, _| h.table.clone());
    cx.update_window(win, |_, w, cx| w.click("kv-head-enabled", cx))
        .unwrap();
    assert_eq!(flags(cx, &h), [true, true]);
    assert_eq!(t.read_with(cx, |t, cx| t.rows(cx).len()), 2);
    assert_eq!(t.read_with(cx, |t, _| t.enabled_state()), Tri::On);
}

#[gpui_kit::test]
fn the_heading_box_reports_its_state_and_name(cx: &mut TestAppContext) {
    let (win, _h) = host(cx, vec![row("a", false, false), row("b", false, false)]);
    let read = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let e = w.find("kv-head-enabled");
            (e.checked(), e.label().map(str::to_owned))
        })
        .unwrap()
    };
    assert_eq!(
        read(cx),
        (Some(false), Some("Enable all rows".into())),
        "all off at start"
    );
    cx.update_window(win, |_, w, cx| w.click("kv-head-enabled", cx))
        .unwrap();
    assert_eq!(read(cx).0, Some(true), "one click turns all on");
}

#[gpui_kit::test]
fn space_on_the_focused_heading_toggles(cx: &mut TestAppContext) {
    let (win, h) = host(cx, vec![row("a", true, false)]);
    tab_to(cx, win, "kv-head-enabled");
    cx.update_window(win, |_, w, cx| w.press("space", cx))
        .unwrap();
    assert_eq!(flags(cx, &h), [false]);
}
