//! The Preview tabs preference: on, a single click in the sidebar previews and the next one
//! replaces it; off, every open keeps its tab.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::settings::Settings;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

fn preview_tabs(cx: &mut TestAppContext) -> bool {
    cx.update(|cx| cx.global::<AppState>().settings.read(cx).get().preview_tabs)
}

fn set_preview_tabs(cx: &mut TestAppContext, on: bool) {
    let settings = cx.update(|cx| cx.global::<AppState>().settings.clone());
    settings.update(cx, |p, cx| p.update(cx, |s| s.preview_tabs = on));
}

fn click_row(cx: &mut TestAppContext, s: &Started, path: &str) {
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click(row(path), cx))
        .unwrap();
    frame(cx, s.win);
}

// Each open tab as its request name and whether it is the preview.
fn tabs(cx: &mut TestAppContext, s: &Started) -> Vec<(String, bool)> {
    s.ws.read_with(cx, |w, _| {
        (w.tabs().entries().iter())
            .filter_map(|e| Some((e.item()?.as_str().to_owned(), e.preview)))
            .collect()
    })
}

#[test]
fn preview_tabs_are_on_by_default_also_for_older_settings_files() {
    assert!(Settings::default().preview_tabs);
    let old: Settings = serde_json::from_str(r#"{"autosave":true}"#).unwrap();
    assert!(old.preview_tabs);
    let off = Settings {
        preview_tabs: false,
        ..Settings::default()
    };
    let back: Settings = serde_json::from_str(&serde_json::to_string(&off).unwrap()).unwrap();
    assert!(!back.preview_tabs);
}

#[gpui_kit::test]
fn with_preview_tabs_on_a_second_click_replaces_the_preview(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    click_row(cx, &s, "a.yml");
    assert_eq!(tabs(cx, &s), [("a.yml".to_owned(), true)]);
    click_row(cx, &s, "b.yml");
    assert_eq!(tabs(cx, &s), [("b.yml".to_owned(), true)]);
}

#[gpui_kit::test]
fn with_preview_tabs_off_each_click_keeps_its_tab(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    set_preview_tabs(cx, false);
    click_row(cx, &s, "a.yml");
    assert_eq!(tabs(cx, &s), [("a.yml".to_owned(), false)]);
    click_row(cx, &s, "b.yml");
    assert_eq!(
        tabs(cx, &s),
        [("a.yml".to_owned(), false), ("b.yml".to_owned(), false)]
    );
}

#[gpui_kit::test]
fn the_preference_switches_preview_tabs(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        assert!(
            w.within("prefs-general")
                .try_find("pref-preview-tabs")
                .is_some()
        );
    })
    .unwrap();
    assert!(preview_tabs(cx));
    prefs_click(cx, &s, "preview-tabs");
    assert!(!preview_tabs(cx));
    prefs_click(cx, &s, "preview-tabs");
    assert!(preview_tabs(cx));
}
