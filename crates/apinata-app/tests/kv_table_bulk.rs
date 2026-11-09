//! The table's bulk text mode: toggle, emit, echo, and where it is offered.

use crate::support;

use apinata_app::kv_table::{KvColumns, KvEvent, KvRow, KvTable, RowEntry};
use apinata_core::KeyValue;
use gpui_kit::Focusable;
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
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.table.clone())
    }
}

fn row(name: &str, value: &str, source: usize) -> KvRow {
    KvRow {
        name: name.into(),
        value: value.into(),
        enabled: true,
        source: Some(source),
        entry: Some(RowEntry::new(KeyValue::new(name, value))),
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

fn table(cx: &mut TestAppContext, h: &Entity<Host>) -> Entity<KvTable> {
    h.read_with(cx, |h, _| h.table.clone())
}

// Replaces the bulk text by typing, pressing Enter between lines as a user would.
fn type_lines(cx: &mut TestAppContext, win: AnyWindowHandle, lines: &[&str]) {
    replace_code(cx, win, None, "kv-bulk-text", lines[0]);
    for line in &lines[1..] {
        cx.update_window(win, |_, w, cx| {
            w.press("enter", cx);
            w.input(line, cx);
        })
        .unwrap();
    }
}

fn bulk_value(cx: &mut TestAppContext, t: &Entity<KvTable>) -> String {
    t.read_with(cx, |t, cx| {
        t.bulk_editor()
            .expect("bulk on")
            .read(cx)
            .value()
            .to_string()
    })
}

#[gpui_kit::test]
fn the_toggle_shows_the_rows_as_text_and_back(cx: &mut TestAppContext) {
    let (win, h) = host(
        cx,
        vec![
            row("a", "1", 0),
            KvRow {
                enabled: false,
                ..row("b", "2", 1)
            },
        ],
    );
    let t = table(cx, &h);
    cx.update_window(win, |_, w, cx| w.click("kv-bulk-toggle", cx))
        .unwrap();
    frame(cx, win);
    assert!(t.read_with(cx, |t, _| t.bulk()));
    assert_eq!(bulk_value(cx, &t), "a: 1\n//b: 2");
    assert!(present(cx, win, "kv-bulk-text"));
    assert!(
        h.read_with(cx, |h, _| h.events.is_empty()),
        "toggling alone does not edit"
    );
    cx.update_window(win, |_, w, cx| w.click("kv-bulk-toggle", cx))
        .unwrap();
    frame(cx, win);
    assert!(!t.read_with(cx, |t, _| t.bulk()));
    assert!(!present(cx, win, "kv-bulk-text"));
}

#[gpui_kit::test]
fn typing_in_bulk_mode_emits_matched_rows(cx: &mut TestAppContext) {
    let (win, h) = host(cx, vec![row("a", "1", 0), row("b", "2", 1)]);
    let t = table(cx, &h);
    cx.update_window(win, |_, w, cx| w.click("kv-bulk-toggle", cx))
        .unwrap();
    frame(cx, win);
    type_lines(cx, win, &["b: 20", "//a: 1", "c: 3"]);
    cx.run_until_parked();
    let last = h.read_with(cx, |h, _| h.events.last().cloned().expect("an event"));
    let got: Vec<_> = last
        .iter()
        .map(|r| (r.name.as_str(), r.value.as_str(), r.enabled, r.source))
        .collect();
    assert_eq!(
        got,
        [
            ("b", "20", true, Some(1)),
            ("a", "1", false, Some(0)),
            ("c", "3", true, None)
        ]
    );
    // Matched rows carry their entry through the table, a new row has none.
    let entries: Vec<_> = last.iter().map(|r| r.entry.is_some()).collect();
    assert_eq!(entries, [true, true, false]);
    // The table behind the text follows, so switching back shows the edit.
    let shown = t.read_with(cx, |t, cx| t.rows(cx));
    assert_eq!(shown, last);
    assert_eq!(
        shown.iter().map(|r| r.entry.is_some()).collect::<Vec<_>>(),
        [true, true, false]
    );
}

#[gpui_kit::test]
fn typing_in_bulk_mode_survives_the_echo(cx: &mut TestAppContext) {
    let (win, h) = host(cx, vec![row("a", "1", 0)]);
    let t = table(cx, &h);
    cx.update_window(win, |_, w, cx| w.click("kv-bulk-toggle", cx))
        .unwrap();
    frame(cx, win);
    type_lines(cx, win, &["a: 1", "b: 2", "no colon yet"]);
    cx.run_until_parked();
    // The owner writes the rows and reloads them with fresh sources, as a snapshot would.
    let echoed = vec![row("a", "1", 0), row("b", "2", 1)];
    cx.update_window(win, |_, w, cx| {
        t.update(cx, |t, cx| t.load(echoed.clone(), w, cx))
    })
    .unwrap();
    frame(cx, win);
    assert_eq!(
        bulk_value(cx, &t),
        "a: 1\nb: 2\nno colon yet",
        "the text is not reset"
    );
    assert_eq!(t.read_with(cx, |t, cx| t.rows(cx)), echoed);
    // A load that means something else (an external change) replaces the text.
    cx.update_window(win, |_, w, cx| {
        t.update(cx, |t, cx| t.load(vec![row("z", "9", 0)], w, cx))
    })
    .unwrap();
    frame(cx, win);
    assert_eq!(bulk_value(cx, &t), "z: 9");
}

#[gpui_kit::test]
fn a_locked_or_nameless_table_offers_no_bulk_edit(cx: &mut TestAppContext) {
    let (win, h) = host(cx, vec![row("a", "1", 0)]);
    let t = table(cx, &h);
    t.update(cx, |t, cx| t.set_locked(true, cx));
    frame(cx, win);
    assert!(!present(cx, win, "kv-bulk-toggle"));
    t.update(cx, |t, cx| t.set_locked(false, cx));
    t.update(cx, |t, cx| t.set_columns(KvColumns::FILE_BODY, cx));
    frame(cx, win);
    assert!(!present(cx, win, "kv-bulk-toggle"));
    cx.update_window(win, |_, w, cx| {
        t.update(cx, |t, cx| t.set_bulk(true, w, cx))
    })
    .unwrap();
    assert!(!t.read_with(cx, |t, _| t.bulk()));
}

#[gpui_kit::test]
fn an_echo_load_refreshes_the_rows_left_out_of_the_text(cx: &mut TestAppContext) {
    let fixed = |name: &str| KvRow {
        fixed: true,
        ..row(name, "", 1)
    };
    let (win, h) = host(cx, vec![row("a", "1", 0), fixed("id")]);
    let t = table(cx, &h);
    cx.update_window(win, |_, w, cx| w.click("kv-bulk-toggle", cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(bulk_value(cx, &t), "a: 1");
    // The owner renames the path param elsewhere; the text's rows are unchanged, so it is an echo.
    let renamed = vec![row("a", "1", 0), fixed("uid")];
    cx.update_window(win, |_, w, cx| t.update(cx, |t, cx| t.load(renamed, w, cx)))
        .unwrap();
    frame(cx, win);
    assert_eq!(bulk_value(cx, &t), "a: 1");
    type_lines(cx, win, &["a: 2"]);
    cx.run_until_parked();
    let last = h.read_with(cx, |h, _| h.events.last().cloned().expect("an event"));
    let got: Vec<_> = last
        .iter()
        .map(|r| (r.name.as_str(), r.value.as_str(), r.fixed))
        .collect();
    assert_eq!(got, [("a", "2", false), ("uid", "", true)]);
}

fn fixed_row(name: &str, source: usize) -> KvRow {
    KvRow {
        fixed: true,
        ..row(name, "", source)
    }
}

fn names(rows: &[KvRow]) -> Vec<&str> {
    rows.iter().map(|r| r.name.as_str()).collect()
}

#[gpui_kit::test]
fn a_query_row_added_after_bulk_edit_stays_below_the_path_row(cx: &mut TestAppContext) {
    let (win, h) = host(cx, vec![fixed_row("id", 0), row("a", "1", 1)]);
    let t = table(cx, &h);
    cx.update_window(win, |_, w, cx| w.click("kv-bulk-toggle", cx))
        .unwrap();
    frame(cx, win);
    type_lines(cx, win, &["a: 1", "b: 2"]);
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.click("kv-bulk-toggle", cx))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.click(("kv-name", 3usize), cx);
        w.input("c", cx)
    })
    .unwrap();
    cx.run_until_parked();
    let rows = t.read_with(cx, |t, cx| t.rows(cx));
    assert_eq!(names(&rows), ["id", "a", "b", "c"]);
}

#[gpui_kit::test]
fn bulk_edit_keeps_a_path_row_in_its_slot(cx: &mut TestAppContext) {
    let (win, h) = host(cx, vec![fixed_row("id", 0), row("a", "1", 1)]);
    cx.update_window(win, |_, w, cx| w.click("kv-bulk-toggle", cx))
        .unwrap();
    frame(cx, win);
    type_lines(cx, win, &["a: 1", "b: 2"]);
    cx.run_until_parked();
    let last = h.read_with(cx, |h, _| h.events.last().cloned().expect("an event"));
    assert_eq!(names(&last), ["id", "a", "b"]);
}

#[gpui_kit::test]
fn locking_or_reconfiguring_a_table_in_bulk_mode_keeps_focus_in_it(cx: &mut TestAppContext) {
    for reconfigure in [false, true] {
        let (win, h) = host(cx, vec![row("a", "1", 0)]);
        let t = table(cx, &h);
        cx.update_window(win, |_, w, cx| w.click("kv-bulk-toggle", cx))
            .unwrap();
        frame(cx, win);
        t.update(cx, |t, cx| {
            if reconfigure {
                t.set_columns(KvColumns::FILE_BODY, cx)
            } else {
                t.set_locked(true, cx)
            }
        });
        frame(cx, win);
        assert!(!t.read_with(cx, |t, _| t.bulk()));
        let inside = cx
            .update_window(win, |_, w, cx| {
                t.read_with(cx, |t, cx| t.focus_handle(cx).contains_focused(w, cx))
            })
            .unwrap();
        assert!(
            inside,
            "focus lands in the table (reconfigure: {reconfigure})"
        );
    }
}

#[gpui_kit::test]
fn an_echo_load_keeps_the_path_row_in_its_slot(cx: &mut TestAppContext) {
    let (win, h) = host(cx, vec![fixed_row("id", 0), row("a", "1", 1)]);
    let t = table(cx, &h);
    cx.update_window(win, |_, w, cx| w.click("kv-bulk-toggle", cx))
        .unwrap();
    frame(cx, win);
    type_lines(cx, win, &["a: 1", "b: 2"]);
    cx.run_until_parked();
    let echoed = h.read_with(cx, |h, _| h.events.last().cloned().expect("an event"));
    cx.update_window(win, |_, w, cx| {
        t.update(cx, |t, cx| t.load(echoed.clone(), w, cx))
    })
    .unwrap();
    frame(cx, win);
    type_lines(cx, win, &["a: 1", "b: 2", "c: 3"]);
    cx.run_until_parked();
    let last = h.read_with(cx, |h, _| h.events.last().cloned().expect("an event"));
    assert_eq!(names(&last), ["id", "a", "b", "c"]);
}

// Whether element `id` holds focus in the freshly drawn window.
fn on(cx: &mut TestAppContext, win: AnyWindowHandle, id: impl Into<gpui_kit::ElementId>) -> bool {
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id).and_then(|e| e.focused())
    })
    .unwrap()
        == Some(true)
}

