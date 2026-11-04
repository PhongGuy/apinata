use crate::support;

use apinata_app::sidebar::marker_id;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

fn two_selected(cx: &mut TestAppContext) -> SidebarHarness {
    let h = open_sidebar_with(cx, multi_files());
    cx.update_window(h.win, |_, w, cx| w.click(row("a.yml"), cx))
        .unwrap();
    secondary_click(cx, h.win, row("c.yml"));
    focus_tree(cx, &h);
    frame(cx, h.win);
    h
}

fn list_active(cx: &mut TestAppContext) -> gpui_kit::Hsla {
    cx.update(|cx| cx.theme().list_active)
}

#[gpui_kit::test]
fn a_non_focus_member_gets_a_fill_and_a_muted_edge_but_a_bystander_does_not(
    cx: &mut TestAppContext,
) {
    let h = two_selected(cx);
    let fill = list_active(cx);
    assert!(
        filled(cx, h.win, row("a.yml"), fill),
        "the member a is filled"
    );
    assert!(
        shown(cx, h.win, marker_id("edge", &row("a.yml"))),
        "and has the muted edge"
    );
    assert!(
        shown(cx, h.win, marker_id("focus-edge", &row("c.yml"))),
        "the focus row keeps the primary edge"
    );
    assert!(!shown(cx, h.win, marker_id("edge", &row("c.yml"))));
    assert!(!filled(cx, h.win, row("b.yml"), fill), "b is not selected");
    assert!(!shown(cx, h.win, marker_id("edge", &row("b.yml"))));
}

#[gpui_kit::test]
fn one_key_paints_exactly_what_a_single_selection_does(cx: &mut TestAppContext) {
    let h = two_selected(cx);
    secondary_click(cx, h.win, row("c.yml"));
    focus_tree(cx, &h);
    frame(cx, h.win);
    assert!(shown(cx, h.win, marker_id("focus-edge", &row("a.yml"))));
    assert!(!shown(cx, h.win, marker_id("edge", &row("a.yml"))));
    assert!(!shown(cx, h.win, marker_id("focus-edge", &row("c.yml"))));
}

#[gpui_kit::test]
fn every_member_reports_selected_to_assistive_technology(cx: &mut TestAppContext) {
    let h = two_selected(cx);
    cx.update_window(h.win, |_, w, cx| {
        w.render_frame(cx);
        assert_eq!(
            w.find(row("a.yml")).selected(),
            Some(true),
            "non-focus member"
        );
        assert_eq!(w.find(row("c.yml")).selected(), Some(true), "focus member");
        assert_eq!(w.find(row("b.yml")).selected(), Some(false), "bystander");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_single_selection_adds_no_selected_state_to_the_inner_rows(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, multi_files());
    cx.update_window(h.win, |_, w, cx| w.click(row("a.yml"), cx))
        .unwrap();
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, _| {
        assert_ne!(w.find(row("b.yml")).selected(), Some(true));
        assert_eq!(
            w.find(row("b.yml")).selected(),
            None,
            "no role while the set is inactive"
        );
    })
    .unwrap();
}
