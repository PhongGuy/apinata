//! A tab cut by the main tab strip's right edge never shows its close button, whose place
//! lies past the edge, and the right fade over it is a plain fade, never a blank band.

use crate::support;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext, px, size};
use support::*;

// Opens `count` request tabs and, for each window width, activates each tab in turn and then
// wheels the strip a notch at a time, checking after each step that a tab cut by the strip's
// right edge shows no close button and that the right fade is a plain one. Returns how many
// checks found a tab cut there.
fn never_cut(cx: &mut TestAppContext, count: usize, widths: impl Iterator<Item = u32>) -> usize {
    let files: Vec<(String, String)> = (0..count)
        .map(|i| {
            (
                format!("r{i}.yml"),
                request(&format!("Apinata demo environment {i}"), "http://x/"),
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
    let mut covered = 0;
    let mut check = |cx: &mut TestAppContext, why: &str| {
        let cut = cx
            .update_window(s.win, |_, w, cx| {
                w.render_frame(cx);
                let view = w.find("tab-strip-viewport").bounds();
                let fade = w.try_find("tab-strip-fade-right").map(|f| f.bounds());
                let fade_w = apinata_app::style::resolve(apinata_app::style::STRIP_FADE_WIDTH, w);
                (0..count).find_map(|ix| {
                    let b = w.within("tab-strip").find(ix).bounds();
                    let cut =
                        b.left() < view.right() - px(0.5) && b.right() > view.right() + px(0.5);
                    if !cut {
                        return None;
                    }
                    let plain = fade.is_some_and(|f| (f.size.width - fade_w).abs() < px(0.5));
                    let hidden = plain && w.try_find(("tab-close", ix)).is_none();
                    Some((
                        ix,
                        hidden,
                        format!("tab {ix} {b:?} in {view:?}, right fade {fade:?}"),
                    ))
                })
            })
            .unwrap();
        if let Some((_, hidden, what)) = cut {
            covered += 1;
            assert!(hidden, "{why}: {what} shows a close button or a wide fade");
        }
    };
    for width in widths {
        VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(width as f32), px(800.)));
        frames(cx, s.win, 6);
        for pick in 0..count {
            s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(pick)));
            frames(cx, s.win, 6);
            check(cx, &format!("at {width} px, tab {pick} chosen"));
        }
        s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(0)));
        frames(cx, s.win, 6);
        for notch in 0..12 {
            let down = gpui_kit::ScrollDelta::Lines(gpui_kit::point(0., -1.));
            cx.update_window(s.win, |_, w, cx| {
                w.render_frame(cx);
                w.scroll("tab-strip", down, cx)
            })
            .unwrap();
            frames(cx, s.win, 2);
            check(cx, &format!("at {width} px, wheeled {notch}"));
        }
    }
    std::mem::forget(col);
    covered
}

fn frames(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle, n: usize) {
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        for _ in 0..n {
            w.render_frame(cx);
        }
    })
    .unwrap();
}

// Seven long-named tabs at 1280, 800 and 640 px: a tab cut at the right edge offers no close
// button and sits under a plain fade, whether a tab was chosen or the strip wheeled.
#[gpui_kit::test]
fn a_tab_cut_at_the_right_edge_offers_no_close_button(cx: &mut TestAppContext) {
    let covered = never_cut(cx, 7, [1280, 800, 640].into_iter());
    assert!(covered > 0, "no tab was ever cut at the right edge");
}
