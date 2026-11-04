//! `theme::apply` installs the theme each mode's setting names, falling back to the bundled pair.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::focus_mode::{self, Modality};
use apinata_app::settings::{Settings, ThemeChoice};
use apinata_app::theme::{self, DARK, HC_DARK, HC_LIGHT, LIGHT};
use gpui_kit::component::{Colorize as _, Theme, ThemeMode};
use gpui_kit::{AppContext as _, Hsla, TestAppContext};
use support::*;

fn set(cx: &mut TestAppContext, f: impl FnOnce(&mut Settings)) {
    cx.update(|cx| {
        let s = cx.global::<AppState>().settings.clone();
        s.update(cx, |p, cx| p.update(cx, f));
    });
    cx.run_until_parked();
}

fn is(c: Hsla, hex: &str) -> bool {
    let (a, b) = (c.to_rgb(), Hsla::parse_hex(hex).unwrap().to_rgb());
    [(a.r, b.r), (a.g, b.g), (a.b, b.b)]
        .iter()
        .all(|(x, y)| (x - y).abs() < 1.5 / 255.0)
}

fn background(cx: &mut TestAppContext) -> Hsla {
    cx.update(|cx| Theme::global(cx).background)
}

#[gpui_kit::test]
fn the_named_dark_theme_is_applied_in_dark_mode(cx: &mut TestAppContext) {
    let _s = started(cx, &two_requests());
    set(cx, |s| {
        s.theme = ThemeChoice::Dark;
        s.dark_theme = Some(HC_DARK.to_string());
    });
    assert!(is(background(cx), "#000000"));
    set(cx, |s| s.dark_theme = None);
    assert!(is(background(cx), "#131218"), "back to the default");
}

#[gpui_kit::test]
fn the_light_slot_follows_a_later_mode_switch(cx: &mut TestAppContext) {
    let _s = started(cx, &two_requests());
    set(cx, |s| {
        s.theme = ThemeChoice::Dark;
        s.light_theme = Some(HC_LIGHT.to_string());
    });
    set(cx, |s| s.theme = ThemeChoice::Light);
    assert!(is(background(cx), "#FFFFFF"));
    set(cx, |s| s.light_theme = None);
    assert_eq!(
        cx.update(|cx| Theme::global(cx).light_theme.name.to_string()),
        LIGHT
    );
    assert!(is(background(cx), "#FBFAFC"));
}

#[gpui_kit::test]
fn an_unknown_or_wrong_mode_name_falls_back(cx: &mut TestAppContext) {
    let _s = started(cx, &two_requests());
    set(cx, |s| {
        s.theme = ThemeChoice::Dark;
        s.dark_theme = Some("No Such Theme".to_string());
        s.light_theme = Some(HC_DARK.to_string());
    });
    let names = cx.update(|cx| {
        let t = Theme::global(cx);
        (
            t.dark_theme.name.to_string(),
            t.light_theme.name.to_string(),
        )
    });
    assert_eq!(names, (DARK.to_string(), LIGHT.to_string()));
    set(cx, |s| s.dark_theme = Some(HC_LIGHT.to_string()));
    assert!(is(background(cx), "#131218"));
}

#[gpui_kit::test]
fn applying_the_same_theme_twice_does_not_reload(cx: &mut TestAppContext) {
    let s = started(cx, &two_requests());
    set(cx, |s| {
        s.theme = ThemeChoice::Dark;
        s.dark_theme = Some(HC_DARK.to_string());
    });
    // A reload would overwrite this sentinel with the theme's own background.
    let sentinel = Hsla::parse_hex("#123456").unwrap();
    cx.update(|cx| Theme::update(cx, |t| t.background = sentinel));
    cx.update_window(s.win, |_, w, cx| {
        let current = cx.global::<AppState>().settings.read(cx).get().clone();
        theme::apply(&current, w, cx);
    })
    .unwrap();
    assert_eq!(background(cx), sentinel);
    // A different name does reload.
    set(cx, |s| s.dark_theme = None);
    assert!(is(background(cx), "#131218"));
}

#[gpui_kit::test]
fn the_ring_and_selected_fill_survive_a_theme_change(cx: &mut TestAppContext) {
    let _s = started(cx, &two_requests());
    cx.update(|cx| focus_mode::set(Modality::Keyboard, cx));
    set(cx, |s| {
        s.theme = ThemeChoice::Dark;
        s.dark_theme = Some(HC_DARK.to_string());
    });
    cx.update(|cx| {
        let t = Theme::global(cx);
        assert!(is(t.ring, "#FFFF00") && t.focus_ring);
        assert!(is(t.list_active, "#2E2E2E") && t.list_active.a == 1.0);
        assert!(is(t.tokens.list_active.color, "#2E2E2E"));
    });
}

#[gpui_kit::test]
fn choices_list_a_modes_themes_defaults_first(cx: &mut TestAppContext) {
    support::init(cx);
    let tmp = tempfile::tempdir().unwrap();
    let mut set: serde_json::Value = serde_json::from_str(theme::THEME_JSON).unwrap();
    let mut mine = set["themes"][0].take();
    mine["name"] = serde_json::json!("Zed Night");
    let text = serde_json::json!({ "name": "Zed Night", "themes": [mine] }).to_string();
    std::fs::write(tmp.path().join("mine.json"), text).unwrap();
    cx.update(|cx| {
        assert!(theme::load_user_themes(tmp.path(), cx).is_empty());
        let names = |mode| -> Vec<String> {
            theme::choices(mode, cx)
                .iter()
                .map(|n| n.to_string())
                .collect()
        };
        // The kit's own built-in themes are not Apinata's to offer.
        assert_eq!(names(ThemeMode::Dark), [DARK, HC_DARK, "Zed Night"]);
        assert_eq!(names(ThemeMode::Light), [LIGHT, HC_LIGHT]);
    });
}
