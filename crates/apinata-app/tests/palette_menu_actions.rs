//! Palette commands for actions that began as menu-only items: New GraphQL request, Close
//! other tabs, Close saved tabs and Manage globals.

use crate::support;

use apinata_app::commands::registry;
use apinata_app::palette::{PaletteGroup, palette_items};
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext};

// Runs the palette entry `id` the way the palette does: its first target, dispatched from
// the window.
fn run(cx: &mut TestAppContext, win: AnyWindowHandle, id: &str, group: PaletteGroup) {
    let cmd = registry()
        .into_iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("{id} is a command"));
    assert_eq!(cmd.group, group, "{id}");
    let entry = palette_items()
        .into_iter()
        .find(|e| e.label == cmd.label)
        .unwrap_or_else(|| panic!("{id} is in the palette"));
    cx.update_window(win, |_, w, cx| {
        w.dispatch_action(entry.action.boxed_clone(), cx)
    })
    .unwrap();
    support::frame(cx, win);
}

#[gpui_kit::test]
fn new_graphql_request_opens_a_graphql_draft_at_the_root(cx: &mut TestAppContext) {
    let col = support::two_requests();
    let s = support::started(cx, &col);
    run(
        cx,
        s.win,
        "collection.new_graphql_request",
        PaletteGroup::Collection,
    );
    let tabs = support::tab_names(cx, &s.ws);
    let [draft] = tabs.as_slice() else {
        panic!("{tabs:?}")
    };
    assert!(support::ip(draft).is_draft(), "{draft}");
    let view = support::view_of(cx, &s.ws, &s.root, draft);
    assert_eq!(view.kind, apinata_core::RequestKind::Graphql);
}

#[gpui_kit::test]
fn new_graphql_request_without_a_collection_opens_a_scratch_draft(cx: &mut TestAppContext) {
    let col = support::two_requests();
    let s = support::started(cx, &col);
    s.ws.update(cx, |w, cx| w.close(&s.root, cx));
    support::frame(cx, s.win);
    run(
        cx,
        s.win,
        "collection.new_graphql_request",
        PaletteGroup::Collection,
    );
    cx.run_until_parked();
    assert_eq!(support::tab_names(cx, &s.ws).len(), 1);
    assert!(s.ws.read_with(cx, |w, _| w.is_scratch(&w.tabs().active().unwrap().root)));
}

#[gpui_kit::test]
fn close_other_tabs_keeps_only_the_active_tab(cx: &mut TestAppContext) {
    let col = support::two_requests();
    let s = support::started(cx, &col);
    support::open_tab(cx, &s.ws, &s.root, "a.yml");
    support::open_tab(cx, &s.ws, &s.root, "b.yml");
    cx.run_until_parked();
    run(cx, s.win, "tabs.close_others", PaletteGroup::Tabs);
    assert_eq!(support::tab_names(cx, &s.ws), ["b.yml"]);
}

#[gpui_kit::test]
fn close_other_tabs_without_tabs_does_nothing(cx: &mut TestAppContext) {
    let col = support::two_requests();
    let s = support::started(cx, &col);
    run(cx, s.win, "tabs.close_others", PaletteGroup::Tabs);
    assert!(support::tab_names(cx, &s.ws).is_empty());
    assert!(!support::confirm_open(cx));
}

#[gpui_kit::test]
fn close_saved_tabs_closes_clean_tabs_and_keeps_dirty_ones(cx: &mut TestAppContext) {
    let col = support::two_requests();
    let s = support::started(cx, &col);
    support::open_tab(cx, &s.ws, &s.root, "a.yml");
    support::open_tab(cx, &s.ws, &s.root, "b.yml");
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, support::edit_url("b.yml", "http://x/2"), cx)
    });
    cx.run_until_parked();
    run(cx, s.win, "tabs.close_saved", PaletteGroup::Tabs);
    assert_eq!(support::tab_names(cx, &s.ws), ["b.yml"]);
    assert!(!support::confirm_open(cx));
}

#[gpui_kit::test]
fn close_saved_tabs_with_only_dirty_tabs_closes_nothing(cx: &mut TestAppContext) {
    let col = support::two_requests();
    let s = support::started(cx, &col);
    support::open_tab(cx, &s.ws, &s.root, "a.yml");
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, support::edit_url("a.yml", "http://x/2"), cx)
    });
    cx.run_until_parked();
    run(cx, s.win, "tabs.close_saved", PaletteGroup::Tabs);
    assert_eq!(support::tab_names(cx, &s.ws), ["a.yml"]);
}

#[gpui_kit::test]
fn manage_globals_opens_the_global_environment_editor(cx: &mut TestAppContext) {
    let col = support::two_requests();
    let s = support::started(cx, &col);
    assert!(!support::present(cx, s.win, "global-env-editor"));
    run(
        cx,
        s.win,
        "environment.manage_globals",
        PaletteGroup::Environment,
    );
    assert!(support::present(cx, s.win, "global-env-editor"));
}

#[test]
fn menus_take_the_registry_names() {
    use apinata_app::sidebar::menu::MenuEntry;
    let named = |id: &str| registry().into_iter().find(|c| c.id == id).expect(id).label;
    assert_eq!(
        MenuEntry::NewGraphqlRequest.label(),
        named("collection.new_graphql_request")
    );
    assert_eq!(
        apinata_app::global_env_selector::manage_global_environments(),
        named("environment.manage_globals")
    );
    assert_eq!(MenuEntry::Close.label(), named("collection.close"));
    assert_eq!(
        apinata_app::tab_area::tab_menu_labels().to_vec(),
        vec![
            "Close",
            named("tabs.close_others"),
            named("tabs.close_to_right"),
            named("tabs.close_saved"),
            named("tabs.close_all"),
            named("tabs.reveal_in_sidebar"),
        ]
    );
}

#[gpui_kit::test]
fn close_collection_closes_the_open_collection(cx: &mut TestAppContext) {
    let col = support::two_requests();
    let s = support::started(cx, &col);
    assert!(s.ws.read_with(cx, |w, _| w.current_root().is_some()));
    run(cx, s.win, "collection.close", PaletteGroup::Collection);
    cx.run_until_parked();
    assert!(s.ws.read_with(cx, |w, _| w.current_root().is_none()));
}

#[gpui_kit::test]
fn close_collection_without_a_collection_does_nothing(cx: &mut TestAppContext) {
    let col = support::two_requests();
    let s = support::started(cx, &col);
    s.ws.update(cx, |w, cx| w.close(&s.root, cx));
    support::frame(cx, s.win);
    run(cx, s.win, "collection.close", PaletteGroup::Collection);
    assert!(s.ws.read_with(cx, |w, _| w.current_root().is_none()));
    assert!(!support::confirm_open(cx));
}
