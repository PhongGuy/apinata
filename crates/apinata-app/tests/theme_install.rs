//! `theme::install` makes the Apinata themes the kit's light and dark themes.

use crate::support;

use apinata_app::style::{SPACE_3, STATUS_BAR_HEIGHT, at_zoom};
use apinata_app::theme::{self, DARK, HC_DARK, HC_LIGHT, LIGHT};
use gpui_kit::component::{Colorize as _, Theme, ThemeMode, ThemeRegistry};
use gpui_kit::{Anchor, Hsla, TestAppContext, px};

fn is(c: Hsla, hex: &str) -> bool {
    let (a, b) = (c.to_rgb(), Hsla::parse_hex(hex).unwrap().to_rgb());
    [(a.r, b.r), (a.g, b.g), (a.b, b.b)]
        .iter()
        .all(|(x, y)| (x - y).abs() < 1.5 / 255.0)
}

/// The colour is `hex` and fully opaque: the kit clamps some tokens' alpha on load.
fn is_opaque(c: Hsla, hex: &str) -> bool {
    is(c, hex) && c.a == 1.0
}

#[gpui_kit::test]
fn init_installs_the_apinata_themes(cx: &mut TestAppContext) {
    support::init(cx);
    cx.update(|cx| {
        let names = ThemeRegistry::global(cx).themes();
        assert!(names.keys().any(|n| n.as_ref() == DARK));
        assert!(names.keys().any(|n| n.as_ref() == LIGHT));
        let t = Theme::global(cx);
        assert_eq!(t.dark_theme.name.as_ref(), DARK);
        assert_eq!(t.light_theme.name.as_ref(), LIGHT);
        assert_eq!(t.font_family.as_ref(), "Inter");
        assert_eq!(t.mono_font_family.as_ref(), "JetBrains Mono");
        assert_eq!(t.font_size, px(13.));
        assert_eq!(t.radius, px(4.));
    });
}

#[gpui_kit::test]
fn both_modes_use_the_apinata_themes(cx: &mut TestAppContext) {
    support::init(cx);
    cx.update(|cx| {
        Theme::change(ThemeMode::Dark, None, cx);
        assert!(is(Theme::global(cx).primary, "#FF922B"));
        assert!(is(Theme::global(cx).background, "#131218"));
        Theme::change(ThemeMode::Light, None, cx);
        assert!(is(Theme::global(cx).primary, "#C2410C"));
        assert!(is(Theme::global(cx).background, "#FBFAFC"));
        Theme::change(ThemeMode::Dark, None, cx);
        assert!(is(Theme::global(cx).title_bar, "#1A1921"));
    });
}

#[gpui_kit::test]
fn install_twice_is_harmless(cx: &mut TestAppContext) {
    support::init(cx);
    cx.update(|cx| {
        theme::install(cx);
        assert_eq!(Theme::global(cx).dark_theme.name.as_ref(), DARK);
        Theme::change(ThemeMode::Light, None, cx);
        assert!(is(Theme::global(cx).foreground, "#1B1530"));
    });
}

#[gpui_kit::test]
fn the_selected_fill_is_installed_opaque_in_both_modes(cx: &mut TestAppContext) {
    support::init(cx);
    cx.update(|cx| {
        let fill = |cx: &gpui_kit::App| {
            let t = Theme::global(cx);
            (t.list_active, t.tokens.list_active.color)
        };
        theme::set_mode(ThemeMode::Dark, None, cx);
        let (field, token) = fill(cx);
        assert!(
            is_opaque(field, "#33303F") && is_opaque(token, "#33303F"),
            "dark"
        );
        theme::set_mode(ThemeMode::Light, None, cx);
        let (field, token) = fill(cx);
        assert!(
            is_opaque(field, "#E2DEE9") && is_opaque(token, "#E2DEE9"),
            "light"
        );
        theme::set_mode(ThemeMode::Dark, None, cx);
        let (field, token) = fill(cx);
        assert!(
            is_opaque(field, "#33303F") && is_opaque(token, "#33303F"),
            "back to dark"
        );
    });
}

#[gpui_kit::test]
fn toasts_sit_above_the_status_bar_in_both_modes(cx: &mut TestAppContext) {
    support::init(cx);
    cx.update(|cx| {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            Theme::change(mode, None, cx);
            let n = &Theme::global(cx).notification;
            assert_eq!(n.placement, Anchor::BottomRight, "{mode:?}");
            assert_eq!(
                n.margins.bottom,
                at_zoom(STATUS_BAR_HEIGHT, 1.0) + at_zoom(SPACE_3, 1.0),
                "{mode:?}"
            );
        }
    });
}

#[gpui_kit::test]
fn the_high_contrast_themes_are_registered_and_the_defaults_stay(cx: &mut TestAppContext) {
    support::init(cx);
    cx.update(|cx| {
        theme::install(cx);
        let names = ThemeRegistry::global(cx).themes();
        for want in [HC_LIGHT, HC_DARK] {
            assert!(names.keys().any(|n| n.as_ref() == want), "{want}");
        }
        let t = Theme::global(cx);
        assert_eq!(t.dark_theme.name.as_ref(), DARK);
        assert_eq!(t.light_theme.name.as_ref(), LIGHT);
    });
}
