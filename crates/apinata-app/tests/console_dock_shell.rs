//! The shell's console dock: toggled by chord, status-bar button and palette command, laid
//! out under the tab area, and fed by the app-wide log.

use crate::support;

use apinata_app::commands::registry;
use apinata_app::keymap::ToggleConsole;
use apinata_app::style::{DOCK_HEIGHT, at_zoom};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, AsKeystroke as _, Focusable as _, Pixels, TestAppContext, VisualTestContext,
    px,
};
use support::*;
use wiremock::ResponseTemplate;

fn dock_height(cx: &mut TestAppContext, s: &Started) -> Pixels {
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.find("console-dock").bounds().size.height
    })
    .unwrap()
}

fn open(cx: &mut TestAppContext, s: &Started) -> bool {
    s.shell.read_with(cx, |sh, _| sh.dock_open())
}

fn polling(cx: &mut TestAppContext, s: &Started) -> bool {
    let dock = s.shell.read_with(cx, |sh, _| sh.dock().clone());
    dock.read_with(cx, |d, _| d.polling())
}

#[gpui_kit::test]
fn the_chord_toggles_the_dock_and_its_polling(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    assert!(!open(cx, &s));
    assert!(!present(cx, s.win, "console-dock"), "starts closed");
    press_in_shell(cx, &s, "ctrl-`");
    frame(cx, s.win);
    assert!(open(cx, &s));
    assert!(polling(cx, &s));
    assert!(present(cx, s.win, "console-dock"));
    press_in_shell(cx, &s, "ctrl-`");
    frame(cx, s.win);
    assert!(!open(cx, &s));
    assert!(!polling(cx, &s));
    assert!(!present(cx, s.win, "console-dock"));
}

#[gpui_kit::test]
fn the_status_bar_button_and_the_action_toggle_the_dock(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| {
        w.within("status-bar").click("toggle-console", cx)
    })
    .unwrap();
    frame(cx, s.win);
    assert!(open(cx, &s));
    VisualTestContext::from_window(s.win, cx).dispatch_action(ToggleConsole);
    frame(cx, s.win);
    assert!(!open(cx, &s));
}

#[test]
fn the_palette_lists_toggle_console() {
    let cmds = registry();
    let c = cmds
        .iter()
        .find(|c| c.id == "view.toggle_console")
        .expect("registered");
    assert_eq!(c.label, "Toggle console");
    assert_eq!(c.default_chords, ["ctrl-@", "ctrl-`"]);
    assert!(c.in_palette);
    assert!(c.targets[0].action.partial_eq(&ToggleConsole));
}

