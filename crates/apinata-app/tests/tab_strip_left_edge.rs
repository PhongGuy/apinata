//! The main tab strip never shows a sliver of a tab, such as a stray close button or dirty
//! dot, at its left edge once a tab is chosen, nor a blank band where most of a tab could show.

use crate::support;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext, VisualTestContext, px, size};
use support::*;

// Lets the strip's reveal and any glide run to their end, a frame at a time.
fn settle_motion(cx: &mut TestAppContext, win: AnyWindowHandle) {
    for _ in 0..60 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        frame(cx, win);
    }
}

// Opens `count` request tabs and, for each window width, activates each tab in turn and checks
// no tab is cut by the strip's left edge. Scrolled to its end the strip may have no room to
// hide a cut tab nor show it without cutting the chosen one; a sliver of it must then lie
// wholly under the left fade, and more of it shows its close slot clear of the fade. Returns
// how many checks found the strip scrolled.
fn never_cut(cx: &mut TestAppContext, count: usize, widths: impl Iterator<Item = u32>) -> usize {
    let files: Vec<(String, String)> = (0..count)
        .map(|i| {
            (
                format!("r{i}.yml"),
                request(&format!("Request number {i}"), "http://x/"),
            )
        })
        .collect();
    let mut refs: Vec<(&str, &str)> = vec![("opencollection.yml", ROOT)];
    refs.extend(files.iter().map(|(p, t)| (p.as_str(), t.as_str())));
    let col = write_tree(&refs);
    let s = started(cx, &col);
    for i in 0..count {
        open_tab(cx, &s.ws, &s.root, &format!("r{i}.yml"));
    }
    let mut scrolled = 0;
    for width in widths {
        VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(width as f32), px(800.)));
        settle_motion(cx, s.win);
        for pick in (0..count).rev().chain(0..count) {
            s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(pick)));
            settle_motion(cx, s.win);
            let (cut, view) = cx
                .update_window(s.win, |_, w, cx| {
                    w.render_frame(cx);
                    let view = w.find("tab-strip-viewport").bounds();
                    let at_end = w.try_find("tab-strip-fade-right").is_none();
                    let fade = w.try_find("tab-strip-fade-left").map(|f| f.bounds());
                    // The cover is solid up to where it grades over the gap before the next tab,
                    // at most a fade width; a cut tab, glyph slot included, must lie in the solid
                    // part unless it shows more than that slot clear of the fade.
                    let fade_w =
                        apinata_app::style::resolve(apinata_app::style::STRIP_FADE_WIDTH, w);
                    let slot = apinata_app::style::resolve(apinata_app::style::ROW_HEIGHT, w);
                    let cut: Vec<String> = (0..count)
                        .filter_map(|ix| {
                            let b = w.within("tab-strip").find(ix).bounds();
                            let cut = b.left() < view.left() - px(0.5)
                                && b.right() > view.left() + px(0.5);
                            let gap = if ix + 1 < count {
                                (w.within("tab-strip").find(ix + 1).bounds().left() - b.right())
                                    .max(px(0.))
                            } else {
                                px(0.)
                            };
                            let grade = gap.min(fade_w);
                            let faded = at_end
                                && fade.is_some_and(|f| b.right() <= f.right() - grade + px(0.5));
                            let shows = b.right() - view.left() > slot + fade_w
                                && fade.is_some_and(|f| b.right() - slot >= f.right() - px(0.5));
                            (cut && !faded && !shows).then(|| format!("tab {ix} {b:?}"))
                        })
                        .collect();
                    (cut, format!("{view:?}, left fade {fade:?}"))
                })
                .unwrap();
            if present(cx, s.win, "tab-strip-fade-left") {
                scrolled += 1;
            }
            assert!(
                cut.is_empty(),
                "at {width} px, tab {pick} chosen, cut tabs {cut:?} in {view}"
            );
        }
    }
    std::mem::forget(col);
    scrolled
}

// Six tabs in windows from 640 to 1280 px wide: choosing any tab leaves none as a sliver at the
// strip's left edge.
#[gpui_kit::test]
fn choosing_a_tab_never_leaves_a_sliver_at_the_left_edge(cx: &mut TestAppContext) {
    let scrolled = never_cut(cx, 6, (640..=1280).rev().step_by(40));
    assert!(scrolled > 0, "the strip never overflowed");
}

// Six tabs with the last chosen, in windows from 640 to 1280 px: scrolled to its end the strip
// may cut a tab at its left edge that it can neither show whole nor hide. The left fade then
// blanks at most a sliver, the close slot and a fade width, so no band stands empty where most
// of a tab could show.
#[gpui_kit::test]
fn the_left_fade_never_blanks_most_of_a_tab(cx: &mut TestAppContext) {
    let count = 6;
    let files: Vec<(String, String)> = (0..count)
        .map(|i| {
            (
                format!("r{i}.yml"),
                request(&format!("Request number {i}"), "http://x/"),
            )
        })
        .collect();
    let mut refs: Vec<(&str, &str)> = vec![("opencollection.yml", ROOT)];
    refs.extend(files.iter().map(|(p, t)| (p.as_str(), t.as_str())));
    let col = write_tree(&refs);
    let s = started(cx, &col);
    for i in 0..count {
        open_tab(cx, &s.ws, &s.root, &format!("r{i}.yml"));
    }
    let mut wide_cuts = 0;
    for width in (640..=1280).step_by(8) {
        VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(width as f32), px(800.)));
        s.ws.update(cx, |w, cx| {
            w.update_tabs(cx, |t| t.activate_index(count - 1))
        });
        settle_motion(cx, s.win);
        let (shown, fade, sliver, fade_w) = cx
            .update_window(s.win, |_, w, cx| {
                w.render_frame(cx);
                let view = w.find("tab-strip-viewport").bounds();
                let fade_w = apinata_app::style::resolve(apinata_app::style::STRIP_FADE_WIDTH, w);
                let slot = apinata_app::style::resolve(apinata_app::style::ROW_HEIGHT, w);
                // How much of a tab cut at the left edge lies in the viewport.
                let shown = (0..count).find_map(|ix| {
                    let b = w.within("tab-strip").find(ix).bounds();
                    (b.left() < view.left() - px(0.5) && b.right() > view.left() + px(0.5))
                        .then(|| b.right() - view.left())
                });
                let fade = w.try_find("tab-strip-fade-left").map(|f| f.bounds());
                (shown, fade, slot + fade_w, fade_w)
            })
            .unwrap();
        if shown.is_some_and(|s| s > sliver + px(1.)) {
            wide_cuts += 1;
        }
        // A fade that covers a sliver is solid over it and grades at most a fade width after.
        assert!(
            fade.is_none_or(|f| f.size.width <= sliver + fade_w + px(0.5)),
            "at {width} px the left fade {fade:?} blanks {shown:?} of a cut tab"
        );
    }
    assert!(wide_cuts > 0, "no width cut most of a tab at the left edge");
    std::mem::forget(col);
}
