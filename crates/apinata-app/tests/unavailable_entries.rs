//! Menu entries with nothing to do are greyed out: the tab menu's close choices and the app
//! menu's Check for updates on a build without an update source.

use crate::support;

use apinata_app::app_menu::app_menu_disabled;
use apinata_app::commands::registry;
use apinata_app::tabs::TabId;
use gpui_kit::TestAppContext;
use support::*;

fn label(id: &str) -> &'static str {
    registry().into_iter().find(|c| c.id == id).expect(id).label
}

fn greyed(cx: &mut TestAppContext, s: &Started, tab: TabId) -> Vec<&'static str> {
    s.shell
        .read_with(cx, |sh, cx| sh.tab_area().read(cx).menu_disabled(tab, cx))
}

fn pin(cx: &mut TestAppContext, s: &Started, id: TabId, on: bool) {
    s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.set_pinned(id, on)));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_tab_menu_greys_out_close_choices_with_nothing_to_close(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let (others, right, saved, all) = (
        label("tabs.close_others"),
        label("tabs.close_to_right"),
        label("tabs.close_saved"),
        label("tabs.close_all"),
    );
    // One clean tab: nothing else to close, but it can close as saved or with all.
    let a = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let off = greyed(cx, &s, a);
    assert!(off.contains(&others) && off.contains(&right), "{off:?}");
    assert!(!off.contains(&saved) && !off.contains(&all), "{off:?}");
    assert!(!off.contains(&"Close"), "Close always applies");
    // Pinned alone: none of the bulk closes touch it.
    pin(cx, &s, a, true);
    let off = greyed(cx, &s, a);
    for l in [others, right, saved, all] {
        assert!(off.contains(&l), "{l} greyed on a lone pinned tab: {off:?}");
    }
    // A second, edited tab: Close other tabs and Close all have it to close, Close saved not.
    let b = open_tab(cx, &s.ws, &s.root, "b.yml");
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, edit_url("b.yml", "http://x/2"), cx)
    });
    cx.run_until_parked();
    frame(cx, s.win);
    let off = greyed(cx, &s, a);
    assert!(!off.contains(&others) && !off.contains(&all), "{off:?}");
    assert!(off.contains(&saved), "b holds edits: {off:?}");
    let _ = b;
}

#[test]
fn the_app_menu_greys_out_check_for_updates_without_an_update_source() {
    let check = label("view.check_for_updates");
    assert!(app_menu_disabled(false, true, false).contains(&check));
    assert!(!app_menu_disabled(false, true, true).contains(&check));
}

// A greyed app-menu entry says on hover why it cannot be used. The test collection is in no
// git repository and the test build has no update source.
#[gpui_kit::test]
fn greyed_app_menu_entries_say_why_on_hover(cx: &mut TestAppContext) {
    use apinata_app::app_menu::APP_MENU_REASON_TIP;
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    let col = two_requests();
    let s = started(cx, &col);
    let repo = "Open a collection in a git repository";
    for (id, reason) in [
        ("collection.switch_branch", repo),
        ("collection.show_changes", repo),
        (
            "view.check_for_updates",
            "Update checks are not configured for this build",
        ),
    ] {
        cx.update_window(s.win, |_, w, cx| {
            w.within("title-bar").click("app-menu", cx)
        })
        .unwrap();
        frame(cx, s.win);
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.within("popup-menu").hover(label(id), cx)
        })
        .unwrap();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(800));
        frame(cx, s.win);
        let tip = cx
            .update_window(s.win, |_, w, cx| {
                w.render_frame(cx);
                w.try_find(APP_MENU_REASON_TIP)
                    .and_then(|e| e.label().map(str::to_owned))
            })
            .unwrap();
        assert_eq!(tip.as_deref(), Some(reason), "{id}");
        cx.simulate_keystrokes(s.win, "escape");
        cx.run_until_parked();
    }
}

// A greyed entry's reason opens beside the menu, level with its row, so it never hides the
// entries below. The test collection is in no git repository.
#[gpui_kit::test]
fn greyed_app_menu_tips_open_beside_the_menu(cx: &mut TestAppContext) {
    use apinata_app::app_menu::{APP_MENU_REASON_TIP, app_menu_groups};
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    // The popup row of `label`, counting one separator row between groups.
    let row_of = |label: &str| {
        let mut row = 0;
        for (i, group) in app_menu_groups().iter().enumerate() {
            row += usize::from(i > 0);
            for (l, _) in group {
                if *l == label {
                    return row;
                }
                row += 1;
            }
        }
        panic!("{label} is not in the menu")
    };
    let col = two_requests();
    let s = started(cx, &col);
    for id in ["collection.switch_branch", "view.check_for_updates"] {
        cx.update_window(s.win, |_, w, cx| {
            w.within("title-bar").click("app-menu", cx)
        })
        .unwrap();
        frame(cx, s.win);
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.within("popup-menu").hover(label(id), cx)
        })
        .unwrap();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(800));
        frame(cx, s.win);
        let (tip, row, below, menu) = cx
            .update_window(s.win, |_, w, cx| {
                w.render_frame(cx);
                let tip = w.find(APP_MENU_REASON_TIP).bounds();
                let menu = w.within("popup-menu");
                let row = menu.find(row_of(label(id))).bounds();
                let below = menu.find(row_of(label(id)) + 1).bounds();
                (tip, row, below, w.find("popup-menu").bounds())
            })
            .unwrap();
        assert!(!tip.intersects(&below), "{id}: {tip:?} covers {below:?}");
        assert!(
            !tip.intersects(&menu),
            "{id}: {tip:?} covers the menu {menu:?}"
        );
        let gap = if tip.left() >= menu.right() {
            tip.left() - menu.right()
        } else {
            menu.left() - tip.right()
        };
        assert!(
            gap <= gpui_kit::px(24.),
            "{id}: {tip:?} is far from the menu {menu:?}"
        );
        assert!(
            (tip.center().y - row.center().y).abs() <= row.size.height,
            "{id}: {tip:?} is not level with its row {row:?}"
        );
        cx.simulate_keystrokes(s.win, "escape");
        cx.run_until_parked();
    }
}
