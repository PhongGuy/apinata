//! The tab strip and the request editor keep one inset from their pane's edges.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::request_editor::Section;
use apinata_app::settings::Orientation;
use apinata_app::style::{FORM_INPUT_WIDTH, PANE_INSET, SPACE_4, at_zoom};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, px, size};
use support::*;

#[gpui_kit::test]
fn the_first_tab_and_the_url_row_clear_the_pane_edges(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        let sidebar = w.find("sidebar").bounds();
        let area = w.find("tab-area").bounds();
        let tab = w.find(("tab", 0usize)).bounds();
        let method = w.find("method").bounds();
        let send = w.find("send").bounds();
        let width = w.viewport_size().width;
        assert!(
            tab.left() >= sidebar.right() + at_zoom(PANE_INSET, 1.0),
            "tab {tab:?}, sidebar {sidebar:?}"
        );
        assert!(
            method.left() >= area.left() + at_zoom(PANE_INSET, 1.0),
            "method {method:?}, area {area:?}"
        );
        assert!(
            send.right() <= area.right() - at_zoom(PANE_INSET, 1.0),
            "send {send:?}, area {area:?}"
        );
        assert!(
            send.right() <= width - at_zoom(PANE_INSET, 1.0),
            "send {send:?}, window {width:?}"
        );
        assert!(
            (tab.left() - method.left()).abs() < px(1.),
            "the first tab lines up with the method"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_editor_insets_both_panes_and_its_top(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let width = w.viewport_size().width;
        for id in ["request-column", "response"] {
            let b = w.find(id).bounds();
            assert!(b.left() >= at_zoom(PANE_INSET, 1.0), "{id} left: {b:?}");
            assert!(
                b.right() <= width - at_zoom(PANE_INSET, 1.0),
                "{id} right: {b:?}"
            );
        }
        let request = w.find("request-column").bounds();
        let response = w.find("response").bounds();
        assert_eq!(request.left(), response.left(), "one inset for both panes");
        let crumb = w.find("breadcrumb").bounds();
        assert!(
            crumb.top() >= at_zoom(PANE_INSET, 1.0),
            "breadcrumb {crumb:?}"
        );
    })
    .unwrap();
}

fn side_by_side(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| {
            p.update(cx, |s| s.response_orientation = Orientation::Horizontal)
        });
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_split_keeps_a_gutter_on_both_sides_of_the_divider(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        let request = w.find("request-column").bounds();
        let response = w.find("response").bounds();
        assert!(
            response.top() - request.bottom() >= at_zoom(SPACE_4, 1.0),
            "stacked: request {request:?}, response {response:?}"
        );
    })
    .unwrap();
    side_by_side(cx);
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        let request = w.find("request-column").bounds();
        let response = w.find("response").bounds();
        let send = w.find("send").bounds();
        assert!(
            response.left() - request.right() >= at_zoom(SPACE_4, 1.0),
            "side by side: {request:?} {response:?}"
        );
        assert!(
            response.left() - send.right() >= at_zoom(SPACE_4, 1.0),
            "send {send:?} against the divider"
        );
    })
    .unwrap();
}

const TIMED: &str = "info:\n  name: t\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://x/0\"\nsettings:\n  encodeUrl: true\n  timeout: 5000\n  followRedirects: true\n  maxRedirects: 5\n  forwardAuthorizationHeader: true\n";

#[gpui_kit::test]
fn the_settings_form_fits_a_side_by_side_column(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("t.yml", TIMED)]);
    let s = started(cx, &col);
    cx.simulate_window_resize(s.win, size(px(1280.), px(800.)));
    side_by_side(cx);
    let tab = open_tab(cx, &s.ws, &s.root, "t.yml");
    frame(cx, s.win);
    let ed = editor_of(cx, &s, tab);
    give_result(cx, &ed);
    // Ten section tabs overflow a side-by-side column, so Settings is picked from the menu.
    frame(cx, s.win);
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("section-overflow", cx))
        .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.within("popup-menu").click(Section::Settings as usize, cx)
    })
    .unwrap();
    frame(cx, s.win);
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        let column = w.find("section-body").bounds();
        let control = w.within("set-row-timeout").find("form-control").bounds();
        let input = w.find("set-timeout").bounds();
        assert!(
            control.right() <= column.right(),
            "control {control:?} past {column:?}"
        );
        assert!(
            input.right() <= column.right(),
            "input {input:?} past {column:?}"
        );
        assert_eq!(input.size.width, at_zoom(FORM_INPUT_WIDTH, 1.0));
    })
    .unwrap();
}

// Every tab kind starts its content where the first tab starts, as the request editor does.
#[gpui_kit::test]
fn settings_and_environments_line_up_with_the_first_tab(cx: &mut TestAppContext) {
    use apinata_app::tabs::TabKind;
    let col = two_requests();
    let s = started(cx, &col);
    for (kind, id) in [
        (TabKind::Collection, "config-title"),
        (TabKind::Environments, "env-list"),
    ] {
        s.ws.update(cx, |w, cx| w.open_settings(&s.root, kind, cx));
        frame(cx, s.win);
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            let tab = w.find(("tab", 0usize)).bounds();
            let b = w.find(id).bounds();
            assert!(
                (tab.left() - b.left()).abs() < px(1.),
                "{id} {b:?}, tab {tab:?}"
            );
        })
        .unwrap();
    }
}

// Side by side, the unsent response strip starts on the breadcrumb's line.
#[gpui_kit::test]
fn side_by_side_the_response_strip_shares_the_breadcrumb_top(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    side_by_side(cx);
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let crumb = w.find("breadcrumb-collection").bounds();
        let label = w.find("response-strip-label").bounds();
        assert!(
            (label.top() - crumb.top()).abs() < px(1.),
            "{label:?} against {crumb:?}"
        );
    })
    .unwrap();
}
