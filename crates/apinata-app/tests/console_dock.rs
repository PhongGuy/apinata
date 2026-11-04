//! The dock's model: polling only while open, separate cursors, clear and eviction.

use crate::support;

use std::time::Duration;

use apinata_app::console_dock::ConsoleDock;
use apinata_core::{
    ExecOptions, Executor, GlobalProxy, LogLimits, NetworkSettings, PacSource, SendSource,
    SessionLog,
};
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use support::*;
use wiremock::ResponseTemplate;

fn host(cx: &mut TestAppContext, log: SessionLog) -> (AnyWindowHandle, Entity<ConsoleDock>) {
    support::init(cx);
    open_dock(cx, log)
}

// A dock window in an app that is already initialised.
fn open_dock(cx: &mut TestAppContext, log: SessionLog) -> (AnyWindowHandle, Entity<ConsoleDock>) {
    let (win, dock) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ConsoleDock::new(log, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    (win, dock)
}

fn logged(log: &SessionLog) -> Executor {
    Executor::new(ExecOptions::new().session_log(log.clone())).unwrap()
}

fn shown(cx: &mut TestAppContext, dock: &Entity<ConsoleDock>) -> Vec<u64> {
    dock.read_with(cx, |d, _| d.network_shown().iter().map(|e| e.seq).collect())
}

#[gpui_kit::test]
fn polling_runs_only_while_open(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let log = SessionLog::new();
    let exec = logged(&log);
    let (_win, dock) = host(cx, log.clone());
    assert!(!dock.read_with(cx, |d, _| d.polling()));
    send_direct(cx, &exec, &server.uri());
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert!(shown(cx, &dock).is_empty(), "closed: no poll");
    dock.update(cx, |d, cx| d.set_polling(true, cx));
    cx.run_until_parked();
    assert_eq!(shown(cx, &dock).len(), 1, "opening polls at once");
    send_direct(cx, &exec, &server.uri());
    cx.executor().advance_clock(ConsoleDock::POLL);
    cx.run_until_parked();
    assert_eq!(shown(cx, &dock).len(), 2, "the timer polls");
    dock.update(cx, |d, cx| d.set_polling(false, cx));
    send_direct(cx, &exec, &server.uri());
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(shown(cx, &dock).len(), 2, "closed again");
    assert!(dock.update(cx, |d, cx| d.poll(cx)));
    assert_eq!(shown(cx, &dock).len(), 3);
}

#[gpui_kit::test]
fn clear_drops_the_rows_and_the_selection_but_keeps_reading(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let log = SessionLog::new();
    let exec = logged(&log);
    let (_win, dock) = host(cx, log.clone());
    send_direct(cx, &exec, &server.uri());
    send_direct(cx, &exec, &server.uri());
    dock.update(cx, |d, cx| d.poll(cx));
    let first = shown(cx, &dock)[0];
    dock.update(cx, |d, cx| d.select(Some(first), cx));
    assert_eq!(
        dock.read_with(cx, |d, _| d.selected().map(|e| e.seq)),
        Some(first)
    );
    dock.update(cx, |d, cx| d.clear_network(cx));
    assert!(shown(cx, &dock).is_empty());
    assert!(dock.read_with(cx, |d, _| d.selected().is_none()));
    assert!(
        log.network_since(0).entries.is_empty(),
        "the log itself is cleared"
    );
    send_direct(cx, &exec, &server.uri());
    dock.update(cx, |d, cx| d.poll(cx));
    assert_eq!(shown(cx, &dock).len(), 1);
}

#[gpui_kit::test]
fn an_evicted_entry_leaves_the_dock_and_the_selection(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let mut limits = LogLimits::default();
    limits.network = 2;
    let log = SessionLog::with_limits(limits);
    let exec = logged(&log);
    let (_win, dock) = host(cx, log.clone());
    send_direct(cx, &exec, &server.uri());
    dock.update(cx, |d, cx| d.poll(cx));
    let oldest = shown(cx, &dock)[0];
    dock.update(cx, |d, cx| d.select(Some(oldest), cx));
    send_direct(cx, &exec, &server.uri());
    send_direct(cx, &exec, &server.uri());
    dock.update(cx, |d, cx| d.poll(cx));
    let now = shown(cx, &dock);
    assert_eq!(now.len(), 2);
    assert!(!now.contains(&oldest));
    assert!(dock.read_with(cx, |d, _| d.selected().is_none()));
}

#[gpui_kit::test]
fn the_source_filter_narrows_the_network_rows(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let log = SessionLog::new();
    let exec = logged(&log);
    let (_win, dock) = host(cx, log.clone());
    send_direct(cx, &exec, &server.uri());
    dock.update(cx, |d, cx| d.poll(cx));
    dock.update(cx, |d, cx| {
        d.set_source_filter(Some(SendSource::OAuth2), cx)
    });
    assert!(shown(cx, &dock).is_empty());
    dock.update(cx, |d, cx| {
        d.set_source_filter(Some(SendSource::Request), cx)
    });
    assert_eq!(shown(cx, &dock).len(), 1);
    dock.update(cx, |d, cx| d.set_source_filter(None, cx));
    assert_eq!(shown(cx, &dock).len(), 1);
}

#[gpui_kit::test]
fn clearing_one_kind_leaves_the_other_cursor_alone(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let a = format!(
        "{}runtime:\n  scripts:\n    - type: before-request\n      code: \"console.log('line')\"\n",
        request("a", &format!("{}/ok", server.uri()))
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let root = col.path().to_path_buf();
    let s = started_with(cx, &col, |p| allow_scripts(p, &root));
    let log = cx.update(|cx| apinata_app::log_feed::shared(cx)).unwrap();
    let (_win, dock) = open_dock(cx, log.clone());
    let send = |cx: &mut TestAppContext| {
        let model = sender(cx, &s.ws);
        assert!(model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx)));
        finished(cx, &model);
    };
    send(cx);
    dock.update(cx, |d, cx| d.poll(cx));
    log.clear_console();
    send(cx);
    dock.update(cx, |d, cx| d.poll(cx));
    assert_eq!(shown(cx, &dock).len(), 2, "network kept both sends");
    assert_eq!(
        dock.read_with(cx, |d, _| d.console_shown().len()),
        1,
        "console only the new line"
    );
}