#[gpui_kit::test]
fn the_dock_sits_under_the_tab_area_and_above_the_status_bar(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    VisualTestContext::from_window(s.win, cx).dispatch_action(ToggleConsole);
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let tabs = w.find("tab-area").bounds();
        let dock = w.find("console-dock").bounds();
        let bar = w.find("status-bar").bounds();
        assert!(dock.top() >= tabs.bottom(), "{dock:?} under {tabs:?}");
        assert!(dock.bottom() <= bar.top(), "{dock:?} above {bar:?}");
        assert!(dock.size.height > gpui_kit::px(0.));
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_dock_opens_and_reopens_at_its_initial_height(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let want = at_zoom(DOCK_HEIGHT, 1.0) - px(1.);
    press_in_shell(cx, &s, "ctrl-`");
    frame(cx, s.win);
    assert!(dock_height(cx, &s) >= want, "first open");
    press_in_shell(cx, &s, "ctrl-`");
    frame(cx, s.win);
    press_in_shell(cx, &s, "ctrl-`");
    frame(cx, s.win);
    assert!(dock_height(cx, &s) >= want, "reopen after a layout");
}

#[gpui_kit::test]
fn closing_the_dock_hands_focus_back(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    press_in_shell(cx, &s, "ctrl-`");
    frame(cx, s.win);
    let dock = s.shell.read_with(cx, |sh, _| sh.dock().clone());
    cx.update_window(s.win, |_, w, cx| {
        dock.read(cx).focus_handle(cx).focus(w, cx)
    })
    .unwrap();
    frame(cx, s.win);
    VisualTestContext::from_window(s.win, cx).dispatch_action(ToggleConsole);
    frame(cx, s.win);
    assert!(!open(cx, &s));
    cx.update_window(s.win, |_, w, cx| {
        assert!(!dock.read(cx).focus_handle(cx).contains_focused(w, cx));
        let shell = s.shell.read(cx);
        let expected = shell
            .tab_area()
            .read(cx)
            .active_focus(cx)
            .unwrap_or_else(|| shell.focus_handle(cx));
        assert!(
            expected.is_focused(w),
            "focus returns to the tab or the shell"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn an_app_send_reaches_the_open_dock(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    VisualTestContext::from_window(s.win, cx).dispatch_action(ToggleConsole);
    frame(cx, s.win);
    let model = sender(cx, &s.ws);
    assert!(model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx)));
    finished(cx, &model);
    cx.executor()
        .advance_clock(apinata_app::console_dock::ConsoleDock::POLL);
    cx.run_until_parked();
    let dock = s.shell.read_with(cx, |sh, _| sh.dock().clone());
    assert_eq!(dock.read_with(cx, |d, _| d.network_shown().len()), 1);
}

// On Linux with a US layout, ctrl and the backtick key reach the app as ctrl-@.
#[gpui_kit::test]
fn the_us_layout_chord_toggles_the_dock(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    press_in_shell(cx, &s, "ctrl-@");
    frame(cx, s.win);
    assert!(open(cx, &s));
    press_in_shell(cx, &s, "ctrl-@");
    frame(cx, s.win);
    assert!(!open(cx, &s));
}

#[gpui_kit::test]
fn a_short_window_clamps_the_dock_and_keeps_the_tab_area(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.simulate_window_resize(s.win, gpui_kit::size(px(1000.), px(400.)));
    frame(cx, s.win);
    press_in_shell(cx, &s, "ctrl-`");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let tabs = w.find("tab-area").bounds();
        let dock = w.find("console-dock").bounds();
        let bar = w.find("status-bar").bounds();
        let available = bar.top() - tabs.top();
        assert!(
            dock.size.height <= available * 0.4 + px(1.),
            "{dock:?} in {available:?}"
        );
        assert!(dock.size.height > px(0.));
        assert!(dock.bottom() <= bar.top(), "{dock:?} above {bar:?}");
        assert!(
            tabs.size.height >= available * 0.6 - px(1.),
            "{tabs:?} keeps its share"
        );
    })
    .unwrap();
}

// The palette row and the button tooltip show the highest-precedence binding, which must be
// the chord printed on the key, not the Linux US-layout alias.
#[gpui_kit::test]
fn the_shown_console_chord_is_ctrl_backtick(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let shown = cx
        .update_window(s.win, |_, w, _| {
            let ctx = gpui_kit::KeyContext::parse(apinata_app::keymap::SHELL_CONTEXT).unwrap();
            w.highest_precedence_binding_for_action_in_context(&ToggleConsole, ctx)
                .and_then(|b| b.keystrokes().first().map(|k| k.as_keystroke().clone()))
        })
        .unwrap()
        .expect("bound");
    assert_eq!(shown.key, "`");
    assert!(shown.modifiers.control);
}

// Request and settings content ends at or above the open dock's top in a short window, not
// one tab strip's height into it.
#[gpui_kit::test]
fn tab_content_stays_above_the_open_dock(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    VisualTestContext::from_window(s.win, cx).dispatch_action(ToggleConsole);
    VisualTestContext::from_window(s.win, cx).simulate_resize(gpui_kit::size(px(1000.), px(360.)));
    let bottom_of = |cx: &mut TestAppContext, id: &'static str| {
        frame(cx, s.win);
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            let dock = w.find("console-dock").bounds();
            let content = w.find(id).bounds();
            assert!(
                content.bottom() <= dock.top(),
                "{id} {content:?} past the dock {dock:?}"
            );
        })
        .unwrap();
    };
    open_tab(cx, &s.ws, &s.root, "a.yml");
    bottom_of(cx, "response");
    s.ws.update(cx, |w, cx| {
        w.open_settings(&s.root, apinata_app::tabs::TabKind::Collection, cx)
    })
    .expect("a settings tab");
    bottom_of(cx, "config-editor");
}

