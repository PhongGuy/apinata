use crate::support;

use apinata_app::app::AppState;
use apinata_app::persist::{Paths, save_json};
use apinata_app::settings::{Orientation, Settings, ThemeChoice};
use apinata_app::theme::mode_for;
use apinata_core::MemoryStore;
use gpui_kit::component::{Theme, ThemeMode, WindowExt};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext, WindowAppearance, px};
use std::sync::Arc;
use support::*;

fn settings(cx: &mut TestAppContext) -> Settings {
    cx.update(|cx| cx.global::<AppState>().settings.read(cx).get().clone())
}

fn set(cx: &mut TestAppContext, f: impl FnOnce(&mut Settings)) {
    cx.update(|cx| {
        let s = cx.global::<AppState>().settings.clone();
        s.update(cx, |p, cx| p.update(cx, f));
    });
    cx.run_until_parked();
}

fn mode(cx: &mut TestAppContext) -> ThemeMode {
    cx.update(|cx| Theme::global(cx).mode)
}

fn enter(cx: &mut TestAppContext, s: &Started, id: &'static str, text: &str) {
    replace_input(cx, s.win, None, id, text);
    cx.update_window(s.win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
}

#[test]
fn the_mode_follows_the_choice_then_the_system() {
    use WindowAppearance as A;
    assert_eq!(mode_for(ThemeChoice::System, A::Dark), ThemeMode::Dark);
    assert_eq!(
        mode_for(ThemeChoice::System, A::VibrantDark),
        ThemeMode::Dark
    );
    assert_eq!(
        mode_for(ThemeChoice::System, A::VibrantLight),
        ThemeMode::Light
    );
    assert_eq!(mode_for(ThemeChoice::Light, A::Dark), ThemeMode::Light);
    assert_eq!(mode_for(ThemeChoice::Dark, A::Light), ThemeMode::Dark);
}

#[gpui_kit::test]
fn start_applies_the_saved_theme_and_zoom(cx: &mut TestAppContext) {
    support::init(cx);
    let files = tempfile::tempdir().unwrap();
    let paths = Paths::in_dir(files.path());
    let saved = Settings {
        theme: ThemeChoice::Dark,
        zoom: 2.0,
        ..Settings::default()
    };
    save_json(&paths.settings, &saved).unwrap();
    cx.update(|cx| apinata_app::app::start(&paths, Arc::new(MemoryStore::new()), Vec::new(), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(mode(cx), ThemeMode::Dark);
    assert_eq!(cx.update(|cx| Theme::global(cx).mono_font_size), px(22.75));
}

#[gpui_kit::test]
fn a_settings_change_applies_at_once(cx: &mut TestAppContext) {
    let col = two_requests();
    let _s = started(cx, &col);
    assert_eq!(
        mode(cx),
        ThemeMode::Light,
        "System follows the test window, which is light"
    );
    set(cx, |s| s.theme = ThemeChoice::Dark);
    assert_eq!(mode(cx), ThemeMode::Dark);
    set(cx, |s| s.theme = ThemeChoice::System);
    assert_eq!(mode(cx), ThemeMode::Light);
}

#[gpui_kit::test]
fn the_preferences_dialog_edits_the_settings(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap();
    frame(cx, s.win);
    assert!(
        cx.update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
            .unwrap()
    );
    assert!(present(cx, s.win, "pref-verify-tls-restart"));
    cx.update_window(s.win, |_, w, cx| {
        w.within("theme").click(2usize, cx);
        w.within("orientation").click(1usize, cx);
        w.click("verify-tls", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let got = settings(cx);
    assert_eq!(
        (got.theme, got.response_orientation, got.insecure),
        (ThemeChoice::Dark, Orientation::Horizontal, true)
    );
    assert_eq!(mode(cx), ThemeMode::Dark);
    enter(cx, &s, "body-cap", "5");
    assert_eq!(settings(cx).max_body_bytes, Some(5 * 1024 * 1024));
    enter(cx, &s, "body-cap", "");
    assert_eq!(settings(cx).max_body_bytes, None);
    enter(cx, &s, "body-cap", "abc");
    assert!(present(cx, s.win, "body-cap-error"));
    assert_eq!(settings(cx).max_body_bytes, None);
}

#[gpui_kit::test]
fn a_failed_settings_save_is_a_notification(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    // A directory where the settings file belongs makes the write fail.
    let path = Paths::in_dir(s.files.path()).settings;
    let _ = std::fs::remove_file(&path);
    std::fs::create_dir_all(path.join("blocker")).unwrap();
    set(cx, |s| s.theme = ThemeChoice::Dark);
    let task = cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| p.flush(cx))
    });
    cx.run_until_parked();
    drop(task);
    let shown = cx
        .update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap();
    assert_eq!(shown, 1);
}
