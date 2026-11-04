//! The compact environment control and the app menu in the title bar.

use crate::support;

use apinata_app::app_menu::{app_menu_groups, app_menu_index, app_menu_items};
use apinata_app::keymap::{NewRequest, bindings};
use apinata_app::shell::{
    ImportCollection, ImportEnvironment, NewCollection, OpenCollection, OpenCookies,
    OpenGlobalEnvironments,
};
use apinata_app::style::{ENV_CONTROL_WIDTH, at_zoom};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Action, AppContext as _, TestAppContext};
use support::*;

#[test]
fn the_menu_lists_existing_actions_under_unique_names() {
    let items = app_menu_items();
    let mut labels: Vec<&str> = items.iter().map(|(l, _)| *l).collect();
    assert!(
        labels
            .iter()
            .all(|l| !l.is_empty() && !l.to_lowercase().contains("bruno"))
    );
    let n = labels.len();
    labels.sort();
    labels.dedup();
    assert_eq!(labels.len(), n, "labels are unique");
    let names: Vec<&str> = items.iter().map(|(_, a)| a.name()).collect();
    for want in [
        "apinata::OpenCollection",
        "apinata::NewRequest",
        "apinata::OpenEnvironments",
        "apinata::SaveActive",
        "apinata::SaveAll",
        "apinata::OpenPreferences",
        "apinata::CheckForUpdates",
    ] {
        assert!(names.contains(&want), "{want} is in the menu");
    }
}

#[gpui_kit::test]
fn a_menu_entry_reaches_the_shell(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let ix = app_menu_index(&NewRequest).unwrap();
    let before = s.ws.read_with(cx, |w, _| w.tabs().entries().len());
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("app-menu", cx)
    })
    .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.within("popup-menu").click(ix, cx))
        .unwrap();
    cx.run_until_parked();
    let after = s.ws.read_with(cx, |w, _| w.tabs().entries().len());
    assert_eq!(after, before + 1);
}

#[gpui_kit::test]
fn the_environment_control_is_compact(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, _| {
        let mut bar = w.within("title-bar");
        assert_eq!(
            bar.find("env-control").bounds().size.width,
            at_zoom(ENV_CONTROL_WIDTH, 1.0)
        );
        assert!(bar.within("env-control").try_find("env-select").is_some());
    })
    .unwrap();
}

fn menu_click(cx: &mut TestAppContext, s: &Started, action: &dyn Action) {
    let ix = app_menu_index(action).expect("in the app menu");
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("app-menu", cx)
    })
    .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.within("popup-menu").click(ix, cx))
        .unwrap();
    cx.run_until_parked();
}

#[test]
fn the_menu_keeps_every_action_that_left_the_title_bar() {
    let names: Vec<&str> = app_menu_items().iter().map(|(_, a)| a.name()).collect();
    for want in [
        "apinata::OpenCollection",
        "apinata::ImportCollection",
        "apinata::NewCollection",
        "apinata::ImportEnvironment",
        "apinata::OpenCookies",
    ] {
        assert!(names.contains(&want), "{want} is in the menu");
    }
}

#[test]
fn the_collection_actions_keep_their_keys() {
    let actions: [&dyn Action; 3] = [&OpenCollection, &ImportCollection, &NewCollection];
    for action in actions {
        assert!(
            bindings().iter().any(|b| b.action().partial_eq(action)),
            "{} keeps its key",
            action.name()
        );
    }
}

#[gpui_kit::test]
fn cookies_in_the_menu_open_the_cookie_jar(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    assert!(
        !cx.update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
            .unwrap()
    );
    menu_click(cx, &s, &OpenCookies);
    assert!(
        cx.update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
            .unwrap()
    );
    assert!(
        present(cx, s.win, "cookie-jar"),
        "the cookie jar dialog is open"
    );
}

// Global environments are reachable from the menu, not only from the palette.
#[gpui_kit::test]
fn manage_globals_in_the_menu_opens_global_environments(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    menu_click(cx, &s, &OpenGlobalEnvironments);
    assert!(
        present(cx, s.win, "global-env-editor"),
        "the Global environments dialog is open"
    );
}

