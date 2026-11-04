//! Controls that were click-only: each is a Tab stop and answers Enter and Space.

use crate::support;

use apinata_app::body_view::BodyView;
use apinata_app::console_dock::{ConsoleDock, DockTab};
use apinata_app::keymap::ToggleConsole;
use apinata_app::kv_table::{KvEvent, KvRow, KvTable};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, ElementId, Entity, IntoElement, ParentElement as _,
    Render, Styled as _, Subscription, TestAppContext, VisualTestContext, Window, WindowOptions,
    div,
};
use support::*;
use wiremock::ResponseTemplate;

// Tabs forward until `id` holds focus; false when it never does.
fn tab_to(cx: &mut TestAppContext, win: AnyWindowHandle, id: impl Into<ElementId>) -> bool {
    let id = id.into();
    for _ in 0..60 {
        cx.update_window(win, |_, w, cx| w.render_frame(cx))
            .unwrap();
        let on = cx
            .update_window(win, |_, w, _| {
                w.try_find(id.clone()).and_then(|e| e.focused())
            })
            .unwrap();
        if on == Some(true) {
            return true;
        }
        cx.simulate_keystrokes(win, "tab");
        cx.run_until_parked();
    }
    false
}

fn press(cx: &mut TestAppContext, win: AnyWindowHandle, key: &str) {
    cx.update_window(win, |_, w, cx| w.press(key, cx)).unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
}

fn dock_with_rows(cx: &mut TestAppContext, tab: DockTab) -> (Started, Entity<ConsoleDock>) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let a = format!(
        "{}runtime:\n  scripts:\n    - type: before-request\n      code: \"console.log('hi')\"\n",
        request("a", &format!("{}/ok", server.uri()))
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let root = col.path().to_path_buf();
    let s = started_with(cx, &col, |p| allow_scripts(p, &root));
    std::mem::forget((col, server));
    VisualTestContext::from_window(s.win, cx).dispatch_action(ToggleConsole);
    let dock = s.shell.read_with(cx, |sh, _| sh.dock().clone());
    dock.update(cx, |d, cx| d.set_tab(tab, cx));
    let model = sender(cx, &s.ws);
    assert!(model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx)));
    finished(cx, &model);
    dock.update(cx, |d, cx| d.poll(cx));
    frame(cx, s.win);
    (s, dock)
}

#[gpui_kit::test]
fn the_console_clear_button_takes_tab_and_both_keys(cx: &mut TestAppContext) {
    for key in ["space", "enter"] {
        let (s, dock) = dock_with_rows(cx, DockTab::Console);
        assert!(!dock.read_with(cx, |d, _| d.console_shown().is_empty()));
        assert!(tab_to(cx, s.win, "dock-console-clear"), "Tab reaches Clear");
        press(cx, s.win, key);
        assert!(
            dock.read_with(cx, |d, _| d.console_shown().is_empty()),
            "{key}"
        );
    }
}

