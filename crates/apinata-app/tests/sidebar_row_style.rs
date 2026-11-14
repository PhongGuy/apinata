//! Sidebar rows: chevrons and icons on folders, a fixed method column indented per
//! level, and the dirty dot at the right edge.

use crate::support;

use apinata_app::sidebar::{
    chevron_icon, folder_icon, marker_id, selection_edge_color, selection_edges,
};
use apinata_app::style::{METHOD_COLUMN, SPACE_3, at_zoom};
use gpui_kit::assets::IconName;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, Bounds, Pixels, TestAppContext};
use support::*;

fn bounds(
    cx: &mut TestAppContext,
    h: &SidebarHarness,
    id: gpui_kit::SharedString,
) -> Bounds<Pixels> {
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, _| w.find(id).bounds())
        .unwrap()
}

#[test]
fn icons_follow_expansion() {
    assert_eq!(chevron_icon(true), IconName::ChevronDown);
    assert_eq!(chevron_icon(false), IconName::ChevronRight);
    assert_eq!(folder_icon(true), IconName::FolderOpen);
    assert_eq!(folder_icon(false), IconName::Folder);
}

#[gpui_kit::test]
fn folders_have_a_chevron_and_requests_a_method_column(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    assert!(shown(cx, h.win, marker_id("chevron", &row("users"))));
    assert!(shown(cx, h.win, marker_id("icon", &row("users"))));
    assert!(!shown(cx, h.win, marker_id("chevron", &row("health.yml"))));
    let method = bounds(cx, &h, marker_id("method", &row("health.yml")));
    assert_eq!(method.size.width, at_zoom(METHOD_COLUMN, 1.0));
}

#[gpui_kit::test]
fn each_level_indents_by_twelve_pixels(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    cx.update_window(h.win, |_, w, cx| w.click(row("users"), cx))
        .unwrap();
    cx.run_until_parked();
    let top = bounds(cx, &h, marker_id("method", &row("health.yml")));
    let nested = bounds(cx, &h, marker_id("method", &row("users/list.yml")));
    assert_eq!(nested.left() - top.left(), at_zoom(SPACE_3, 1.0));
}

#[gpui_kit::test]
fn the_dirty_dot_sits_at_the_right_edge(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    h.ws.update(cx, |w, cx| {
        w.send(&h.root, edit_url("health.yml", "http://x/9"), cx)
    });
    cx.run_until_parked();
    let row_bounds = bounds(cx, &h, row("health.yml"));
    let dot = bounds(cx, &h, marker_id("dirty", &row("health.yml")));
    assert!(
        dot.left() > row_bounds.center().x,
        "{dot:?} in {row_bounds:?}"
    );
    assert!(
        row_bounds.right() - dot.right()
            <= apinata_app::style::at_zoom(apinata_app::style::SPACE_4, 1.0)
    );
}

#[gpui_kit::test]
fn folder_icons_and_method_tags_start_in_one_column(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let icon = bounds(cx, &h, marker_id("icon", &row("users")));
    let method = bounds(cx, &h, marker_id("method", &row("health.yml")));
    assert_eq!(icon.left(), method.left());
}

#[gpui_kit::test]
fn selecting_a_row_does_not_shift_its_content(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let before = bounds(cx, &h, marker_id("method", &row("health.yml")));
    cx.update_window(h.win, |_, w, cx| w.click(row("health.yml"), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        h.view.read_with(cx, |s, cx| s.selected(cx)),
        Some((h.root.clone(), ip("health.yml")))
    );
    let after = bounds(cx, &h, marker_id("method", &row("health.yml")));
    assert_eq!(after.left(), before.left());
}

#[test]
fn the_selection_is_a_two_pixel_left_edge_without_an_outline() {
    use gpui_kit::px;
    let edge = selection_edges();
    assert_eq!(edge.left, px(2.));
    assert_eq!(
        (edge.top, edge.right, edge.bottom),
        (px(0.), px(0.), px(0.))
    );
}

#[gpui_kit::test]
fn the_edge_is_the_accent_only_while_the_tree_has_focus(cx: &mut TestAppContext) {
    use gpui_kit::component::{ActiveTheme as _, ThemeMode};
    support::init(cx);
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        cx.update(|cx| apinata_app::theme::set_mode(mode, None, cx));
        cx.update(|cx| {
            let t = cx.theme();
            assert_eq!(selection_edge_color(true, cx), t.primary, "{mode:?}");
            assert_eq!(
                selection_edge_color(false, cx),
                t.muted_foreground,
                "{mode:?} the unfocused edge stays visible on the fill"
            );
            assert_ne!(t.list_active, t.primary, "{mode:?} the fill is neutral");
        });
    }
}