#[gpui_kit::test]
fn the_dock_renders_its_tabs_and_clear(cx: &mut TestAppContext) {
    http_ready(cx);
    let (win, dock) = host(cx, SessionLog::new());
    assert!(present(cx, win, "console-dock"));
    assert!(present(cx, win, "dock-tabs"));
    assert!(present(cx, win, "dock-console-clear"));
    dock.update(cx, |d, cx| {
        d.set_tab(apinata_app::console_dock::DockTab::Network, cx)
    });
    assert!(present(cx, win, "dock-network-clear"));
}

#[gpui_kit::test]
fn the_network_list_draws_only_the_newest_rows(cx: &mut TestAppContext) {
    use apinata_app::console_dock::{DOCK_NETWORK_ROWS, DockTab};
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let log = SessionLog::new();
    let exec = logged(&log);
    let (win, dock) = host(cx, log.clone());
    for _ in 0..=DOCK_NETWORK_ROWS {
        send_direct(cx, &exec, &server.uri());
    }
    dock.update(cx, |d, cx| {
        d.poll(cx);
        d.set_tab(DockTab::Network, cx);
    });
    assert_eq!(
        shown(cx, &dock).len(),
        DOCK_NETWORK_ROWS + 1,
        "the model keeps all"
    );
    assert!(present(cx, win, "dock-network-omitted"));
    assert!(present(cx, win, ("dock-network", DOCK_NETWORK_ROWS - 1)));
    assert!(!present(cx, win, ("dock-network", DOCK_NETWORK_ROWS)));
}

#[gpui_kit::test]
fn a_selection_the_source_filter_hides_is_dropped(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let log = SessionLog::new();
    let exec = logged(&log);
    let (_win, dock) = host(cx, log.clone());
    send_direct(cx, &exec, &server.uri());
    dock.update(cx, |d, cx| d.poll(cx));
    let seq = shown(cx, &dock)[0];
    dock.update(cx, |d, cx| d.select(Some(seq), cx));
    dock.update(cx, |d, cx| {
        d.set_source_filter(Some(SendSource::Script), cx)
    });
    assert!(
        dock.read_with(cx, |d, _| d.selected().is_none()),
        "hidden: no detail"
    );
    dock.update(cx, |d, cx| d.set_source_filter(None, cx));
    assert!(
        dock.read_with(cx, |d, _| d.selected().is_none()),
        "the selection stays dropped"
    );
}

