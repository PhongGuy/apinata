//! The store and send cookie preferences: their defaults, startup, and live toggling.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::executor::{exec_options, shared};
use apinata_app::settings::Settings;
use apinata_core::Executor;
use gpui_kit::TestAppContext;
use support::*;

fn app_settings(cx: &mut TestAppContext) -> Settings {
    cx.update(|cx| cx.global::<AppState>().settings.read(cx).get().clone())
}

/// (store, send) as the shared executor holds them.
fn exec_flags(cx: &mut TestAppContext) -> (bool, bool) {
    cx.update(|cx| {
        let exec = shared(cx).expect("installed");
        (exec.store_cookies_enabled(), exec.send_cookies_enabled())
    })
}

#[test]
fn both_default_on_also_for_older_settings_files() {
    let d = Settings::default();
    assert!(d.store_cookies && d.send_cookies);
    let old: Settings = serde_json::from_str(r#"{"insecure":false,"history_limit":5}"#).unwrap();
    assert!(old.store_cookies && old.send_cookies);
    assert_eq!(old.history_limit, 5);
}

#[test]
fn exec_options_carries_both_switches() {
    let mut s = Settings::default();
    let e = Executor::new(exec_options(&s)).unwrap();
    assert!(e.store_cookies_enabled() && e.send_cookies_enabled());
    s.store_cookies = false;
    let e = Executor::new(exec_options(&s)).unwrap();
    assert!(!e.store_cookies_enabled() && e.send_cookies_enabled());
    s.store_cookies = true;
    s.send_cookies = false;
    let e = Executor::new(exec_options(&s)).unwrap();
    assert!(e.store_cookies_enabled() && !e.send_cookies_enabled());
}

#[gpui_kit::test]
fn saved_cookie_settings_reach_the_executor_at_start(cx: &mut TestAppContext) {
    let col = two_requests();
    let _s = started_with(cx, &col, |paths| {
        let mut s = test_settings();
        s.store_cookies = false;
        s.send_cookies = false;
        apinata_app::persist::save_json(&paths.settings, &s).unwrap();
    });
    assert_eq!(exec_flags(cx), (false, false));
}

#[gpui_kit::test]
fn the_checkboxes_save_and_apply_at_once(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    assert_eq!(exec_flags(cx), (true, true));
    open_prefs(cx, &s);
    assert!(present(cx, s.win, "prefs-cookies-note"));
    prefs_click(cx, &s, "store-cookies");
    cx.run_until_parked();
    assert!(!app_settings(cx).store_cookies);
    assert_eq!(exec_flags(cx), (false, true));
    prefs_click(cx, &s, "send-cookies");
    cx.run_until_parked();
    assert!(!app_settings(cx).send_cookies);
    assert_eq!(exec_flags(cx), (false, false));
    prefs_click(cx, &s, "store-cookies");
    cx.run_until_parked();
    assert!(app_settings(cx).store_cookies);
    assert_eq!(exec_flags(cx), (true, false));
}
