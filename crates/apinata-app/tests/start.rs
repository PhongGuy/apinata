use crate::support;

use apinata_app::app::{AppState, start, window_options};
use apinata_app::persist::{Paths, save_json};
use apinata_app::session::{SavedBounds, Session};
use apinata_app::settings::Settings;
use apinata_core::MemoryStore;
use gpui_kit::component::WindowExt;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Bounds, Pixels, TestAppContext, WindowBounds, point, px, size};
use std::sync::Arc;
use support::*;

fn b(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
    Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
}

#[gpui_kit::test]
fn start_restores_the_window_the_split_and_the_collections(cx: &mut TestAppContext) {
    support::init(cx);
    let files = tempfile::tempdir().unwrap();
    let paths = Paths::in_dir(files.path());
    let col = collection();
    let saved = WindowBounds::Windowed(b(100., 80., 900., 700.));
    let s = Session {
        open: vec![col.path().to_path_buf()],
        sidebar_width: Some(300.0),
        window: Some(SavedBounds::from_window(saved)),
        ..Session::default()
    };
    save_json(&paths.session, &s).unwrap();
    let (win, shell) = cx
        .update(|cx| start(&paths, Arc::new(MemoryStore::new()), Vec::new(), cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    assert_eq!(
        cx.update_window(win, |_, w, _| w.window_bounds()).unwrap(),
        saved
    );
    assert_eq!(
        shell.read_with(cx, |v, cx| v.workspace().read(cx).collections().len()),
        1
    );
    assert_eq!(
        shell.read_with(cx, |v, cx| v.split().read(cx).sizes()[0]),
        px(300.)
    );
    assert_eq!(
        cx.update_window(win, |_, w, cx| w.notifications(cx).len())
            .unwrap(),
        0
    );
}

/// Starts the app on a saved session whose workspaces are `dirs`, returning how many
/// notifications the window shows.
fn notices_after_restoring(cx: &mut TestAppContext, dirs: Vec<std::path::PathBuf>) -> usize {
    support::init(cx);
    let files = tempfile::tempdir().unwrap();
    let paths = Paths::in_dir(files.path());
    let s = Session {
        workspaces: dirs,
        ..Session::default()
    };
    save_json(&paths.session, &s).unwrap();
    let (win, _) = cx
        .update(|cx| start(&paths, Arc::new(MemoryStore::new()), Vec::new(), cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

#[gpui_kit::test]
fn a_workspace_folder_that_vanished_is_reported_on_start(cx: &mut TestAppContext) {
    let gone = tempfile::tempdir()
        .unwrap()
        .path()
        .join("deleted-workspace");
    assert_eq!(notices_after_restoring(cx, vec![gone]), 1);
}

#[gpui_kit::test]
fn a_malformed_workspace_file_is_reported_on_start(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("workspace.yml"), "- not\n- a mapping\n").unwrap();
    assert_eq!(
        notices_after_restoring(cx, vec![dir.path().to_path_buf()]),
        1
    );
}

#[gpui_kit::test]
fn a_corrupt_settings_file_is_set_aside_and_reported(cx: &mut TestAppContext) {
    support::init(cx);
    let files = tempfile::tempdir().unwrap();
    let paths = Paths::in_dir(files.path());
    std::fs::write(&paths.settings, "{not json").unwrap();
    let notes = vec!["The keychain is unavailable; secrets last until quit.".to_string()];
    let (win, _) = cx
        .update(|cx| start(&paths, Arc::new(MemoryStore::new()), notes, cx))
        .unwrap();
    cx.run_until_parked();
    assert!(files.path().join("settings.json.bad").exists());
    assert_eq!(
        cx.update_window(win, |_, w, cx| w.notifications(cx).len())
            .unwrap(),
        2
    );
    let settings = cx.update(|cx| cx.global::<AppState>().settings.read(cx).get().clone());
    assert_eq!(settings, Settings::default());
}

#[test]
fn window_options_use_saved_bounds_only_when_visible() {
    let screens = [b(0., 0., 1920., 1080.)];
    let mut s = Session::default();
    let opts = window_options(&s, &screens);
    assert_eq!(opts.window_bounds, None);
    assert_eq!(
        opts.titlebar.as_ref().and_then(|t| t.title.clone()),
        Some("Apinata".into())
    );
    assert_eq!(opts.app_id.as_deref(), Some("apinata-app"));
    s.window = Some(SavedBounds::from_window(WindowBounds::Windowed(b(
        100., 80., 900., 700.,
    ))));
    assert_eq!(
        window_options(&s, &screens).window_bounds,
        Some(WindowBounds::Windowed(b(100., 80., 900., 700.)))
    );
    s.window = Some(SavedBounds::from_window(WindowBounds::Windowed(b(
        5000., 5000., 900., 700.,
    ))));
    assert_eq!(window_options(&s, &screens).window_bounds, None);
}

#[gpui_kit::test]
fn quitting_writes_a_change_still_waiting_to_be_saved(cx: &mut TestAppContext) {
    support::init(cx);
    let files = tempfile::tempdir().unwrap();
    let paths = Paths::in_dir(files.path());
    cx.update(|cx| start(&paths, Arc::new(MemoryStore::new()), Vec::new(), cx))
        .unwrap();
    cx.run_until_parked();
    let session = cx.update(|cx| cx.global::<AppState>().session.clone());
    session.update(cx, |p, cx| p.update(cx, |s| s.sidebar_width = Some(321.0)));
    cx.quit();
    let saved: Session = apinata_app::persist::load_json(&paths.session).value;
    assert_eq!(saved.sidebar_width, Some(321.0));
}
