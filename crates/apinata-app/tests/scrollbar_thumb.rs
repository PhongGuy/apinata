//! The scrollbar thumb, resting and hovered, is 3:1 against the pane in all four themes.

use crate::support;

use apinata_app::theme;
use gpui_kit::component::{ActiveTheme as _, Colorize as _};
use gpui_kit::{Hsla, TestAppContext};
use support::*;

const THEMES: [&str; 4] = [theme::DARK, theme::LIGHT, theme::HC_DARK, theme::HC_LIGHT];

#[gpui_kit::test]
fn the_scrollbar_thumb_is_three_to_one_on_the_page(cx: &mut TestAppContext) {
    let mut failures = Vec::new();
    for name in THEMES {
        use_theme(cx, name);
        cx.update(|cx| {
            let t = cx.theme();
            // Read the theme file rather than the live token, as the ring test does.
            let colors = if t.mode.is_dark() {
                &t.dark_theme.colors
            } else {
                &t.light_theme.colors
            };
            let hex = |s: &Option<_>| -> Hsla {
                let s: &gpui_kit::SharedString = s.as_ref().unwrap();
                Hsla::parse_hex(s).unwrap()
            };
            for (what, c) in [
                ("rest", hex(&colors.scrollbar_thumb)),
                ("hover", hex(&colors.scrollbar_thumb_hover)),
            ] {
                let r = contrast(c, t.background);
                eprintln!("{name} {what}: {r:.2}");
                if r < 3.0 {
                    failures.push(format!("{name} {what}: {r:.2}"));
                }
            }
        });
    }
    assert!(failures.is_empty(), "thumb below 3:1: {failures:?}");
}