fn key_in(cx: &mut TestAppContext, win: AnyWindowHandle, key: &str) {
    cx.update_window(win, |_, w, cx| w.press(key, cx)).unwrap();
    cx.run_until_parked();
    frame(cx, win);
}

fn editor_focused(cx: &mut TestAppContext, win: AnyWindowHandle, t: &Entity<KvTable>) -> bool {
    cx.update_window(win, |_, w, cx| {
        t.read(cx)
            .bulk_editor()
            .is_some_and(|e| e.read(cx).focus_handle(cx).is_focused(w))
    })
    .unwrap()
}

#[gpui_kit::test]
fn tab_reaches_the_bulk_toggle_between_the_heading_and_the_first_row(cx: &mut TestAppContext) {
    let (win, _h) = host(cx, vec![row("a", "1", 0)]);
    tab_to(cx, win, "kv-head-enabled");
    key_in(cx, win, "tab");
    assert!(on(cx, win, "kv-bulk-toggle"), "Tab reaches the toggle");
    key_in(cx, win, "tab");
    assert!(on(cx, win, ("kv-enabled", 0usize)), "then the first row");
    key_in(cx, win, "shift-tab");
    assert!(on(cx, win, "kv-bulk-toggle"), "Shift+Tab comes back to it");
}

#[gpui_kit::test]
fn in_bulk_mode_the_toggle_sits_right_before_the_editor(cx: &mut TestAppContext) {
    let (win, h) = host(cx, vec![row("a", "1", 0)]);
    let t = table(cx, &h);
    cx.update_window(win, |_, w, cx| w.click("kv-bulk-toggle", cx))
        .unwrap();
    frame(cx, win);
    assert!(editor_focused(cx, win, &t), "a click opens the editor");
    // Shift+Tab outdents inside the editor; the app's chord leaves it backwards.
    cx.update(apinata_app::keymap::init);
    key_in(cx, win, "ctrl-shift-tab");
    assert!(
        on(cx, win, "kv-bulk-toggle"),
        "Ctrl+Shift+Tab from the editor"
    );
    key_in(cx, win, "tab");
    assert!(editor_focused(cx, win, &t), "Tab from the toggle");
}

