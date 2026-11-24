//! The title bar's one environment picker: a collection section and a global section that
//! choose independently, under one button that names both.

use crate::support;

use apinata_app::env_picker::{
    ACTIONS_GROUP, COLLECTION_GROUP, GLOBAL_GROUP, IMPORT_ENVIRONMENT, MANAGE_ENVIRONMENTS,
    NO_ENVIRONMENT, NO_GLOBAL_ENVIRONMENT, global_option_id, manage_global_environments, option_id,
};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, ElementId, SharedString, TestAppContext};
use support::*;

fn with_envs() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
        ("environments/dev.yml", "name: dev\n"),
        ("environments/stage.yml", "name: stage\n"),
    ])
}

// The app with `with_envs` open and two global environments, g and h, on disk.
fn start(cx: &mut TestAppContext, col: &tempfile::TempDir) -> Started {
    let s = started_with(cx, col, |paths| {
        std::fs::create_dir_all(&paths.global_environments).unwrap();
        std::fs::write(paths.global_environments.join("g.yml"), "name: g\n").unwrap();
        std::fs::write(paths.global_environments.join("h.yml"), "name: h\n").unwrap();
    });
    settle(cx);
    frame(cx, s.win);
    s
}

fn pick(cx: &mut TestAppContext, s: &Started, row: impl Into<ElementId>) {
    let row = row.into();
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("env-select", cx)
    })
    .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.within("popup-menu").click(row, cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
}

fn collection_env(cx: &mut TestAppContext, s: &Started) -> Option<String> {
    s.ws.read_with(cx, |w, cx| w.selected_environment(&s.root, cx))
}

fn global_env(cx: &mut TestAppContext, s: &Started) -> Option<String> {
    s.ws.read_with(cx, |w, _| w.selected_global_environment())
}

fn label(cx: &mut TestAppContext, s: &Started) -> SharedString {
    let picker = s.shell.read_with(cx, |v, _| v.env_picker().clone());
    picker.read_with(cx, |p, cx| p.label(cx))
}

#[gpui_kit::test]
fn one_picker_replaces_the_two_selects(cx: &mut TestAppContext) {
    let col = with_envs();
    let s = start(cx, &col);
    assert!(present(cx, s.win, "env-select"));
    assert!(!present(cx, s.win, "global-env-selector"));
    assert!(!present(cx, s.win, "global-env-select"));
}

#[gpui_kit::test]
fn the_menu_lists_the_collection_then_the_globals_then_the_actions(cx: &mut TestAppContext) {
    let col = with_envs();
    let s = start(cx, &col);
    let picker = s.shell.read_with(cx, |v, _| v.env_picker().clone());
    let sections = picker.read_with(cx, |p, cx| p.sections(cx));
    let s = |items: &[&str]| -> Vec<SharedString> {
        items.iter().map(|i| SharedString::from(*i)).collect()
    };
    assert_eq!(
        sections,
        vec![
            (
                COLLECTION_GROUP.into(),
                s(&[NO_ENVIRONMENT, "dev", "stage"])
            ),
            (GLOBAL_GROUP.into(), s(&[NO_GLOBAL_ENVIRONMENT, "g", "h"])),
            (
                ACTIONS_GROUP.into(),
                s(&[
                    MANAGE_ENVIRONMENTS,
                    IMPORT_ENVIRONMENT,
                    manage_global_environments()
                ])
            ),
        ]
    );
}

#[gpui_kit::test]
fn picking_a_global_keeps_the_collection_environment(cx: &mut TestAppContext) {
    let col = with_envs();
    let s = start(cx, &col);
    pick(cx, &s, option_id("dev"));
    assert_eq!(collection_env(cx, &s).as_deref(), Some("dev"));
    assert_eq!(global_env(cx, &s), None);
    pick(cx, &s, global_option_id("g"));
    assert_eq!(global_env(cx, &s).as_deref(), Some("g"));
    assert_eq!(collection_env(cx, &s).as_deref(), Some("dev"));
    pick(cx, &s, global_option_id(NO_GLOBAL_ENVIRONMENT));
    assert_eq!(global_env(cx, &s), None);
    assert_eq!(collection_env(cx, &s).as_deref(), Some("dev"));
}

#[gpui_kit::test]
fn picking_a_collection_environment_keeps_the_global(cx: &mut TestAppContext) {
    let col = with_envs();
    let s = start(cx, &col);
    pick(cx, &s, global_option_id("h"));
    pick(cx, &s, option_id("stage"));
    assert_eq!(collection_env(cx, &s).as_deref(), Some("stage"));
    assert_eq!(global_env(cx, &s).as_deref(), Some("h"));
    pick(cx, &s, option_id(NO_ENVIRONMENT));
    assert_eq!(collection_env(cx, &s), None);
    assert_eq!(global_env(cx, &s).as_deref(), Some("h"));
}

#[gpui_kit::test]
fn the_button_names_both_choices(cx: &mut TestAppContext) {
    let col = with_envs();
    let s = start(cx, &col);
    assert_eq!(label(cx, &s), NO_ENVIRONMENT);
    pick(cx, &s, option_id("stage"));
    assert_eq!(label(cx, &s), "stage");
    pick(cx, &s, global_option_id("g"));
    assert_eq!(label(cx, &s), "stage · g");
    pick(cx, &s, option_id(NO_ENVIRONMENT));
    assert_eq!(label(cx, &s), "No environment · g");
    let drawn = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.find("env-select").label().unwrap_or_default().to_owned()
        })
        .unwrap();
    assert!(drawn.contains("No environment · g"), "{drawn:?}");
}

#[gpui_kit::test]
fn the_open_menu_marks_both_choices(cx: &mut TestAppContext) {
    let col = with_envs();
    let s = start(cx, &col);
    pick(cx, &s, option_id("dev"));
    pick(cx, &s, global_option_id("h"));
    let picker = s.shell.read_with(cx, |v, _| v.env_picker().clone());
    let checked = picker.read_with(cx, |p, cx| p.checked(cx));
    assert_eq!(
        checked,
        (SharedString::from("dev"), SharedString::from("h"))
    );
}

#[gpui_kit::test]
fn arrow_keys_cross_from_the_collection_section_into_the_globals(cx: &mut TestAppContext) {
    let col = with_envs();
    let s = start(cx, &col);
    pick(cx, &s, option_id("dev"));
    press_key(cx, s.win, &key("alt-e"));
    assert!(
        present(cx, s.win, global_option_id("g")),
        "the menu is open"
    );
    // No environment, dev, stage, then over the Global header onto No global environment
    // and g.
    for _ in 0..5 {
        press_key(cx, s.win, "down");
    }
    press_key(cx, s.win, "enter");
    cx.run_until_parked();
    assert_eq!(global_env(cx, &s).as_deref(), Some("g"));
    assert_eq!(collection_env(cx, &s).as_deref(), Some("dev"));
    // Down onto No global environment, then one up over the header lands on the collection's
    // last environment.
    press_key(cx, s.win, &key("alt-e"));
    for _ in 0..4 {
        press_key(cx, s.win, "down");
    }
    press_key(cx, s.win, "up");
    press_key(cx, s.win, "enter");
    cx.run_until_parked();
    assert_eq!(collection_env(cx, &s).as_deref(), Some("stage"));
    assert_eq!(global_env(cx, &s).as_deref(), Some("g"));
}