#[gpui_kit::test]
fn the_network_clear_button_takes_tab_and_both_keys(cx: &mut TestAppContext) {
    for key in ["space", "enter"] {
        let (s, dock) = dock_with_rows(cx, DockTab::Network);
        assert!(!dock.read_with(cx, |d, _| d.network_shown().is_empty()));
        assert!(tab_to(cx, s.win, "dock-network-clear"), "Tab reaches Clear");
        press(cx, s.win, key);
        assert!(
            dock.read_with(cx, |d, _| d.network_shown().is_empty()),
            "{key}"
        );
    }
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

fn kv_host(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Host>) {
    support::init(cx);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| {
                    let table = cx.new(|cx| KvTable::new(w, cx));
                    table.update(cx, |t, cx| {
                        t.load(
                            vec![KvRow {
                                name: "a".into(),
                                value: "1".into(),
                                enabled: true,
                                ..KvRow::default()
                            }],
                            w,
                            cx,
                        )
                    });
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
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(("kv-value", 0usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    (win, view)
}

fn last(cx: &mut TestAppContext, v: &Entity<Host>) -> Vec<KvRow> {
    v.read_with(cx, |h, _| h.events.last().cloned().expect("an event"))
}

#[gpui_kit::test]
fn a_table_rows_enable_toggle_takes_tab_and_both_keys(cx: &mut TestAppContext) {
    let (win, view) = kv_host(cx);
    assert!(
        tab_to(cx, win, ("kv-enabled", 0usize)),
        "Tab reaches the toggle"
    );
    press(cx, win, "space");
    assert!(!last(cx, &view)[0].enabled, "space turns the row off");
    press(cx, win, "enter");
    assert!(last(cx, &view)[0].enabled, "enter turns it back on");
}

#[gpui_kit::test]
fn a_table_rows_remove_button_takes_tab_and_both_keys(cx: &mut TestAppContext) {
    for key in ["space", "enter"] {
        let (win, view) = kv_host(cx);
        assert!(tab_to(cx, win, ("kv-delete", 0usize)), "Tab reaches Remove");
        press(cx, win, key);
        assert!(last(cx, &view).is_empty(), "{key}");
    }
}

#[gpui_kit::test]
// The branch label opens the branch dialog by Tab and either key.
fn the_title_bars_branch_entry_takes_tab_and_both_keys(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    for key in ["space", "enter"] {
        let sb = crate::git_support::Sandbox::new();
        let col = crate::h::changed(&sb);
        let s = crate::h::git_started(cx, &sb, &col);
        assert!(tab_to(cx, s.win, "git-branch"), "Tab reaches the entry");
        press(cx, s.win, key);
        assert!(
            cx.update(|cx| apinata_app::branch_dialog::current(cx))
                .is_some(),
            "{key} opens the branch dialog"
        );
    }
}

#[gpui_kit::test]
fn the_title_bars_change_count_takes_tab_and_both_keys(cx: &mut TestAppContext) {
    for key in ["space", "enter"] {
        let sb = crate::git_support::Sandbox::new();
        let col = crate::h::changed(&sb);
        let s = crate::h::git_started(cx, &sb, &col);
        assert!(tab_to(cx, s.win, "git-changes"), "Tab reaches the count");
        press(cx, s.win, key);
        assert!(
            cx.update(|cx| apinata_app::changes_dialog::current(cx))
                .is_some(),
            "{key} opens the Changes dialog"
        );
    }
}

fn json_body(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<BodyView>) {
    let body = r#"{"data":[{"id":7},{"id":8}]}"#;
    let s = send_once(
        cx,
        ResponseTemplate::new(200).set_body_raw(body, "application/json"),
        "",
    );
    support::init(cx);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| BodyView::new(w, cx))
            })
        })
        .unwrap();
    let r = s.result.clone();
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(r), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    // Tab needs a focused start; the body editor is the first stop of the pane.
    let editor = view.read_with(cx, |v, _| v.editor().clone());
    cx.update_window(win, |_, w, cx| editor.update(cx, |e, cx| e.focus(w, cx)))
        .unwrap();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    (win, view)
}

#[gpui_kit::test]
fn the_filter_button_takes_tab_and_both_keys(cx: &mut TestAppContext) {
    for key in ["space", "enter"] {
        let (win, view) = json_body(cx);
        assert!(tab_to(cx, win, "body-filter-button"), "Tab reaches Filter");
        press(cx, win, key);
        assert!(view.read_with(cx, |v, _| v.filter_open()), "{key}");
    }
}

