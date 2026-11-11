//! The Light theme and Dark theme selects in Preferences.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::settings::{Settings, ThemeChoice};
use apinata_app::theme::{self, DARK, HC_DARK, HC_LIGHT, LIGHT};
use gpui_kit::component::{Colorize as _, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Hsla, TestAppContext};
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

/// A user theme file "Zed Night" (dark) in a scratch dir, loaded into the registry.
fn load_user_theme(cx: &mut TestAppContext) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let mut set: serde_json::Value = serde_json::from_str(theme::THEME_JSON).unwrap();
    let mut mine = set["themes"][0].take();
    mine["name"] = serde_json::json!("Zed Night");
    let text = serde_json::json!({ "name": "Zed Night", "themes": [mine] }).to_string();
    std::fs::write(tmp.path().join("mine.json"), text).unwrap();
    assert!(
        cx.update(|cx| theme::load_user_themes(tmp.path(), cx))
            .is_empty()
    );
    tmp
}

fn names(cx: &mut TestAppContext, mode: ThemeMode) -> Vec<String> {
    cx.update(|cx| {
        theme::choices(mode, cx)
            .iter()
            .map(|n| n.to_string())
            .collect()
    })
}

/// Opens the select `id`, returns its menu's item labels, and leaves the menu open.
fn open_select(cx: &mut TestAppContext, s: &Started, id: &str, count: usize) -> Vec<String> {
    prefs_reveal(cx, s, id);
    let id = id.to_string();
    cx.update_window(s.win, |_, w, cx| w.click(id, cx)).unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        let menu = w.within("popup-menu");
        (0..count)
            .map(|ix| menu.find(ix).label().unwrap_or_default().to_string())
            .collect()
    })
    .unwrap()
}

fn pick(cx: &mut TestAppContext, s: &Started, id: &str, ix: usize, count: usize) {
    open_select(cx, s, id, count);
    cx.update_window(s.win, |_, w, cx| w.within("popup-menu").click(ix, cx))
        .unwrap();
    frame(cx, s.win);
}

fn shown(cx: &mut TestAppContext, s: &Started, id: &str) -> String {
    prefs_reveal(cx, s, id);
    let id = id.to_string();
    cx.update_window(s.win, |_, w, _| {
        w.find(format!("{id}-value"))
            .label()
            .unwrap_or_default()
            .to_string()
    })
    .unwrap()
}