#[gpui_kit::test]
fn a_selection_past_the_drawn_rows_is_dropped(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let log = SessionLog::new();
    let exec = logged(&log);
    let (_win, dock) = host(cx, log.clone());
    send_direct(cx, &exec, &server.uri());
    dock.update(cx, |d, cx| d.poll(cx));
    let oldest = shown(cx, &dock)[0];
    dock.update(cx, |d, cx| d.select(Some(oldest), cx));
    for _ in 0..apinata_app::console_dock::DOCK_NETWORK_ROWS {
        send_direct(cx, &exec, &server.uri());
    }
    dock.update(cx, |d, cx| d.poll(cx));
    assert!(shown(cx, &dock).contains(&oldest), "still in the ring");
    assert!(dock.read_with(cx, |d, _| d.selected().is_none()));
}

#[gpui_kit::test]
fn a_pac_routed_hop_shows_its_route_in_the_detail(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("direct.pac");
    std::fs::write(
        &script,
        r#"function FindProxyForURL(url, host) { return "DIRECT"; }"#,
    )
    .unwrap();
    let source = PacSource::parse(script.to_str().unwrap(), None).unwrap();
    let log = SessionLog::new();
    let exec = Executor::new(
        ExecOptions::new()
            .network(NetworkSettings::new().proxy(GlobalProxy::Pac { source }))
            .session_log(log.clone()),
    )
    .unwrap();
    send_direct(cx, &exec, &server.uri());
    let e = log.network_since(0).entries.remove(0);
    let detail = apinata_app::console_dock::detail_text(&e);
    let lines: Vec<&str> = detail.lines().collect();
    let hop = lines
        .iter()
        .position(|l| l.starts_with("GET ") && l.contains(" 200 · "));
    let hop = hop.unwrap_or_else(|| panic!("no hop line: {detail}"));
    assert_eq!(lines.get(hop + 1), Some(&"PAC: DIRECT"), "{detail}");
}

#[gpui_kit::test]
fn a_retried_send_names_its_attempt_in_the_detail(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let log = SessionLog::new();
    let exec = Executor::new(ExecOptions::new().session_log(log.clone())).unwrap();
    send_direct(cx, &exec, &server.uri());
    let e = log.network_since(0).entries.remove(0);
    assert!(
        !apinata_app::console_dock::detail_text(&e).contains("Attempt"),
        "a first send has no attempt line"
    );
    let mut e = (*e).clone();
    e.attempt = Some((2, 3));
    let detail = apinata_app::console_dock::detail_text(&e);
    let lines: Vec<&str> = detail.lines().collect();
    assert_eq!(lines.get(1), Some(&"Attempt: 2 of 3"), "{detail}");
}

#[gpui_kit::test]
fn the_dock_tabs_switch_the_panel(cx: &mut TestAppContext) {
    use apinata_app::console_dock::DockTab;
    use gpui_kit::test::TestWindowExt as _;
    http_ready(cx);
    let (win, dock) = host(cx, SessionLog::new());
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click(("dock-tab", 1usize), cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(dock.read_with(cx, |d, _| d.tab()), DockTab::Network);
    assert!(present(cx, win, "dock-network-clear"));
    cx.update_window(win, |_, w, cx| w.click(("dock-tab", 0usize), cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(dock.read_with(cx, |d, _| d.tab()), DockTab::Console);
}

// The dock tab holding focus, if any, after a fresh frame.
fn focused_tab(cx: &mut TestAppContext, win: AnyWindowHandle) -> Option<usize> {
    use gpui_kit::test::TestWindowExt as _;
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        (0..2usize).find(|&ix| w.find(("dock-tab", ix)).focused() == Some(true))
    })
    .unwrap()
}

// A dock with the app's keymap whose first tab holds keyboard focus.
fn dock_on_first_tab(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<ConsoleDock>) {
    let (win, dock) = host(cx, SessionLog::new());
    cx.update(apinata_app::keymap::init);
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.focus_next(cx)).unwrap();
    assert_eq!(
        focused_tab(cx, win),
        Some(0),
        "Tab reaches the first dock tab"
    );
    (win, dock)
}

#[gpui_kit::test]
fn enter_and_space_choose_the_focused_dock_tab(cx: &mut TestAppContext) {
    use apinata_app::console_dock::DockTab;
    let (win, dock) = dock_on_first_tab(cx);
    cx.update_window(win, |_, w, cx| w.focus_next(cx)).unwrap();
    assert_eq!(focused_tab(cx, win), Some(1));
    assert_eq!(
        dock.read_with(cx, |d, _| d.tab()),
        DockTab::Console,
        "focus alone chooses nothing"
    );
    cx.simulate_keystrokes(win, "enter");
    frame(cx, win);
    assert_eq!(dock.read_with(cx, |d, _| d.tab()), DockTab::Network);
    cx.update_window(win, |_, w, cx| w.focus_prev(cx)).unwrap();
    assert_eq!(focused_tab(cx, win), Some(0));
    cx.simulate_keystrokes(win, "space");
    frame(cx, win);
    assert_eq!(dock.read_with(cx, |d, _| d.tab()), DockTab::Console);
}

