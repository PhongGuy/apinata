//! The General section and its autosave switch.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::settings::Settings;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

fn autosave(cx: &mut TestAppContext) -> bool {
    cx.update(|cx| cx.global::<AppState>().settings.read(cx).get().autosave)
}

#[test]
fn autosave_is_off_by_default_also_for_older_settings_files() {
    assert!(!Settings::default().autosave);
    let old: Settings = serde_json::from_str(r#"{"remember_responses":true}"#).unwrap();
    assert!(!old.autosave);
}

#[gpui_kit::test]
fn general_comes_first_and_switches_autosave(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let general = w.find("prefs-general").bounds();
        let appearance = w.find("prefs-appearance").bounds();
        assert!(general.top() < appearance.top());
        assert!(
            w.within("prefs-general")
                .try_find("pref-autosave")
                .is_some()
        );
    })
    .unwrap();
    assert!(!autosave(cx));
    prefs_click(cx, &s, "autosave");
    assert!(autosave(cx));
    prefs_click(cx, &s, "autosave");
    assert!(!autosave(cx));
}