#[gpui_kit::test]
fn both_rows_sit_directly_under_the_theme_row(cx: &mut TestAppContext) {
    let s = started(cx, &two_requests());
    open_prefs(cx, &s);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let top = |id: &'static str| w.find(id).bounds().top();
        assert!(top("pref-theme") < top("pref-light-theme"));
        assert!(top("pref-light-theme") < top("pref-dark-theme"));
        assert!(top("pref-dark-theme") < top("pref-orientation"));
        assert!(w.try_find("light-theme").is_some() && w.try_find("dark-theme").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_dark_options_include_a_user_theme_and_the_high_contrast_pair(cx: &mut TestAppContext) {
    let s = started(cx, &two_requests());
    let _dir = load_user_theme(cx);
    open_prefs(cx, &s);
    let dark = names(cx, ThemeMode::Dark);
    assert_eq!(dark, [DARK, HC_DARK, "Zed Night"]);
    assert_eq!(open_select(cx, &s, "dark-theme", dark.len()), dark);
    assert_eq!(shown(cx, &s, "dark-theme"), DARK);
    let light = names(cx, ThemeMode::Light);
    assert_eq!(light, [LIGHT, HC_LIGHT]);
}

#[gpui_kit::test]
fn picking_a_theme_persists_and_applies_live_and_the_default_stores_none(cx: &mut TestAppContext) {
    let s = started(cx, &two_requests());
    set(cx, |s| s.theme = ThemeChoice::Dark);
    open_prefs(cx, &s);
    let before = cx.update(|cx| Theme::global(cx).background);
    pick(cx, &s, "dark-theme", 1, 2);
    assert_eq!(settings(cx).dark_theme.as_deref(), Some(HC_DARK));
    assert_eq!(settings(cx).light_theme, None);
    let after = cx.update(|cx| Theme::global(cx).background);
    assert_ne!(before, after);
    assert_eq!(after, Hsla::parse_hex("#000000").unwrap());
    assert_eq!(shown(cx, &s, "dark-theme"), HC_DARK);
    pick(cx, &s, "dark-theme", 0, 2);
    assert_eq!(settings(cx).dark_theme, None);
    assert_eq!(cx.update(|cx| Theme::global(cx).background), before);
    pick(cx, &s, "light-theme", 1, 2);
    assert_eq!(settings(cx).light_theme.as_deref(), Some(HC_LIGHT));
}

#[gpui_kit::test]
fn a_saved_name_that_is_not_offered_shows_the_default(cx: &mut TestAppContext) {
    let s = started(cx, &two_requests());
    set(cx, |s| {
        s.dark_theme = Some("No Such Theme".to_string());
        s.light_theme = Some(HC_DARK.to_string());
    });
    open_prefs(cx, &s);
    assert_eq!(shown(cx, &s, "dark-theme"), DARK);
    assert_eq!(shown(cx, &s, "light-theme"), LIGHT);
}

#[gpui_kit::test]
fn search_finds_the_rows(cx: &mut TestAppContext) {
    let s = started(cx, &two_requests());
    open_prefs(cx, &s);
    replace_input(cx, s.win, None, "prefs-search", "dark theme");
    frame(cx, s.win);
    assert!(present(cx, s.win, "pref-dark-theme"));
    assert!(!present(cx, s.win, "pref-light-theme"));
    replace_input(cx, s.win, None, "prefs-search", "light theme");
    frame(cx, s.win);
    assert!(present(cx, s.win, "pref-light-theme"));
}

#[gpui_kit::test]
fn the_button_is_named_for_assistive_tools(cx: &mut TestAppContext) {
    let s = started(cx, &two_requests());
    open_prefs(cx, &s);
    prefs_reveal(cx, &s, "dark-theme");
    let label = cx
        .update_window(s.win, |_, w, _| {
            w.find("dark-theme").label().map(str::to_owned)
        })
        .unwrap();
    assert_eq!(label.as_deref(), Some("Dark theme: Apinata Dark"));
}

#[gpui_kit::test]
fn enter_on_the_focused_select_opens_its_menu(cx: &mut TestAppContext) {
    let s = started(cx, &two_requests());
    open_prefs(cx, &s);
    tab_to(cx, s.win, "light-theme");
    cx.update_window(s.win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    frame(cx, s.win);
    let items = cx
        .update_window(s.win, |_, w, _| {
            w.within("popup-menu").try_find(1usize).is_some()
        })
        .unwrap();
    assert!(items, "the menu lists the themes");
}

#[gpui_kit::test]
fn a_very_long_theme_name_stays_inside_the_dialog(cx: &mut TestAppContext) {
    let s = started(cx, &two_requests());
    let tmp = tempfile::tempdir().unwrap();
    let long = "W".repeat(80);
    let mut file: serde_json::Value = serde_json::from_str(theme::THEME_JSON).unwrap();
    let mut mine = file["themes"][0].take();
    mine["name"] = serde_json::json!(long);
    let text = serde_json::json!({ "name": long, "themes": [mine] }).to_string();
    std::fs::write(tmp.path().join("long.json"), text).unwrap();
    assert!(
        cx.update(|cx| theme::load_user_themes(tmp.path(), cx))
            .is_empty()
    );
    set(cx, |s| s.dark_theme = Some(long.clone()));
    open_prefs(cx, &s);
    prefs_reveal(cx, &s, "dark-theme");
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let button = w.find("dark-theme").bounds();
        let rows = w.find("preferences-rows").bounds();
        assert!(button.right() <= rows.right(), "{button:?} in {rows:?}");
        assert!(button.left() >= rows.left());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_opened_menu_starts_on_the_current_theme_so_one_down_moves_past_it(cx: &mut TestAppContext) {
    let s = started(cx, &two_requests());
    open_prefs(cx, &s);
    open_select(cx, &s, "dark-theme", 2);
    press_key(cx, s.win, "down");
    press_key(cx, s.win, "enter");
    frame(cx, s.win);
    assert_eq!(settings(cx).dark_theme.as_deref(), Some(HC_DARK));
}

#[gpui_kit::test]
fn one_down_from_a_later_current_theme_picks_the_one_after_it(cx: &mut TestAppContext) {
    let s = started(cx, &two_requests());
    let _dir = load_user_theme(cx);
    set(cx, |s| s.dark_theme = Some(HC_DARK.to_string()));
    open_prefs(cx, &s);
    assert_eq!(names(cx, ThemeMode::Dark), [DARK, HC_DARK, "Zed Night"]);
    open_select(cx, &s, "dark-theme", 3);
    press_key(cx, s.win, "down");
    press_key(cx, s.win, "enter");
    frame(cx, s.win);
    assert_eq!(settings(cx).dark_theme.as_deref(), Some("Zed Night"));
}

// Each theme picker shows a chevron at its right end, so it reads as a menu, not a text field.
#[gpui_kit::test]
fn each_theme_picker_shows_a_chevron_after_its_value(cx: &mut TestAppContext) {
    use apinata_app::style::{SELECT_CHEVRON, at_zoom};
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    let side = at_zoom(SELECT_CHEVRON, 1.0);
    for id in ["light-theme", "dark-theme"] {
        prefs_reveal(cx, &s, id);
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            let select = w.find(id).bounds();
            let value = w.find(format!("{id}-value")).bounds();
            let chevron = w.find(format!("{id}-chevron"));
            let at = chevron.bounds();
            assert!(chevron.visible(), "{id}: the chevron is drawn");
            assert!(
                (at.size.width - side).abs() < gpui_kit::px(0.5),
                "{id}: {at:?}"
            );
            assert!(
                at.left() >= value.right() && at.right() <= select.right(),
                "{id}: {at:?}"
            );
        })
        .unwrap();
    }
}