#[gpui_kit::test]
fn arrows_move_the_dock_tab_and_the_focus(cx: &mut TestAppContext) {
    use apinata_app::console_dock::DockTab;
    let (win, dock) = dock_on_first_tab(cx);
    let at = |cx: &mut TestAppContext| (dock.read_with(cx, |d, _| d.tab()), focused_tab(cx, win));
    cx.simulate_keystrokes(win, "right");
    assert_eq!(at(cx), (DockTab::Network, Some(1)));
    cx.simulate_keystrokes(win, "right");
    assert_eq!(at(cx), (DockTab::Console, Some(0)), "wraps");
    cx.simulate_keystrokes(win, "left");
    assert_eq!(at(cx), (DockTab::Network, Some(1)), "wraps back");
    cx.simulate_keystrokes(win, "up");
    assert_eq!(at(cx), (DockTab::Console, Some(0)));
    cx.simulate_keystrokes(win, "down");
    assert_eq!(at(cx), (DockTab::Network, Some(1)));
}

// The dock's keyboard ring leaves the room around its label that a request section tab's
// ring does, for a section tab that carries a count.
#[gpui_kit::test]
fn the_dock_tab_ring_pads_its_label_like_the_section_strip(cx: &mut TestAppContext) {
    use apinata_app::focus_mode::{self, Modality};
    use apinata_app::request_editor::Section;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{Bounds, Pixels};
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0?q=1")),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (ed_win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let (dock_win, _dock) = open_dock(cx, SessionLog::new());
    let params = Section::Params as usize;
    let handle = ed.read_with(cx, |e, _| e.section_focus(params));
    cx.update_window(ed_win, |_, w, cx| handle.focus(w, cx))
        .unwrap();
    cx.update_window(dock_win, |_, w, cx| w.focus_next(cx))
        .unwrap();
    cx.update(|cx| focus_mode::set(Modality::Keyboard, cx));
    // (left, top, bottom) room between a ring and the label it marks.
    let room = |ring: Bounds<Pixels>, label: Bounds<Pixels>| {
        (
            label.left() - ring.left(),
            label.top() - ring.top(),
            ring.bottom() - label.bottom(),
        )
    };
    let section = cx
        .update_window(ed_win, |_, w, cx| {
            w.render_frame(cx);
            room(
                w.find(("section-focus", params)).bounds(),
                w.find(("section", params)).bounds(),
            )
        })
        .unwrap();
    let (dock, right) = cx
        .update_window(dock_win, |_, w, cx| {
            w.render_frame(cx);
            let ring = w.find(("dock-tab-ring", 0usize)).bounds();
            let label = w.find(("dock-tab-label", 0usize)).bounds();
            (room(ring, label), ring.right() - label.right())
        })
        .unwrap();
    let near = |a: Pixels, b: Pixels| (f32::from(a) - f32::from(b)).abs() < 0.5;
    assert!(
        near(dock.0, section.0) && near(dock.1, section.1) && near(dock.2, section.2),
        "dock room {dock:?}, section room {section:?}"
    );
    assert!(
        near(right, dock.0),
        "even sides: {right:?} and {:?}",
        dock.0
    );
}

#[gpui_kit::test]
fn network_rows_leave_the_scrollbar_its_gutter(cx: &mut TestAppContext) {
    use apinata_app::console_dock::DockTab;
    use apinata_app::style::SCROLLBAR_GUTTER;
    use gpui_kit::px;
    use gpui_kit::test::TestWindowExt as _;
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let log = SessionLog::new();
    let exec = logged(&log);
    let (win, dock) = host(cx, log.clone());
    send_direct(cx, &exec, &server.uri());
    dock.update(cx, |d, cx| {
        d.poll(cx);
        d.set_tab(DockTab::Network, cx);
    });
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let column = w.find("dock-network-column").bounds();
        let row = w.find(("dock-network", 0usize)).bounds();
        assert!(
            row.right() <= column.right() - SCROLLBAR_GUTTER + px(0.5),
            "row {row:?} runs under the scrollbar of {column:?}"
        );
    })
    .unwrap();
}