#[gpui_kit::test]
fn enter_and_space_on_the_focused_bulk_toggle_switch_modes_both_ways(cx: &mut TestAppContext) {
    for key in ["enter", "space"] {
        let (win, h) = host(cx, vec![row("a", "1", 0)]);
        let t = table(cx, &h);
        tab_to(cx, win, "kv-bulk-toggle");
        key_in(cx, win, key);
        assert!(t.read_with(cx, |t, _| t.bulk()), "{key} turns bulk on");
        assert!(
            on(cx, win, "kv-bulk-toggle"),
            "{key}: focus stays on the toggle"
        );
        key_in(cx, win, key);
        assert!(!t.read_with(cx, |t, _| t.bulk()), "{key} turns bulk off");
        assert!(on(cx, win, "kv-bulk-toggle"), "{key}: focus stays again");
    }
}

const WITH_ROWS: &str = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://x/0?q=1\"\n  params:\n    - name: q\n      value: \"1\"\n      type: query\n  headers:\n    - name: H1\n      value: one\n";

// In the whole app, Enter or Space on the focused toggle of Headers or Params switches the
// mode and leaves focus on the toggle, so the same key switches back.
#[gpui_kit::test]
fn in_the_app_a_key_on_the_bulk_toggle_keeps_focus_on_it(cx: &mut TestAppContext) {
    use apinata_app::request_editor::Section;
    for section in [Section::Headers, Section::Params] {
        for key in ["enter", "space"] {
            let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", WITH_ROWS)]);
            let s = started(cx, &col);
            cx.update_window(s.win, |_, w, _| w.activate_window())
                .unwrap();
            open_tab(cx, &s.ws, &s.root, "a.yml");
            frame(cx, s.win);
            cx.update_window(s.win, |_, w, cx| w.click(("section", section as usize), cx))
                .unwrap();
            frame(cx, s.win);
            for _ in 0..64 {
                if on(cx, s.win, "kv-bulk-toggle") {
                    break;
                }
                key_in(cx, s.win, "tab");
            }
            assert!(on(cx, s.win, "kv-bulk-toggle"), "Tab reaches the toggle");
            real_key(cx, s.win, key);
            assert!(
                present(cx, s.win, "kv-bulk-text"),
                "{section:?} {key}: bulk on"
            );
            assert!(
                on(cx, s.win, "kv-bulk-toggle"),
                "{section:?} {key}: focus stays on the toggle"
            );
            real_key(cx, s.win, key);
            assert!(
                !present(cx, s.win, "kv-bulk-text"),
                "{section:?} {key}: bulk off"
            );
            assert!(
                on(cx, s.win, "kv-bulk-toggle"),
                "{section:?} {key}: focus stays again"
            );
        }
    }
}

// A key as the platform delivers it: a frame between key-down and key-up, and the effects of
// the key-up, deferred work included, settled before the next frame is drawn.
fn real_key(cx: &mut TestAppContext, win: AnyWindowHandle, key: &str) {
    use gpui_kit::{InputEvent as _, KeyDownEvent, KeyUpEvent, Keystroke};
    let stroke = Keystroke::parse_with_os(key, apinata_app::shortcuts::HOST_OS).unwrap();
    let down = KeyDownEvent {
        keystroke: stroke.clone(),
        is_held: false,
        prefer_character_input: false,
    };
    cx.update_window(win, |_, w, cx| {
        w.dispatch_event(down.to_platform_input(), cx)
    })
    .unwrap();
    frame(cx, win);
    let up = KeyUpEvent { keystroke: stroke };
    cx.update_window(win, |_, w, cx| w.dispatch_event(up.to_platform_input(), cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, win);
}
