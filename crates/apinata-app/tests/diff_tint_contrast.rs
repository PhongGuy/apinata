//! The added and removed diff row tints form a quiet wash on the page and keep the row text
//! readable (4.5:1) in all four themes; the +/- gutter mark still carries the meaning.

use crate::support::*;

use apinata_app::compare_rows::ComparePalette;
use apinata_app::diff_doc::RowKind;
use apinata_app::diff_rows::DiffPalette;
use apinata_app::theme::{self, Status, diff_fill};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{AppContext as _, TestAppContext};

const THEMES: [&str; 4] = [theme::DARK, theme::LIGHT, theme::HC_DARK, theme::HC_LIGHT];

#[gpui_kit::test]
fn the_tints_are_a_quiet_wash_with_readable_text_numbers_and_marks(cx: &mut TestAppContext) {
    let col = collection_for("http://127.0.0.1:1", "");
    let s = started(cx, &col);
    frame(cx, s.win);
    for name in THEMES {
        use_theme(cx, name);
        cx.update_window(s.win, |_, w, cx| {
            let (page, text) = (cx.theme().background, cx.theme().foreground);
            let d = DiffPalette::new(&[], w, cx);
            for (kind, fill) in [(RowKind::Added, d.added()), (RowKind::Removed, d.removed())] {
                let wash = contrast(fill, page);
                assert!((1.3..=1.6).contains(&wash), "{name} {kind:?}: wash {wash}");
                assert!(contrast(text, fill) >= 4.5, "{name} {kind:?}: text");
                let n = d.number_color(kind);
                assert!(contrast(n, fill) >= 4.5, "{name} {kind:?}: numbers");
                let g = d.glyph_color(kind);
                assert!(contrast(g, fill) >= 4.5, "{name} {kind:?}: glyph");
                let c = ComparePalette::new(&[], w, cx);
                let g = c.glyph_color(kind);
                assert!(contrast(g, fill) >= 4.5, "{name} {kind:?}: compare glyph");
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn the_glyph_takes_the_accent_of_the_change(cx: &mut TestAppContext) {
    let col = collection_for("http://127.0.0.1:1", "");
    let s = started(cx, &col);
    frame(cx, s.win);
    for name in THEMES {
        use_theme(cx, name);
        cx.update_window(s.win, |_, w, cx| {
            let muted = cx.theme().muted_foreground;
            let d = DiffPalette::new(&[], w, cx);
            let c = ComparePalette::new(&[], w, cx);
            // The diff token (the theme's, bar High Contrast Dark's clearer red) when it clears
            // AA on its wash, else the same hue darkened or lightened.
            for (kind, token, status) in [
                (
                    RowKind::Added,
                    theme::diff_token(Status::Success, cx),
                    Status::Success,
                ),
                (
                    RowKind::Removed,
                    theme::diff_token(Status::Danger, cx),
                    Status::Danger,
                ),
            ] {
                let wash = diff_fill(status, cx);
                for g in [d.glyph_color(kind), c.glyph_color(kind)] {
                    if contrast(token, wash) >= 4.5 {
                        assert_eq!(g, token, "{name} {kind:?}");
                    } else {
                        assert!(hue_gap(g.h, token.h) < 0.02, "{name} {kind:?}: hue");
                    }
                }
            }
            assert_eq!(d.glyph_color(RowKind::Context), muted, "{name}");
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn the_diff_and_compare_rows_use_the_diff_fills(cx: &mut TestAppContext) {
    let col = collection_for("http://127.0.0.1:1", "");
    let s = started(cx, &col);
    frame(cx, s.win);
    for name in THEMES {
        use_theme(cx, name);
        cx.update_window(s.win, |_, w, cx| {
            let (added, removed) = (
                diff_fill(Status::Success, cx),
                diff_fill(Status::Danger, cx),
            );
            let d = DiffPalette::new(&[], w, cx);
            assert_eq!((d.added(), d.removed()), (added, removed), "{name}: diff");
            let c = ComparePalette::new(&[], w, cx);
            assert_eq!(
                (c.added(), c.removed()),
                (added, removed),
                "{name}: compare"
            );
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn the_gutter_still_marks_added_and_removed_rows(_cx: &mut TestAppContext) {
    assert_eq!(RowKind::Added.gutter(), "+");
    assert_eq!(RowKind::Removed.gutter(), "-");
}

#[gpui_kit::test]
fn the_changed_mark_reads_on_its_amber_wash_in_every_theme(cx: &mut TestAppContext) {
    let col = collection_for("http://127.0.0.1:1", "");
    let s = started(cx, &col);
    frame(cx, s.win);
    for name in THEMES {
        use_theme(cx, name);
        cx.update_window(s.win, |_, _, cx| {
            let wash = theme::status_fill(Status::Warning, cx);
            let g = theme::diff_glyph(Status::Warning, cx);
            assert!(contrast(g, wash) >= 4.5, "{name}: changed glyph");
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn the_compare_marks_draw_in_the_shared_diff_glyph_colours(cx: &mut TestAppContext) {
    let col = collection_for("http://127.0.0.1:1", "");
    let s = started(cx, &col);
    frame(cx, s.win);
    for name in THEMES {
        use_theme(cx, name);
        cx.update_window(s.win, |_, w, cx| {
            let c = ComparePalette::new(&[], w, cx);
            assert_eq!(
                c.glyph_color(RowKind::Added),
                theme::diff_glyph(Status::Success, cx),
                "{name}: added"
            );
            assert_eq!(
                c.glyph_color(RowKind::Removed),
                theme::diff_glyph(Status::Danger, cx),
                "{name}: removed"
            );
            assert_eq!(
                c.changed_glyph(),
                theme::diff_glyph(Status::Warning, cx),
                "{name}: changed"
            );
        })
        .unwrap();
    }
}

// gpui hues run 0..1 and wrap, so red at 0.99 is close to red at 0.01.
fn hue_gap(a: f32, b: f32) -> f32 {
    let d = (a - b).abs();
    d.min(1. - d)
}

#[gpui_kit::test]
fn the_hue_gap_wraps_around_red(_cx: &mut TestAppContext) {
    assert!(hue_gap(0.99, 0.01) < 0.03);
    assert!(hue_gap(0.0, 0.5) > 0.4);
}

#[gpui_kit::test]
fn hc_dark_removed_mark_is_a_clear_red_on_its_wash(cx: &mut TestAppContext) {
    let col = collection_for("http://127.0.0.1:1", "");
    let s = started(cx, &col);
    frame(cx, s.win);
    use_theme(cx, theme::HC_DARK);
    cx.update_window(s.win, |_, _, cx| {
        let g = theme::diff_glyph(Status::Danger, cx);
        let wash = diff_fill(Status::Danger, cx);
        assert!(contrast(g, wash) >= 4.5, "contrast");
        assert!(hue_gap(g.h, 0.0) < 0.03, "hue {}", g.h);
        assert!(g.s >= 0.8, "saturation {}", g.s);
        // A pale tint of red reads pink; a clear red stays at mid lightness.
        assert!(g.l <= 0.7, "lightness {}", g.l);
    })
    .unwrap();
}
