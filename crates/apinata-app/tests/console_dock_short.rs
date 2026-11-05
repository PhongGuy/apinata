//! The console dock in a very short window: it shows only its header until the person opens
//! it, and the open state it was given is kept for taller windows.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::palette::OpenCommandPalette;
use apinata_app::settings::Settings;
use apinata_app::style::{DOCK_HEIGHT, DOCK_MIN, at_zoom, dock_header};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, Focusable as _, Pixels, TestAppContext, VisualTestContext, px, size,
};
use support::*;

fn bounds(cx: &mut TestAppContext, s: &Started, id: &'static str) -> Bounds<Pixels> {
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).bounds()
    })
    .unwrap()
}

fn resize(cx: &mut TestAppContext, s: &Started, height: f32) {
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1000.), px(height)));
    frame(cx, s.win);
    frame(cx, s.win);
}

fn open(cx: &mut TestAppContext, s: &Started) -> bool {
    s.shell.read_with(cx, |sh, _| sh.dock_open())
}

fn click_header(cx: &mut TestAppContext, s: &Started) {
    cx.update_window(s.win, |_, w, cx| w.click("dock-header", cx))
        .unwrap();
    frame(cx, s.win);
    frame(cx, s.win);
}

fn near(a: Pixels, b: Pixels) -> bool {
    (a - b).abs() <= px(1.)
}

// A request tab in a 1000x300 window with the dock toggled on.
fn short_with_dock(cx: &mut TestAppContext, col: &tempfile::TempDir) -> Started {
    let s = started(cx, col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    resize(cx, &s, 300.);
    press_in_shell(cx, &s, "ctrl-`");
    frame(cx, s.win);
    frame(cx, s.win);
    s
}

#[gpui_kit::test]
fn a_short_window_shows_the_dock_collapsed_to_its_header(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = short_with_dock(cx, &col);
    assert!(open(cx, &s), "the toggle still counts as open");
    let dock = bounds(cx, &s, "console-dock");
    assert!(
        near(dock.size.height, dock_header(1.)),
        "collapsed dock {dock:?}"
    );
    let header = bounds(cx, &s, "dock-header");
    assert!(near(header.size.height, dock.size.height), "{header:?}");
    assert!(
        !present(cx, s.win, "dock-console-search"),
        "the body is hidden"
    );
}

#[gpui_kit::test]
fn the_collapsed_dock_gives_the_tab_area_the_height_back(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = short_with_dock(cx, &col);
    let area = bounds(cx, &s, "tab-area");
    let dock = bounds(cx, &s, "console-dock");
    let bar = bounds(cx, &s, "status-bar");
    assert!(near(area.bottom(), dock.top()), "{area:?} over {dock:?}");
    assert!(near(dock.bottom(), bar.top()), "{dock:?} over {bar:?}");
    // 240 px between the bars, less the header alone rather than the open dock.
    assert!(
        near(area.size.height, px(240.) - dock_header(1.)),
        "{area:?}"
    );
    click_header(cx, &s);
    let opened = bounds(cx, &s, "tab-area");
    assert!(opened.size.height < area.size.height, "{opened:?}");
}

#[gpui_kit::test]
fn a_click_on_the_header_opens_the_dock_and_it_stays_open(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = short_with_dock(cx, &col);
    click_header(cx, &s);
    let dock = bounds(cx, &s, "console-dock");
    assert!(
        near(dock.size.height, at_zoom(DOCK_MIN, 1.)),
        "opened dock {dock:?}"
    );
    assert!(present(cx, s.win, "dock-console-search"));
    // Still short, so the person's choice holds through a resize and later frames.
    resize(cx, &s, 320.);
    let again = bounds(cx, &s, "console-dock");
    assert!(
        near(again.size.height, at_zoom(DOCK_MIN, 1.)),
        "after a resize {again:?}"
    );
    assert!(present(cx, s.win, "dock-console-search"));
}

#[gpui_kit::test]
fn a_tall_window_opens_the_dock_in_full(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    resize(cx, &s, 800.);
    press_in_shell(cx, &s, "ctrl-`");
    frame(cx, s.win);
    let dock = bounds(cx, &s, "console-dock");
    assert!(
        dock.size.height >= at_zoom(DOCK_HEIGHT, 1.) - px(1.),
        "{dock:?}"
    );
    assert!(present(cx, s.win, "dock-console-search"));
}

// Collapsing is a view rule: the open toggle survives a short spell, a taller window shows
// the dock at its usual height, and a later short spell collapses it afresh.
#[gpui_kit::test]
fn a_short_spell_keeps_the_dock_open_state(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = short_with_dock(cx, &col);
    click_header(cx, &s);
    resize(cx, &s, 800.);
    assert!(open(cx, &s));
    let dock = bounds(cx, &s, "console-dock");
    assert!(
        dock.size.height >= at_zoom(DOCK_HEIGHT, 1.) - px(1.),
        "tall again {dock:?}"
    );
    resize(cx, &s, 300.);
    assert!(open(cx, &s));
    let dock = bounds(cx, &s, "console-dock");
    assert!(
        near(dock.size.height, dock_header(1.)),
        "short again {dock:?}"
    );
}

// The window height at 100% below which the open dock collapses.
#[gpui_kit::test]
fn the_collapse_threshold_is_pinned(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = short_with_dock(cx, &col);
    resize(cx, &s, 345.);
    let dock = bounds(cx, &s, "console-dock");
    assert!(near(dock.size.height, dock_header(1.)), "345 {dock:?}");
    resize(cx, &s, 346.);
    let dock = bounds(cx, &s, "console-dock");
    assert!(
        near(dock.size.height, at_zoom(DOCK_MIN, 1.)),
        "346 {dock:?}"
    );
}

fn collapsed(cx: &mut TestAppContext, s: &Started) -> bool {
    let dock = s.shell.read_with(cx, |sh, _| sh.dock().clone());
    dock.read_with(cx, |d, _| d.collapsed())
}

// Keys typed where focus already is, without moving it to the shell first.
fn keys(cx: &mut TestAppContext, s: &Started, ks: &str) {
    let mut v = VisualTestContext::from_window(s.win, cx);
    v.simulate_keystrokes(ks);
    v.run_until_parked();
    frame(cx, s.win);
    frame(cx, s.win);
}

#[gpui_kit::test]
fn the_chord_expands_a_collapsed_dock_then_closes_it(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = short_with_dock(cx, &col);
    assert!(collapsed(cx, &s));
    press_in_shell(cx, &s, "ctrl-`");
    frame(cx, s.win);
    assert!(open(cx, &s), "the first press expands");
    assert!(!collapsed(cx, &s));
    assert!(present(cx, s.win, "dock-console-search"));
    press_in_shell(cx, &s, "ctrl-`");
    frame(cx, s.win);
    assert!(!open(cx, &s), "the second press closes");
}

#[gpui_kit::test]
fn enter_on_a_collapsed_dock_tab_expands_it(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = short_with_dock(cx, &col);
    let dock = s.shell.read_with(cx, |sh, _| sh.dock().clone());
    cx.update_window(s.win, |_, w, cx| {
        dock.read(cx).focus_handle(cx).focus(w, cx);
        w.focus_next(cx);
    })
    .unwrap();
    frame(cx, s.win);
    let on_tab = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.find(("dock-tab", 0usize)).focused()
        })
        .unwrap();
    assert_eq!(on_tab, Some(true), "Tab reaches the first dock tab");
    keys(cx, &s, "enter");
    assert!(!collapsed(cx, &s));
    assert!(present(cx, s.win, "dock-console-search"));
}

