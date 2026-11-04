use crate::support;

use apinata_app::command::Command;
use apinata_app::shell::Shell;
use apinata_app::tab_area::{
    ActivateLastTab, ActivateNextTab, ActivateTab, CloseAllTabs, CloseTab, ReopenClosedTab, SaveAll,
};
use gpui_kit::component::WindowExt;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Focusable, TestAppContext, VisualTestContext, WindowOptions};
use std::time::Duration;
use support::*;

fn active(cx: &mut TestAppContext, a: &Area) -> String {
    a.ws.read_with(cx, |w, _| {
        w.tabs()
            .active()
            .and_then(|e| e.item())
            .map(|p| p.as_str().to_owned())
    })
    .unwrap()
}

// The tab keys bind in the shell's context, which a bare tab area lacks, so the test
// dispatches the action each key maps to with the tab area focused.
fn keys(cx: &mut TestAppContext, a: &Area, action: impl gpui_kit::Action) {
    let view = a.view.clone();
    let mut vcx = VisualTestContext::from_window(a.win, cx);
    vcx.update(|w, cx| view.read(cx).focus_handle(cx).focus(w, cx));
    vcx.dispatch_action(action);
    vcx.run_until_parked();
}

#[gpui_kit::test]
fn tab_keys_navigate_close_reopen_and_save_all(cx: &mut TestAppContext) {
    let a = open_area(cx);
    cx.update(apinata_app::keymap::init);
    open_tab(cx, &a.ws, &a.root, "a.yml");
    open_tab(cx, &a.ws, &a.root, "b.yml");
    cx.run_until_parked();
    keys(cx, &a, ActivateTab(0));
    assert_eq!(active(cx, &a), "a.yml");
    keys(cx, &a, ActivateLastTab);
    assert_eq!(active(cx, &a), "b.yml");
    keys(cx, &a, ActivateNextTab);
    assert_eq!(active(cx, &a), "a.yml");
    keys(cx, &a, CloseTab);
    assert_eq!(tab_names(cx, &a.ws), ["b.yml"]);
    keys(cx, &a, ReopenClosedTab);
    assert_eq!(tab_names(cx, &a.ws), ["b.yml", "a.yml"]);
    a.ws.update(cx, |w, cx| {
        w.send(&a.root, edit_url("b.yml", "http://x/2"), cx)
    });
    cx.run_until_parked();
    keys(cx, &a, SaveAll);
    keys(cx, &a, CloseAllTabs);
    assert!(!confirm_open(cx));
    assert!(tab_names(cx, &a.ws).is_empty());
    assert!(!view_of(cx, &a.ws, &a.root, "b.yml").dirty);
}

#[gpui_kit::test]
fn the_shell_opens_sidebar_clicks_as_tabs_and_reports_closed_tabs(cx: &mut TestAppContext) {
    support::init(cx);
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let shared = ws.clone();
    let (win, _shell) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| Shell::new(shared, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(row("a.yml"), cx)
    })
    .unwrap();
    cx.run_until_parked();
    let first = ws
        .read_with(cx, |w, _| w.tabs().active().cloned())
        .expect("a tab");
    assert!(first.preview);
    assert!(present(cx, win, "url"));
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.update_window(win, |_, w, cx| w.double_click(row("b.yml"), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(tab_names(cx, &ws), ["b.yml"]);
    assert!(!ws.read_with(cx, |w, _| w.tabs().active().unwrap().preview));
    std::fs::remove_file(col.path().join("b.yml")).unwrap();
    ws.update(cx, |w, cx| {
        w.send(&root, Command::Reload(vec![col.path().join("b.yml")]), cx)
    });
    cx.run_until_parked();
    assert_eq!(
        cx.update_window(win, |_, w, cx| w.notifications(cx).len())
            .unwrap(),
        1
    );
}