#[gpui_kit::test]
fn only_the_focused_selection_has_the_accent_edge(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let id = row("health.yml");
    cx.update_window(h.win, |_, w, cx| w.click(id.clone(), cx))
        .unwrap();
    cx.run_until_parked();
    focus_tree(cx, &h);
    assert!(
        shown(cx, h.win, marker_id("focus-edge", &id)),
        "focused: bar and outline"
    );
    assert!(!shown(cx, h.win, marker_id("edge", &id)));
    assert_eq!(
        bounds(cx, &h, marker_id("focus-edge", &id)),
        bounds(cx, &h, id.clone()),
        "the outline and the bar share the row's own box"
    );
    let filter = h.view.read_with(cx, |s, _| s.filter().clone());
    cx.update_window(h.win, |_, w, cx| filter.update(cx, |i, cx| i.focus(w, cx)))
        .unwrap();
    assert!(
        shown(cx, h.win, marker_id("edge", &id)),
        "unfocused: the bar only"
    );
    assert!(!shown(cx, h.win, marker_id("focus-edge", &id)));
    assert_eq!(
        bounds(cx, &h, marker_id("edge", &id)),
        bounds(cx, &h, id.clone())
    );
    let other = row("status.yml");
    assert!(
        !shown(cx, h.win, marker_id("edge", &other)),
        "unselected rows draw none"
    );
    assert!(!shown(cx, h.win, marker_id("focus-edge", &other)));
}

#[gpui_kit::test]
fn tabbing_into_a_tree_without_a_selection_selects_the_first_row(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    // Focus listeners only run in an active window, as the app's window is when Tab is pressed.
    cx.update_window(h.win, |_, w, _| w.activate_window())
        .unwrap();
    let filter = h.view.read_with(cx, |s, _| s.filter().clone());
    cx.update_window(h.win, |_, w, cx| filter.update(cx, |i, cx| i.focus(w, cx)))
        .unwrap();
    frame(cx, h.win);
    assert!(
        h.view.read_with(cx, |s, cx| s.selected(cx)).is_none(),
        "the case needs a tree with nothing selected"
    );
    // The History button is the stop between the filter and the tree.
    for _ in 0..2 {
        cx.update_window(h.win, |_, w, cx| w.focus_next(cx))
            .unwrap();
        frame(cx, h.win);
    }
    let tree = h.view.read_with(cx, |s, _| s.tree().clone());
    let (ix, first) = tree.read_with(cx, |t, _| {
        (t.selected_index(), t.entry(0).map(|e| e.item().id.clone()))
    });
    assert_eq!(ix, Some(0), "Tab into the tree selects its first row");
    let first = first.expect("a first row");
    assert!(
        shown(cx, h.win, marker_id("focus-edge", &first)),
        "the first row shows the focus outline"
    );
    assert!(
        h.ws.read_with(cx, |w, _| w.tabs().entries().is_empty()),
        "selecting on Tab-in opens nothing"
    );
}

// Guard: a click focuses the kit tree directly, not through `tree_focus`, and must still ring.
#[gpui_kit::test]
fn clicking_a_row_shows_its_outline(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let id = row("health.yml");
    cx.update_window(h.win, |_, w, cx| w.click(id.clone(), cx))
        .unwrap();
    cx.run_until_parked();
    assert!(
        shown(cx, h.win, marker_id("focus-edge", &id)),
        "a click focuses the tree"
    );
}

// A selected request row keeps its method colour in both themes; the selected fill is neutral.
#[gpui_kit::test]
fn a_selected_row_keeps_its_method_colour(cx: &mut TestAppContext) {
    use gpui_kit::component::{ActiveTheme as _, ThemeMode};
    let with = |m: &str| request("R", "http://x/r").replace("method: GET", &format!("method: {m}"));
    let h = open_sidebar_with(
        cx,
        vec![
            ("opencollection.yml", ROOT.to_string()),
            ("d.yml", with("DELETE")),
            ("p.yml", with("POST")),
        ],
    );
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        cx.update(|cx| apinata_app::theme::set_mode(mode, None, cx));
        for (file, danger) in [("d.yml", true), ("p.yml", false)] {
            cx.update_window(h.win, |_, w, cx| w.click(row(file), cx))
                .unwrap();
            cx.run_until_parked();
            focus_tree(cx, &h);
            assert!(
                shown(cx, h.win, marker_id("method", &row(file))),
                "{mode:?} {file} draws its tag"
            );
            let picked = h.view.read_with(cx, |s, cx| s.selected(cx)).map(|(_, p)| p);
            assert_eq!(picked, Some(ip(file)), "{mode:?}");
            frame(cx, h.win);
            let painted = h.view.read_with(cx, |s, _| s.painted_tag_color(&row(file)));
            let (want, foreground) = cx.update(|cx| {
                let t = cx.theme();
                (if danger { t.danger } else { t.warning }, t.foreground)
            });
            assert_eq!(
                painted,
                Some(want),
                "{mode:?} {file} painted its method colour"
            );
            assert_ne!(
                want, foreground,
                "{mode:?} the method colour is not the row text"
            );
        }
    }
}