// Focus in the body moves out as the body stops drawing, so the shell's keys still work.
#[gpui_kit::test]
fn collapsing_moves_focus_out_of_the_dock_body(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    resize(cx, &s, 800.);
    press_in_shell(cx, &s, "ctrl-`");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("dock-console-search", cx))
        .unwrap();
    frame(cx, s.win);
    let dock = s.shell.read_with(cx, |sh, _| sh.dock().clone());
    let in_dock = |cx: &mut TestAppContext| {
        cx.update_window(s.win, |_, w, cx| {
            dock.read(cx).focus_handle(cx).contains_focused(w, cx)
        })
        .unwrap()
    };
    assert!(in_dock(cx), "the filter holds focus");
    resize(cx, &s, 300.);
    assert!(collapsed(cx, &s));
    assert!(!in_dock(cx), "focus left the hidden body");
    let handed_back = cx
        .update_window(s.win, |_, w, cx| {
            let shell = s.shell.read(cx);
            let expected = (shell.tab_area().read(cx).active_focus(cx))
                .unwrap_or_else(|| shell.focus_handle(cx));
            expected.is_focused(w)
        })
        .unwrap();
    assert!(handed_back, "focus moves to the active tab or the shell");
    let reachable = cx
        .update_window(s.win, |_, w, cx| {
            w.is_action_available(&OpenCommandPalette, cx)
        })
        .unwrap();
    assert!(reachable, "the shell's keys reach the app");
    keys(cx, &s, "ctrl-`");
    assert!(!collapsed(cx, &s), "the chord reaches the shell");
}

#[gpui_kit::test]
fn a_collapsed_header_has_no_clear_button(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = short_with_dock(cx, &col);
    assert!(!present(cx, s.win, "dock-console-clear"));
    click_header(cx, &s);
    assert!(present(cx, s.win, "dock-console-clear"));
}

#[gpui_kit::test]
fn the_header_follows_the_zoom(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = short_with_dock(cx, &col);
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| p.update(cx, |s: &mut Settings| s.zoom = 1.5));
    });
    resize(cx, &s, 400.);
    let dock = bounds(cx, &s, "console-dock");
    let header = bounds(cx, &s, "dock-header");
    let want = dock_header(1.5);
    assert!(near(dock.size.height, want), "dock {dock:?}");
    assert!(near(header.size.height, want), "header {header:?}");
}
