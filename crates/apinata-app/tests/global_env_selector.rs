//! The title bar's global environment picker.

use crate::support;

use apinata_app::global_env_selector::{NO_GLOBAL_ENVIRONMENT, option_id};
use apinata_app::shell::Shell;
use apinata_app::workspace::Workspace;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, SharedString, TestAppContext, WindowOptions,
};
use support::*;

fn shell_with_globals(
    cx: &mut TestAppContext,
) -> (
    tempfile::TempDir,
    tempfile::TempDir,
    tempfile::TempDir,
    AnyWindowHandle,
    Entity<Workspace>,
    Entity<Shell>,
) {
    support::init(cx);
    let col = collection();
    let globals = write_tree(&[("g.yml", "name: g\n"), ("h.yml", "name: h\n")]);
    let (state, ws, _root) = open_dir(cx, &col);
    let dir = globals.path().to_path_buf();
    ws.update(cx, |w, cx| w.set_global_dir(dir, cx));
    cx.run_until_parked();
    let shared = ws.clone();
    let (win, shell) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| Shell::new(shared, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    (col, globals, state, win, ws, shell)
}

#[test]
fn option_ids_keep_names_apart_from_the_fixed_row() {
    assert_eq!(
        option_id(NO_GLOBAL_ENVIRONMENT),
        SharedString::from("global-env-none")
    );
    assert_eq!(
        option_id("none"),
        SharedString::from("global-env-option-none")
    );
}

#[gpui_kit::test]
fn the_picker_lists_and_chooses_global_environments(cx: &mut TestAppContext) {
    let (_c, _g, _s, win, ws, shell) = shell_with_globals(cx);
    let sel = shell.read_with(cx, |s, _| s.global_env_selector().clone());
    let items = sel.read_with(cx, |s, cx| s.items(cx));
    // Task 3 appends a manage row; the listed environments come first.
    assert_eq!(
        items[..3],
        [
            SharedString::from(NO_GLOBAL_ENVIRONMENT),
            "g".into(),
            "h".into()
        ]
    );
    assert_eq!(
        sel.read_with(cx, |s, _| s.shown()),
        Some(NO_GLOBAL_ENVIRONMENT.into())
    );
    cx.update_window(win, |_, w, cx| sel.update(cx, |s, cx| s.choose("h", w, cx)))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        ws.read_with(cx, |w, _| w.selected_global_environment())
            .as_deref(),
        Some("h")
    );
    assert_eq!(sel.read_with(cx, |s, _| s.shown()), Some("h".into()));
    cx.update_window(win, |_, w, cx| {
        sel.update(cx, |s, cx| s.choose(NO_GLOBAL_ENVIRONMENT, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        ws.read_with(cx, |w, _| w.selected_global_environment()),
        None
    );
}

#[gpui_kit::test]
fn refresh_shows_a_file_added_outside_the_app(cx: &mut TestAppContext) {
    let (_c, globals, _s, _win, _ws, shell) = shell_with_globals(cx);
    std::fs::write(globals.path().join("k.yml"), "name: k\n").unwrap();
    let sel = shell.read_with(cx, |s, _| s.global_env_selector().clone());
    sel.update(cx, |s, cx| s.refresh(cx));
    cx.run_until_parked();
    let items = sel.read_with(cx, |s, cx| s.items(cx));
    assert!(items.contains(&SharedString::from("k")), "{items:?}");
}

#[gpui_kit::test]
fn the_picker_sits_in_the_title_bar(cx: &mut TestAppContext) {
    let (_c, _g, _s, win, _ws, _shell) = shell_with_globals(cx);
    assert!(present(cx, win, "global-env-selector"));
}

#[test]
fn the_empty_global_row_reads_no_globals() {
    assert_eq!(NO_GLOBAL_ENVIRONMENT, "No globals");
    assert_eq!(
        apinata_app::global_env_selector::manage_global_environments(),
        "Manage globals…"
    );
}