/// The request tab after a send, on the Auth section with NTLM, at 500 px with the dock open.
fn ntlm_tab_at_500(cx: &mut TestAppContext) -> (Started, tempfile::TempDir, wiremock::MockServer) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let a = format!(
        "{}  auth:\n    type: ntlm\n    username: u\n    password: p\n    domain: CORP\n",
        request("a", &format!("{}/ok", server.uri()))
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let s = started(cx, &col);
    VisualTestContext::from_window(s.win, cx).simulate_resize(gpui_kit::size(px(1000.), px(500.)));
    frame(cx, s.win);
    VisualTestContext::from_window(s.win, cx).dispatch_action(ToggleConsole);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("send", cx))
        .unwrap();
    wait_real(cx, |cx| present(cx, s.win, "response-status"));
    cx.update_window(s.win, |_, w, cx| {
        w.click(
            (
                "section",
                apinata_app::request_editor::Section::Auth as usize,
            ),
            cx,
        )
    })
    .unwrap();
    frame(cx, s.win);
    (s, col, server)
}

// At 500 px with the dock open the NTLM Password and Domain rows are fully inside the
// request pane, above the response, with no scrolling, and the dock header is whole.
#[gpui_kit::test]
fn the_ntlm_rows_fit_the_request_pane_in_a_short_window_with_the_dock_open(
    cx: &mut TestAppContext,
) {
    let (s, _c, _srv) = ntlm_tab_at_500(cx);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let response = w.find("response").bounds();
        let pane_bottom = response.top() - px(8.);
        for id in ["auth-password", "auth-domain"] {
            let row = w.find(id).bounds();
            assert!(
                row.bottom() <= pane_bottom,
                "{id} {row:?} clipped by the request pane ending at {pane_bottom:?}"
            );
        }
        let dock = w.find("console-dock").bounds();
        let tabs = w.find("dock-tabs").bounds();
        let clear = w.find("dock-console-clear").bounds();
        assert!(
            tabs.bottom() <= dock.bottom() && clear.bottom() <= dock.bottom(),
            "dock header {tabs:?} {clear:?} clipped by {dock:?}"
        );
        assert!(response.size.height >= px(60.), "response {response:?}");
    })
    .unwrap();
}

const WITH_EXAMPLE: &str = "info:\n  name: Users\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: http://x/users\nexamples:\n  - name: ok\n    request:\n      url: http://x/users\n      method: GET\n    response:\n      status: 200\n      statusText: OK\n      body:\n        type: json\n        data: '{\"a\": 1}'\n";

// The guard above also holds for the environment editor and the example view: their content
// ends at or above the open dock's top in a short window.
#[gpui_kit::test]
fn environment_and_example_content_stay_above_the_open_dock(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("users.yml", WITH_EXAMPLE),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: host\n    value: x\n",
        ),
    ]);
    let s = started(cx, &col);
    VisualTestContext::from_window(s.win, cx).dispatch_action(ToggleConsole);
    VisualTestContext::from_window(s.win, cx).simulate_resize(gpui_kit::size(px(1000.), px(360.)));
    let bottom_of = |cx: &mut TestAppContext, id: &'static str| {
        frame(cx, s.win);
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            let dock = w.find("console-dock").bounds();
            let content = w.find(id).bounds();
            assert!(
                content.bottom() <= dock.top(),
                "{id} {content:?} past the dock {dock:?}"
            );
        })
        .unwrap();
    };
    s.ws.update(cx, |w, cx| {
        w.open_example(&s.root, ip("users.yml"), 0, false, cx)
    })
    .expect("an example tab");
    bottom_of(cx, "example-view");
    s.ws.update(cx, |w, cx| {
        w.open_settings(&s.root, apinata_app::tabs::TabKind::Environments, cx)
    })
    .expect("a settings tab");
    bottom_of(cx, "env-editor");
}

