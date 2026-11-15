//! The main tab strip never leaves a sliver of a tab at its left edge once a tab is chosen,
//! never shows a close button or dirty dot beside a name too cut to read, and never draws a
//! fade wider than a plain one.

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
// hide a cut tab nor show it without cutting the chosen one; such a sliver then shows no
// glyph in its slot until a readable run of its name shows. Returns how many checks found the
// strip scrolled.
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
                    // Only at the strip's end may a tab stay cut, and then its slot is empty
                    // unless a readable run of its name shows.
                    let readable =
                        apinata_app::style::resolve(apinata_app::style::TAB_READABLE_WIDTH, w);
                    let cut: Vec<String> = (0..count)
                        .filter_map(|ix| {
                            let b = w.within("tab-strip").find(ix).bounds();
                            let cut = b.left() < view.left() - px(0.5)
                                && b.right() > view.left() + px(0.5);
                            let glyph = w.try_find(("tab-close", ix)).is_some()
                                || w.try_find(("dirty", ix)).is_some()
                                || w.try_find(("tab-pin", ix)).is_some();
                            let quiet = at_end && !glyph;
                            let name = w.find(("tab-name", ix)).bounds();
                            let shows = name.right() - view.left() >= readable - px(1.);
                            (cut && !quiet && !shows).then(|| format!("tab {ix} {b:?}"))
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
// may cut a tab at its left edge that it can neither show whole nor hide. The left fade over it
// is a plain fade, never a wider band, however much of that tab shows.
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
        let (shown, fade, readable, fade_w) = cx
            .update_window(s.win, |_, w, cx| {
                w.render_frame(cx);
                let view = w.find("tab-strip-viewport").bounds();
                let fade_w = apinata_app::style::resolve(apinata_app::style::STRIP_FADE_WIDTH, w);
                let readable =
                    apinata_app::style::resolve(apinata_app::style::TAB_READABLE_WIDTH, w);
                // How much of a tab cut at the left edge lies in the viewport.
                let shown = (0..count).find_map(|ix| {
                    let b = w.within("tab-strip").find(ix).bounds();
                    (b.left() < view.left() - px(0.5) && b.right() > view.left() + px(0.5))
                        .then(|| b.right() - view.left())
                });
                let fade = w.try_find("tab-strip-fade-left").map(|f| f.bounds());
                (shown, fade, readable, fade_w)
            })
            .unwrap();
        if shown.is_some_and(|s| s > readable + px(1.)) {
            wide_cuts += 1;
        }
        assert!(
            fade.is_none_or(|f| (f.size.width - fade_w).abs() < px(0.5)),
            "at {width} px the left fade {fade:?} beside {shown:?} of a cut tab is not a plain fade"
        );
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
// left edge, how much of its name shows, the left fade and that tab's close button.
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
                    shown: w.find(("tab-name", ix)).bounds().right() - view.left(),
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

// A cut tab shows no close button until `readable` px of its name shows; past that its button
// shows clear of the fade. The fade is a plain `fade` px either way. Cuts within 3 px either
// side of the boundary must be met.
fn closes_only_once_readable(cuts: &[LeftCut], readable: f32, fade: f32) {
    let (mut under, mut over) = (0, 0);
    let near = |shown: f32| (shown - readable).abs() <= 3.;
    for c in cuts {
        let shown = f32::from(c.shown);
        assert!(
            (f32::from(c.fade.size.width) - fade).abs() < 0.5,
            "at {} px, {shown} px of a tab shows beside a {:?} fade",
            c.width,
            c.fade
        );
        if shown <= readable - 1. {
            under += usize::from(near(shown));
            assert!(
                c.close.is_none(),
                "at {} px, {shown} px of a tab shows its close button {:?}",
                c.width,
                c.close
            );
        } else if shown >= readable + 1. {
            over += usize::from(near(shown));
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

// The readable run of a cut tab's name, from the same constant the strip uses.
fn readable_at(zoom: f32) -> f32 {
    f32::from(apinata_app::style::at_zoom(
        apinata_app::style::TAB_READABLE_WIDTH,
        zoom,
    ))
}

// At 100% two 24 px fades make 48 px: with less of its name showing, a cut tab shows no close
// button, so none sits beside a faded glyph it would close unread.
#[gpui_kit::test]
fn a_cut_tab_shows_only_once_its_name_can_be_read(cx: &mut TestAppContext) {
    let readable = readable_at(1.0);
    assert_eq!(readable, 48.);
    let cuts = left_cuts(cx, 1.0, 640..=800);
    closes_only_once_readable(&cuts, readable, 24.);
}

// At 125% a fade is 30 px, so a cut tab's button shows from 60 px of its name.
#[gpui_kit::test]
fn at_a_larger_zoom_a_cut_tab_shows_only_once_its_name_can_be_read(cx: &mut TestAppContext) {
    let readable = readable_at(1.25);
    assert_eq!(readable, 60.);
    let cuts = left_cuts(cx, 1.25, 640..=760);
    closes_only_once_readable(&cuts, readable, 30.);
}
