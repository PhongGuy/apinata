//! The tab strip: method tags on request tabs, a close button on every clean tab, and the
//! dirty dot in its place on inactive dirty tabs until hovered.

use crate::support;

use apinata_app::style::{TAB_MAX_WIDTH, at_zoom};
use apinata_app::tabs::TabKind;
use gpui_kit::AppContext as _;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt;
use support::*;

fn hover(cx: &mut TestAppContext, a: &Area, id: impl Into<gpui_kit::ElementId>) {
    let id = id.into();
    cx.update_window(a.win, |_, w, cx| w.hover(id, cx)).unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn every_clean_tab_keeps_its_close_button(cx: &mut TestAppContext) {
    let a = open_area(cx);
    open_tab(cx, &a.ws, &a.root, "a.yml");
    open_tab(cx, &a.ws, &a.root, "b.yml");
    cx.run_until_parked();
    assert!(present(cx, a.win, ("tab-close", 1usize)), "the active tab");
    let visible = cx
        .update_window(a.win, |_, w, _| w.find(("tab-close", 0usize)).visible())
        .unwrap();
    assert!(
        visible,
        "an inactive clean tab can still be closed in one click"
    );
}

#[gpui_kit::test]
fn moving_between_tabs_moves_the_hover(cx: &mut TestAppContext) {
    let a = open_area(cx);
    open_tab(cx, &a.ws, &a.root, "a.yml");
    open_tab(cx, &a.ws, &a.root, "b.yml");
    a.ws.update(cx, |w, cx| {
        w.send(&a.root, edit_url("a.yml", "http://x/1"), cx)
    });
    cx.run_until_parked();
    hover(cx, &a, ("tab", 0usize));
    assert!(present(cx, a.win, ("tab-close", 0usize)));
    hover(cx, &a, ("tab", 1usize));
    assert!(
        present(cx, a.win, ("dirty", 0usize)),
        "leaving tab 0 restores its dot"
    );
    assert!(!present(cx, a.win, ("tab-close", 0usize)));
    hover(cx, &a, "url");
    assert!(present(cx, a.win, ("dirty", 0usize)));
}

#[gpui_kit::test]
fn a_dirty_inactive_tab_shows_its_dot_until_hovered(cx: &mut TestAppContext) {
    let a = open_area(cx);
    open_tab(cx, &a.ws, &a.root, "a.yml");
    open_tab(cx, &a.ws, &a.root, "b.yml");
    a.ws.update(cx, |w, cx| {
        w.send(&a.root, edit_url("a.yml", "http://x/1"), cx)
    });
    cx.run_until_parked();
    // The test pointer starts at the window origin, which is over tab 0.
    hover(cx, &a, "url");
    assert!(present(cx, a.win, ("dirty", 0usize)));
    assert!(!present(cx, a.win, ("tab-close", 0usize)));
    hover(cx, &a, ("tab", 0usize));
    assert!(present(cx, a.win, ("tab-close", 0usize)));
    assert!(!present(cx, a.win, ("dirty", 0usize)));
    // Clicking the revealed button closes the tab, which is dirty, so it asks first.
    cx.update_window(a.win, |_, w, cx| w.click(("tab-close", 0usize), cx))
        .unwrap();
    cx.run_until_parked();
    assert!(confirm_open(cx));
    answer_confirm(cx, "confirm-cancel");
    cx.run_until_parked();
}

#[gpui_kit::test]
fn request_tabs_carry_a_method_tag(cx: &mut TestAppContext) {
    let a = open_area(cx);
    open_tab(cx, &a.ws, &a.root, "a.yml");
    a.ws.update(cx, |w, cx| {
        w.open_settings(&a.root, TabKind::Collection, cx)
    });
    cx.run_until_parked();
    assert!(present(cx, a.win, ("tab-method", 0usize)));
    assert!(!present(cx, a.win, ("tab-method", 1usize)));
}

#[gpui_kit::test]
fn the_preview_id_sits_on_the_name(cx: &mut TestAppContext) {
    let a = open_area(cx);
    a.ws.update(cx, |w, cx| w.open_item(&a.root, ip("a.yml"), true, cx));
    cx.run_until_parked();
    let (tab, name) = cx
        .update_window(a.win, |_, w, cx| {
            w.render_frame(cx);
            (
                w.find(("tab", 0usize)).bounds(),
                w.find(("preview", 0usize)).bounds(),
            )
        })
        .unwrap();
    assert!(
        tab.contains(&name.center()),
        "the name is inside the tab label"
    );
    let method = cx
        .update_window(a.win, |_, w, _| w.find(("tab-method", 0usize)).bounds())
        .unwrap();
    assert!(method.right() <= name.left(), "the tag comes first");
}

/// A tab area over a collection with two requests named `request_1`, one at the root and
/// one in `folder_1`, and a uniquely named `request_2`.
fn duplicate_area(
    cx: &mut TestAppContext,
) -> (
    tempfile::TempDir,
    tempfile::TempDir,
    gpui_kit::Entity<apinata_app::workspace::Workspace>,
    std::path::PathBuf,
    gpui_kit::AnyWindowHandle,
) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("request_1.yml", &request("request_1", "http://x/1")),
        ("folder_1/folder.yml", &folder("folder_1")),
        (
            "folder_1/request_1.yml",
            &request("request_1", "http://x/f1"),
        ),
        ("request_2.yml", &request("request_2", "http://x/2")),
    ]);
    let (state, ws, root) = open_dir(cx, &col);
    support::init(cx);
    let shared = ws.clone();
    let (win, _view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| apinata_app::tab_area::TabArea::new(shared, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    (col, state, ws, root, win)
}

#[gpui_kit::test]
fn tabs_sharing_a_name_show_a_muted_detail_after_it(cx: &mut TestAppContext) {
    let (_col, _state, ws, root, win) = duplicate_area(cx);
    open_tab(cx, &ws, &root, "folder_1/request_1.yml");
    open_tab(cx, &ws, &root, "request_2.yml");
    open_tab(cx, &ws, &root, "request_1.yml");
    cx.run_until_parked();
    assert!(
        present(cx, win, ("tab-detail", 0usize)),
        "folder_1/request_1"
    );
    assert!(
        !present(cx, win, ("tab-detail", 1usize)),
        "request_2 is unique"
    );
    assert!(
        present(cx, win, ("tab-detail", 2usize)),
        "the root request_1"
    );
    cx.update_window(win, |_, w, _| {
        for ix in [0usize, 2] {
            let method = w.find(("tab-method", ix)).bounds();
            let detail = w.find(("tab-detail", ix)).bounds();
            let label = w.find(("tab", ix)).bounds();
            assert!(
                detail.left() > method.right(),
                "the detail follows the name"
            );
            assert!(detail.right() <= label.right(), "inside the tab's label");
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn closing_a_duplicate_drops_the_other_detail(cx: &mut TestAppContext) {
    let (_col, _state, ws, root, win) = duplicate_area(cx);
    let first = open_tab(cx, &ws, &root, "folder_1/request_1.yml");
    open_tab(cx, &ws, &root, "request_1.yml");
    cx.run_until_parked();
    assert!(present(cx, win, ("tab-detail", 1usize)));
    ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.close(first)));
    cx.run_until_parked();
    assert!(
        !present(cx, win, ("tab-detail", 0usize)),
        "the name is unique again"
    );
}

#[gpui_kit::test]
fn a_long_request_name_is_cut_to_the_tab_width(cx: &mut TestAppContext) {
    let long = "A request whose name is far longer than any tab should ever grow";
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("long.yml", &request(long, "http://x/")),
        ("short.yml", &request("Short", "http://x/")),
    ]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "long.yml");
    open_tab(cx, &s.ws, &s.root, "short.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        let tab = w.find(("tab", 0usize)).bounds();
        assert!(tab.size.width <= at_zoom(TAB_MAX_WIDTH, 1.0), "tab {tab:?}");
        let name = w.find(("tab-name", 0usize)).bounds();
        assert!(
            name.right() <= tab.right(),
            "name {name:?} past its tab {tab:?}"
        );
        // A cut name stays on one line: as tall as a short name, not wrapped to several.
        let short = w.find(("tab-name", 1usize)).bounds();
        assert_eq!(name.size.height, short.size.height, "name {name:?} wrapped");
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_all_tabs_menu_shows_the_same_detail_as_the_tabs(cx: &mut TestAppContext) {
    let (_col, _state, ws, root, win) = duplicate_area(cx);
    open_tab(cx, &ws, &root, "folder_1/request_1.yml");
    open_tab(cx, &ws, &root, "request_2.yml");
    open_tab(cx, &ws, &root, "request_1.yml");
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.click("tab-overflow", cx))
        .unwrap();
    frame(cx, win);
    assert!(present(cx, win, ("tab-menu-detail", 0usize)), "folder_1");
    assert!(
        !present(cx, win, ("tab-menu-detail", 1usize)),
        "request_2 is unique"
    );
    assert!(
        present(cx, win, ("tab-menu-detail", 2usize)),
        "the root one"
    );
}

#[gpui_kit::test]
fn the_all_tabs_menu_marks_unsaved_tabs_with_the_dot(cx: &mut TestAppContext) {
    let a = open_area(cx);
    open_tab(cx, &a.ws, &a.root, "a.yml");
    open_tab(cx, &a.ws, &a.root, "b.yml");
    a.ws.update(cx, |w, cx| {
        w.send(&a.root, edit_url("a.yml", "http://x/1"), cx)
    });
    cx.run_until_parked();
    cx.update_window(a.win, |_, w, cx| w.click("tab-overflow", cx))
        .unwrap();
    frame(cx, a.win);
    assert!(present(cx, a.win, ("tab-menu-dirty", 0usize)), "a.yml");
    assert!(
        !present(cx, a.win, ("tab-menu-dirty", 1usize)),
        "b.yml is saved"
    );
}