#[gpui_kit::test]
fn import_environment_in_the_menu_prompts_for_files(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    menu_click(cx, &s, &ImportEnvironment);
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|o| {
        assert!(o.files && o.multiple && !o.directories);
        None
    });
    cx.run_until_parked();
}

#[test]
fn the_menu_is_four_groups_named_by_the_registry() {
    let groups: Vec<Vec<&str>> = app_menu_groups()
        .iter()
        .map(|g| g.iter().map(|(l, _)| *l).collect())
        .collect();
    assert_eq!(
        groups,
        [
            vec![
                "Open collection…",
                "New collection…",
                "Import collection…",
                "Import cURL",
                "Export collection to Postman",
                "Export collection to OpenAPI",
                "Export collection to HTML documentation",
                "Run collection",
                "Collection settings",
            ],
            vec!["New request", "Save", "Save all", "Generate code"],
            vec![
                "Manage environments",
                "Manage globals…",
                "Import environment…",
                "Cookies",
                "Runtime variables",
                "History",
            ],
            vec!["Preferences", "Check for updates", "Command palette"],
        ]
    );
    let names: Vec<&str> = app_menu_items().iter().map(|(_, a)| a.name()).collect();
    for gone in ["apinata::ToggleSidebar", "apinata::ToggleOrientation"] {
        assert!(!names.contains(&gone), "{gone} left the menu");
    }
}

#[gpui_kit::test]
fn separators_split_the_open_menu_into_its_groups(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("app-menu", cx)
    })
    .unwrap();
    frame(cx, s.win);
    let rows: Vec<Option<String>> = cx
        .update_window(s.win, |_, w, _| {
            (0..26usize)
                .map(|ix| {
                    w.within("popup-menu")
                        .try_find(ix)
                        .map(|e| e.label().unwrap_or("").to_owned())
                })
                .collect()
        })
        .unwrap();
    let mut want: Vec<Option<String>> = Vec::new();
    for (i, group) in app_menu_groups().iter().enumerate() {
        if i > 0 {
            want.push(Some(String::new()));
        }
        want.extend(group.iter().map(|(l, _)| Some(l.to_string())));
    }
    want.push(None);
    assert_eq!(rows, want);
    assert_eq!(app_menu_index(&OpenCookies), Some(18));
}

// Hovers the app-menu slot long enough for a tooltip, then reports whether its tip is drawn.
fn tip_after_hover(cx: &mut TestAppContext, s: &Started) -> bool {
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.within("title-bar").hover("app-menu-slot", cx)
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(800));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(apinata_app::shell::APP_MENU_TIP_ID).is_some()
    })
    .unwrap()
}

// The slot's tooltip would cover the open menu's first row, so it shows only while closed.
#[gpui_kit::test]
fn the_app_menu_tooltip_shows_only_while_the_menu_is_closed(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    assert!(tip_after_hover(cx, &s), "a hover shows the tooltip");
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("app-menu", cx)
    })
    .unwrap();
    frame(cx, s.win);
    assert!(present(cx, s.win, "popup-menu"), "the menu is open");
    assert!(!tip_after_hover(cx, &s), "no tooltip over the open menu");
    cx.simulate_keystrokes(s.win, "escape");
    cx.run_until_parked();
    assert!(!present(cx, s.win, "popup-menu"), "the menu is closed");
    assert!(tip_after_hover(cx, &s), "a hover shows it again");
    // Closed by choosing an entry, the menu gives its tooltip back too.
    let ix = app_menu_index(&NewRequest).unwrap();
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("app-menu", cx)
    })
    .unwrap();
    frame(cx, s.win);
    assert!(
        !tip_after_hover(cx, &s),
        "no tooltip over the reopened menu"
    );
    cx.update_window(s.win, |_, w, cx| w.within("popup-menu").click(ix, cx))
        .unwrap();
    cx.run_until_parked();
    assert!(
        !present(cx, s.win, "popup-menu"),
        "the entry closes the menu"
    );
    assert!(tip_after_hover(cx, &s), "a hover after an entry shows it");
}