#[gpui_kit::test]
fn the_filter_chips_clear_takes_tab_and_both_keys(cx: &mut TestAppContext) {
    for key in ["space", "enter"] {
        let (win, view) = json_body(cx);
        cx.update_window(win, |_, w, cx| {
            view.update(cx, |v, cx| v.set_filter("$.data[0].id", w, cx))
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(win, |_, w, cx| w.render_frame(cx))
            .unwrap();
        assert!(
            tab_to(cx, win, "body-filter-clear-button"),
            "Tab reaches the chip's Clear"
        );
        press(cx, win, key);
        let open = view.read_with(cx, |v, _| (v.filter_open(), v.applied_filter().is_some()));
        assert_eq!(open, (false, false), "{key} clears the filter");
    }
}

#[gpui_kit::test]
fn send_and_cancel_take_tab_and_both_keys(cx: &mut TestAppContext) {
    for key in ["space", "enter"] {
        http_ready(cx);
        let slow = ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(30));
        let server = mock_server(cx, slow);
        let col = collection_for(&server.uri(), "");
        let (_state, ws, root) = open_dir(cx, &col);
        let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
        cx.update_window(win, |_, w, cx| w.click("url", cx))
            .unwrap();
        assert!(tab_to(cx, win, "send-button"), "Tab reaches Send");
        press(cx, win, key);
        wait_real(cx, |cx| served(cx, &server).len() == 1);
        cx.update_window(win, |_, w, cx| w.render_frame(cx))
            .unwrap();
        // The sent-from Send button is gone, so focus starts over from the URL field.
        cx.update_window(win, |_, w, cx| w.click("url", cx))
            .unwrap();
        assert!(tab_to(cx, win, "cancel-button"), "Tab reaches Cancel");
        press(cx, win, key);
        let model = ed.read_with(cx, |e, _| e.send_model().clone());
        assert_eq!(
            finished(cx, &model).outcome,
            apinata_core::Outcome::Cancelled,
            "{key}"
        );
    }
}

// A keyboard Send swaps its button for Cancel, which takes the focus Send held so Enter or
// Space cancels without a hunt for the button.
#[gpui_kit::test]
fn a_keyboard_send_hands_focus_to_cancel(cx: &mut TestAppContext) {
    for key in ["space", "enter"] {
        http_ready(cx);
        let slow = ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(30));
        let server = mock_server(cx, slow);
        let col = collection_for(&server.uri(), "");
        let (_state, ws, root) = open_dir(cx, &col);
        let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
        cx.update_window(win, |_, w, cx| w.click("url", cx))
            .unwrap();
        assert!(tab_to(cx, win, "send-button"), "Tab reaches Send");
        press(cx, win, key);
        wait_real(cx, |cx| served(cx, &server).len() == 1);
        cx.update_window(win, |_, w, cx| w.render_frame(cx))
            .unwrap();
        cx.update_window(win, |_, w, cx| w.render_frame(cx))
            .unwrap();
        let on_cancel = cx
            .update_window(win, |_, w, _| w.find("cancel-button").focused())
            .unwrap();
        assert_eq!(on_cancel, Some(true), "{key}: Cancel holds the focus");
        press(cx, win, key);
        let model = ed.read_with(cx, |e, _| e.send_model().clone());
        assert_eq!(
            finished(cx, &model).outcome,
            apinata_core::Outcome::Cancelled,
            "{key}: the same key cancels"
        );
    }
}

// The focus follows the slot through Cancel to Stop as the stream opens.
#[gpui_kit::test]
fn a_keyboard_send_keeps_focus_through_to_stop(cx: &mut TestAppContext) {
    use support::sse_server::{Script, event};
    http_ready(cx);
    let server = sse(cx, Script::sse(vec![event("one")]));
    let col = sse_collection(&server);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.update_window(win, |_, w, cx| w.click("url", cx))
        .unwrap();
    assert!(tab_to(cx, win, "send-button"), "Tab reaches Send");
    press(cx, win, "enter");
    let model = ed.read_with(cx, |e, _| e.send_model().clone());
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.is_streaming()));
    for _ in 0..2 {
        cx.update_window(win, |_, w, cx| w.render_frame(cx))
            .unwrap();
    }
    let on_stop = cx
        .update_window(win, |_, w, _| w.find("stop-button").focused())
        .unwrap();
    assert_eq!(on_stop, Some(true), "Stop holds the focus");
}
