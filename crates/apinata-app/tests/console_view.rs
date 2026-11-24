//! One console view in two places: the response pane's Console tab and the dock draw the same
//! rows and copy the same lines, while each keeps its own filters.

use crate::console_dock_tabs::{SECRET, scripted_app, secret_app};
use crate::support;

use apinata_app::console_dock::{ConsoleDock, DockTab};
use apinata_app::keymap::ToggleConsole;
use apinata_app::response_pane::{ResponsePane, ResponseTab};
use apinata_core::{ConsoleEntry, ConsoleLevel, ScriptPhase};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, ElementId, Entity, TestAppContext, VisualTestContext,
    WindowOptions,
};
use support::*;
use wiremock::ResponseTemplate;

// Opens the dock on Console, sends `a.yml`, polls, and opens a response pane on that send.
fn both(
    cx: &mut TestAppContext,
    s: &Started,
) -> (Entity<ConsoleDock>, AnyWindowHandle, Entity<ResponsePane>) {
    VisualTestContext::from_window(s.win, cx).dispatch_action(ToggleConsole);
    let dock = s.shell.read_with(cx, |sh, _| sh.dock().clone());
    dock.update(cx, |d, cx| d.set_tab(DockTab::Console, cx));
    let model = sender(cx, &s.ws);
    assert!(model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx)));
    finished(cx, &model);
    dock.update(cx, |d, cx| d.poll(cx));
    frame(cx, s.win);
    let (win, pane) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    pane.update(cx, |p, cx| p.set_tab(ResponseTab::Console, cx));
    frame(cx, win);
    (dock, win, pane)
}

// Labels of `(prefix, 0..)` in `win` for as long as such an element exists.
fn labels(cx: &mut TestAppContext, win: AnyWindowHandle, prefix: &'static str) -> Vec<String> {
    let mut out = Vec::new();
    loop {
        let id = ElementId::from((prefix, out.len()));
        let label = cx
            .update_window(win, |_, w, cx| {
                w.render_frame(cx);
                w.try_find(id)
                    .map(|e| e.label().unwrap_or_default().to_owned())
            })
            .unwrap();
        match label {
            Some(label) => out.push(label),
            None => return out,
        }
    }
}

// Clicks `id` in `win` and returns what landed on the clipboard.
fn copy(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> String {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(id, cx)
    })
    .unwrap();
    cx.run_until_parked();
    cx.read_from_clipboard()
        .and_then(|c| c.text())
        .unwrap_or_default()
}

#[gpui_kit::test]
fn the_tab_and_the_dock_draw_and_copy_the_same_lines(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let code = "console.log('hello from a'); console.warn('careful')";
    let (_col, s) = scripted_app(cx, &server.uri(), code);
    let (_dock, win, _pane) = both(cx, &s);
    let tab = labels(cx, win, "console");
    let dock = labels(cx, s.win, "dock-console");
    assert_eq!(tab.len(), 2, "{tab:?}");
    assert_eq!(dock.len(), 2, "{dock:?}");
    assert_eq!(tab[0], "log [pre-request] hello from a");
    // The dock's rows add when and which request; the rest of each row is the tab's.
    for (t, d) in tab.iter().zip(&dock) {
        assert!(d.ends_with(&format!(" a {t}")), "{d:?} vs {t:?}");
    }
    let tab_copy = copy(cx, win, "console-copy");
    let dock_copy = copy(cx, s.win, "dock-console-copy");
    assert_eq!(tab_copy.lines().collect::<Vec<_>>(), tab);
    assert_eq!(dock_copy.lines().collect::<Vec<_>>(), dock);
}

#[gpui_kit::test]
fn a_filter_set_in_one_place_stays_there(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let code = "console.log('hello from a'); console.warn('careful')";
    let (_col, s) = scripted_app(cx, &server.uri(), code);
    let (dock, win, pane) = both(cx, &s);
    dock.update(cx, |d, cx| d.set_level_filter(Some(ConsoleLevel::Warn), cx));
    assert_eq!(labels(cx, s.win, "dock-console").len(), 1);
    assert_eq!(
        labels(cx, win, "console").len(),
        2,
        "the dock's level stays in the dock"
    );
    assert_eq!(pane.read_with(cx, |p, _| p.console_filter()), None);
    // Typing into the tab's own search box narrows the tab alone.
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click("console-search", cx);
        w.input("HELLO", cx);
    })
    .unwrap();
    frame(cx, win);
    let tab = labels(cx, win, "console");
    assert_eq!(tab.len(), 1, "{tab:?}");
    assert!(tab[0].contains("hello from a"));
    dock.update(cx, |d, cx| d.set_level_filter(None, cx));
    assert_eq!(
        labels(cx, s.win, "dock-console").len(),
        2,
        "the tab's search stays in the tab"
    );
    // Copy takes what the filters keep, in each place by its own filters.
    assert_eq!(
        copy(cx, win, "console-copy"),
        "log [pre-request] hello from a"
    );
    pane.update(cx, |p, cx| p.set_console_search("", cx));
    assert_eq!(labels(cx, win, "console").len(), 2);
}

#[gpui_kit::test]
fn the_dock_copies_a_secret_redacted(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let (_col, s) = secret_app(cx, &server.uri());
    let (_dock, win, _pane) = both(cx, &s);
    let dock = copy(cx, s.win, "dock-console-copy");
    assert!(
        dock.contains("[redacted]") && !dock.contains(SECRET),
        "{dock}"
    );
    let tab = copy(cx, win, "console-copy");
    assert!(tab.contains("[redacted]") && !tab.contains(SECRET), "{tab}");
}

#[gpui_kit::test]
fn the_tab_keeps_the_newest_lines_as_the_dock_does(cx: &mut TestAppContext) {
    // 30 lines of about 4000 characters overrun the 50 000 character pane budget.
    let console: Vec<ConsoleEntry> = (0..30)
        .map(|i| {
            let text = format!("L{i}:{}", "x".repeat(4000));
            ConsoleEntry::new(ScriptPhase::PreRequest, ConsoleLevel::Log, text)
        })
        .collect();
    let s = send_once(cx, ResponseTemplate::new(200), "");
    support::init(cx);
    let mut r = (*s.result).clone();
    r.console = console;
    s.model
        .update(cx, |m, cx| m.show(std::sync::Arc::new(r), cx));
    let model = s.model.clone();
    let (win, pane) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    pane.update(cx, |p, cx| p.set_tab(ResponseTab::Console, cx));
    let rows = labels(cx, win, "console");
    assert!(rows.len() < 30, "the budget cut some lines");
    assert!(
        rows.last().unwrap().contains("L29:"),
        "the newest line is drawn"
    );
    assert!(
        rows.iter().all(|r| !r.contains("L0:")),
        "the oldest line is left out"
    );
    assert!(present(cx, win, "console-more-entries"));
}
