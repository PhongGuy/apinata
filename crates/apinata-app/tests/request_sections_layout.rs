//! The request's section strip never shows a sliver of a section tab at its left edge once a
//! later section is chosen, side by side or stacked.

use crate::support;

use apinata_app::request_editor::Section;
use apinata_app::settings::Orientation;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext, VisualTestContext, px, size};
use support::*;

// Lets the kit's spring reveal run to its end, a frame at a time.
fn settle_motion(cx: &mut TestAppContext, win: AnyWindowHandle) {
    for _ in 0..60 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        frame(cx, win);
    }
}

// For each window width, shows Params, then chooses each of `chosen` as a user would (its
// label when it shows, else the overflow menu) and checks no section tab shows as a sliver at
// the strip's left edge. Returns how many checks found the strip overflowing.
fn never_cut(
    cx: &mut TestAppContext,
    orientation: Orientation,
    widths: impl Iterator<Item = u32>,
    height: f32,
    chosen: &[Section],
) -> usize {
    use apinata_app::app::AppState;
    let col = collection_for("http://127.0.0.1:9", "");
    let s = started(cx, &col);
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| {
            p.update(cx, |st| st.response_orientation = orientation)
        });
    });
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1400.), px(height)));
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let ed = editor_of(cx, &s, tab);
    let mut scrolled = 0;
    for width in widths {
        VisualTestContext::from_window(s.win, cx)
            .simulate_resize(size(px(width as f32), px(height)));
        for &pick in chosen {
            ed.update(cx, |e, cx| e.show(Section::Params, cx));
            settle_motion(cx, s.win);
            let target = ("section", pick as usize);
            let clickable = cx
                .update_window(s.win, |_, w, cx| {
                    w.render_frame(cx);
                    let view = w.try_find("section-strip-viewport")?.bounds();
                    let c = w.find(target).bounds().center();
                    Some(c.x > view.left() && c.x < view.right())
                })
                .unwrap();
            match clickable {
                Some(false) => {
                    cx.update_window(s.win, |_, w, cx| w.click("section-overflow", cx))
                        .unwrap();
                    frame(cx, s.win);
                    cx.update_window(s.win, |_, w, cx| {
                        w.within("popup-menu").click(pick as usize, cx)
                    })
                    .unwrap();
                }
                _ => cx
                    .update_window(s.win, |_, w, cx| w.click(target, cx))
                    .unwrap(),
            }
            settle_motion(cx, s.win);
            assert_eq!(ed.read_with(cx, |e, _| e.section()), pick);
            let cuts = cx
                .update_window(s.win, |_, w, cx| {
                    w.render_frame(cx);
                    let view = w.try_find("section-strip-viewport")?.bounds();
                    // Scrolled to its end, the strip may have no room to hide a cut tab nor to
                    // show it without cutting the chosen one; that sliver must then lie wholly
                    // under the left fade.
                    let at_end = w.try_find("section-strip-fade-right").is_none();
                    let fade = w.try_find("section-strip-fade-left").map(|f| f.bounds());
                    let cut: Vec<String> = Section::ALL
                        .iter()
                        .filter_map(|&sec| {
                            let b = w.find(("section", sec as usize)).bounds();
                            let cut = b.left() < view.left() - px(0.5)
                                && b.right() > view.left() + px(0.5);
                            let faded = at_end && fade.is_some_and(|f| b.right() <= f.right());
                            (cut && !faded).then(|| format!("{sec:?} {b:?}"))
                        })
                        .collect();
                    // With no tab cut at the left edge, the left fade keeps its usual width.
                    let any_cut = Section::ALL.iter().any(|&sec| {
                        let b = w.find(("section", sec as usize)).bounds();
                        b.left() < view.left() - px(0.5) && b.right() > view.left() + px(0.5)
                    });
                    let grade =
                        apinata_app::style::resolve(apinata_app::style::STRIP_FADE_WIDTH, w);
                    assert!(
                        any_cut || fade.is_none_or(|f| f.size.width <= grade + px(0.5)),
                        "the left fade covers {fade:?} with no tab cut"
                    );
                    Some((cut, format!("{view:?}, left fade {fade:?}")))
                })
                .unwrap();
            if let Some((cut, view)) = cuts {
                if present(cx, s.win, "section-overflow") {
                    scrolled += 1;
                }
                assert!(
                    cut.is_empty(),
                    "{orientation:?} at {width} px, {pick:?} chosen, cut tabs {cut:?} in {view}"
                );
            }
        }
    }
    std::mem::forget(col);
    scrolled
}

// Side by side, at every width where the section strip scrolls, choosing a later section leaves
// no section tab as a sliver at the left edge.
#[gpui_kit::test]
fn side_by_side_choosing_a_later_section_never_cuts_a_tab(cx: &mut TestAppContext) {
    let picks = [Section::Settings, Section::Tests];
    let scrolled = never_cut(
        cx,
        Orientation::Horizontal,
        (900..=1600).rev().step_by(20),
        800.,
        &picks,
    );
    assert!(scrolled > 0, "the strip never overflowed");
}

// Stacked in a window about 800 px wide, choosing a later section leaves no tab cut.
#[gpui_kit::test]
fn stacked_at_800_choosing_a_later_section_never_cuts_a_tab(cx: &mut TestAppContext) {
    let picks = [Section::Settings, Section::Tests];
    let scrolled = never_cut(
        cx,
        Orientation::Vertical,
        (640..=960).rev().step_by(20),
        600.,
        &picks,
    );
    assert!(scrolled > 0, "the strip never overflowed");
}
