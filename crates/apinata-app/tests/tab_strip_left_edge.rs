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
// wholly under the left fade until its close slot and a fade width of label clear it. Returns
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
                    // part unless that slot and a fade width of label show clear of the fade.
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
                            let shows = b.right() - view.left() >= slot + fade_w * 2.
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
// covers it whole until its close slot and a fade width of label could show, and blanks no
// more, so no band stands empty where most of a tab could show.
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
                (shown, fade, slot + fade_w * 2., fade_w)
            })
            .unwrap();
        if shown.is_some_and(|s| s > sliver + px(1.)) {
            wide_cuts += 1;
        }
        // A fade that covers a sliver is solid over it and grades at most a fade width after;
        // it is never narrower than a sliver it covers, nor than a fade beside a wider cut.
        assert!(
            fade.is_none_or(|f| f.size.width <= sliver + fade_w + px(0.5)),
            "at {width} px the left fade {fade:?} blanks {shown:?} of a cut tab"
        );
        if let (Some(shown), Some(f)) = (shown, fade) {
            let least = if shown < sliver - px(1.) {
                shown
            } else {
                fade_w
            };
            assert!(
                f.size.width >= least - px(0.5),
                "at {width} px the left fade {f:?} is too narrow for {shown:?} of a cut tab"
            );
        }
    }
    assert!(wide_cuts > 0, "no width cut most of a tab at the left edge");
    std::mem::forget(col);
}

// One window width's view of the tab cut at the strip's left edge, with the last tab chosen.
struct LeftCut {
    width: u32,
    shown: gpui_kit::Pixels,
    fade: gpui_kit::Bounds<gpui_kit::Pixels>,
    close: Option<gpui_kit::Bounds<gpui_kit::Pixels>>,
}

// Six tabs at `zoom` with the last chosen: for each window width that leaves a tab cut at the
// left edge, how much of it shows, the left fade and that tab's close button.
fn left_cuts(
    cx: &mut TestAppContext,
    zoom: f32,
    widths: impl Iterator<Item = u32>,
) -> Vec<LeftCut> {
    use apinata_app::app::AppState;
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
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| p.update(cx, |s| s.zoom = zoom));
    });
    let mut cuts = Vec::new();
    for width in widths {
        VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(width as f32), px(800.)));
        s.ws.update(cx, |w, cx| {
            w.update_tabs(cx, |t| t.activate_index(count - 1))
        });
        settle_motion(cx, s.win);
        let cut = cx
            .update_window(s.win, |_, w, cx| {
                w.render_frame(cx);
                let view = w.find("tab-strip-viewport").bounds();
                let ix = (0..count).find(|ix| {
                    let b = w.within("tab-strip").find(*ix).bounds();
                    b.left() < view.left() - px(0.5) && b.right() > view.left() + px(0.5)
                })?;
                let fade = w.try_find("tab-strip-fade-left")?.bounds();
                Some(LeftCut {
                    width,
                    shown: w.within("tab-strip").find(ix).bounds().right() - view.left(),
                    fade,
                    close: w.try_find(("tab-close", ix)).map(|c| c.bounds()),
                })
            })
            .unwrap();
        cuts.extend(cut);
    }
    std::mem::forget(col);
    cuts
}

// A cut tab stays wholly covered until a readable bit of its name clears the fade, `readable`
// px of it in all; past that the ordinary `fade` px fade shows it and its close button lies
// clear of the fade. Cuts within 3 px either side of the boundary must be met.
fn covered_until_readable(cuts: &[LeftCut], readable: f32, fade: f32) {
    let (mut under, mut over) = (0, 0);
    let near = |shown: f32| (shown - readable).abs() <= 3.;
    for c in cuts {
        let shown = f32::from(c.shown);
        if shown <= readable - 1. {
            under += usize::from(near(shown));
            assert!(
                c.fade.size.width >= c.shown - px(0.5),
                "at {} px, {shown} px of a tab shows beside a {:?} fade",
                c.width,
                c.fade
            );
        } else if shown >= readable + 1. {
            over += usize::from(near(shown));
            assert!(
                (f32::from(c.fade.size.width) - fade).abs() < 0.5,
                "at {} px, {shown} px of a tab is covered by a {:?} fade",
                c.width,
                c.fade
            );
            let close = c
                .close
                .expect("a tab showing its name shows its close button");
            assert!(
                close.left() >= c.fade.right() - px(0.5),
                "at {} px the close button {close:?} lies under the fade {:?}",
                c.width,
                c.fade
            );
        }
    }
    assert!(
        under > 0 && over > 0,
        "{under} cuts just under, {over} just over {readable} px"
    );
}

// At 100% a cut tab's 28 px close slot and two 24 px fades make 76 px: showing less, it is
// covered whole, so no faded glyph sits beside a close button that would close it unread.
#[gpui_kit::test]
fn a_cut_tab_shows_only_once_its_name_can_be_read(cx: &mut TestAppContext) {
    let cuts = left_cuts(cx, 1.0, 640..=800);
    covered_until_readable(&cuts, 76., 24.);
}

// At 125% the close slot is 35 px and a fade 30 px, so a cut tab shows from 95 px.
#[gpui_kit::test]
fn at_a_larger_zoom_a_cut_tab_shows_only_once_its_name_can_be_read(cx: &mut TestAppContext) {
    let cuts = left_cuts(cx, 1.25, 640..=760);
    covered_until_readable(&cuts, 95., 30.);
}