// The divider takes the tab area no lower than its minimum and the dock no lower than its own.
#[gpui_kit::test]
fn dragging_the_dock_divider_keeps_both_minimums(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    VisualTestContext::from_window(s.win, cx).simulate_resize(gpui_kit::size(px(1000.), px(700.)));
    frame(cx, s.win);
    press_in_shell(cx, &s, "ctrl-`");
    frame(cx, s.win);
    let split = s.shell.read_with(cx, |sh, _| sh.dock_split().clone());
    let heights = |cx: &mut TestAppContext, top: Pixels| {
        cx.update_window(s.win, |_, w, cx| {
            split.update(cx, |sp, cx| sp.resize_panel(0, top, w, cx))
        })
        .unwrap();
        frame(cx, s.win);
        frame(cx, s.win);
        cx.update_window(s.win, |_, w, _| {
            (
                w.find("tab-area").bounds().size.height,
                w.find("console-dock").bounds().size.height,
            )
        })
        .unwrap()
    };
    let (tabs, _) = heights(cx, px(100.));
    assert!(
        tabs >= px(379.),
        "tab area {tabs:?} dragged under its minimum"
    );
    let (_, dock) = heights(cx, px(600.));
    assert!(dock >= px(59.), "dock {dock:?} dragged under its minimum");
}

// The environment editor ends above the open dock at 500 px.
#[gpui_kit::test]
fn the_environment_tab_stays_above_the_dock_at_500_px(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    VisualTestContext::from_window(s.win, cx).simulate_resize(gpui_kit::size(px(1000.), px(500.)));
    frame(cx, s.win);
    VisualTestContext::from_window(s.win, cx).dispatch_action(ToggleConsole);
    let id = "env-editor";
    s.ws.update(cx, |w, cx| {
        w.open_settings(&s.root, apinata_app::tabs::TabKind::Environments, cx)
    })
    .expect("a settings tab");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let dock = w.find("console-dock").bounds();
        let content = w.find(id).bounds();
        assert!(
            content.bottom() <= dock.top(),
            "{id} {content:?} past the dock {dock:?}"
        );
    })
    .unwrap();
}

// With the dock open at 500 px over a runner tab, each dock tab's filter row lies fully
// inside the dock, above the status bar.
#[gpui_kit::test]
fn the_dock_filter_rows_stay_visible_at_500_px_over_a_runner_tab(cx: &mut TestAppContext) {
    use apinata_app::console_dock::DockTab;
    let col = two_requests();
    let s = started(cx, &col);
    VisualTestContext::from_window(s.win, cx).simulate_resize(gpui_kit::size(px(1000.), px(500.)));
    frame(cx, s.win);
    s.ws.update(cx, |w, cx| {
        w.open_runner(&s.root, apinata_core::ItemPath::root(), cx)
    })
    .expect("a runner tab");
    VisualTestContext::from_window(s.win, cx).dispatch_action(ToggleConsole);
    frame(cx, s.win);
    let dock = s.shell.read_with(cx, |sh, _| sh.dock().clone());
    for (tab, filters) in [
        (
            DockTab::Console,
            ["dock-console-level", "dock-console-search"],
        ),
        (
            DockTab::Network,
            ["dock-network-source", "dock-network-source"],
        ),
    ] {
        dock.update(cx, |d, cx| d.set_tab(tab, cx));
        frame(cx, s.win);
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            let bounds = w.find("console-dock").bounds();
            let bar = w.find("status-bar").bounds();
            for id in filters {
                let row = w.find(id).bounds();
                assert!(
                    row.size.height > px(0.) && row.bottom() <= bounds.bottom(),
                    "{tab:?} {id} {row:?} clipped by the dock {bounds:?}"
                );
                assert!(row.bottom() <= bar.top(), "{id} {row:?} under {bar:?}");
            }
        })
        .unwrap();
    }
}
