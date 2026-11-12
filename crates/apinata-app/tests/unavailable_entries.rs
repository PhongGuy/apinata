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
