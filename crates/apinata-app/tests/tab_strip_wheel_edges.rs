//! Wherever a wheel leaves the main tab strip, a tab cut by either edge is covered until
//! enough of its name shows to read, and no fade blanks more than that.

use crate::support;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Pixels, TestAppContext, VisualTestContext, point, px, size};
use support::*;

const NAMES: [&str; 15] = [
    "List users",
    "Get user",
    "Create user with a long title",
    "Delete user",
    "Posts as XML",
    "Comments",
    "Search posts by title",
    "Users / Get user",
    "Albums",
    "Photos for an album",
    "Todos",
    "Update a post",
    "Patch title",
    "Health",
    "Last request users",
];

fn frames(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle, n: usize) {
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        for _ in 0..n {
            w.render_frame(cx);
        }
    })
    .unwrap();
}

fn move_pointer(
    cx: &mut TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    at: gpui_kit::Point<Pixels>,
) {
    use gpui_kit::InputEvent as _;
    cx.update_window(win, |_, w, cx| {
        w.dispatch_event(
            gpui_kit::MouseMoveEvent {
                position: at,
                pressed_button: None,
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
    })
    .unwrap();
}

// What one wheel stop shows at one edge: how much of the tab cut there lies in the viewport,
// its bounds, and the fade drawn over that edge.
struct Edge {
    shown: Pixels,
    tab: gpui_kit::Bounds<Pixels>,
    fade: Option<gpui_kit::Bounds<Pixels>>,
}

// How many wheel stops cut a tab just under or just over the readable width at each edge, and
// how many left tabs hidden past both edges at once.
#[derive(Default)]
struct Seen {
    left: (usize, usize),
    right: (usize, usize),
    both: usize,
    clicks: usize,
}

// Opens fifteen tabs, and for each window width wheels the strip from its end toward its start
// by `step` px, one fresh wheel per stop, checking at each stop that a tab cut at either edge
// is covered whole until it shows `readable` px, and that no fade is wider than a plain fade
// unless it covers such a tab. A close slot inside a covered tab is clicked and must not close.
fn wheel_stops(cx: &mut TestAppContext, widths: &[u32], step: f32, zoom: f32) -> Seen {
    use apinata_app::app::AppState;
    let files: Vec<(String, String)> = NAMES
        .iter()
        .enumerate()
        .map(|(i, n)| (format!("r{i}.yml"), request(n, "http://x/")))
        .collect();
    let mut refs: Vec<(&str, &str)> = vec![("opencollection.yml", ROOT)];
    refs.extend(files.iter().map(|(p, t)| (p.as_str(), t.as_str())));
    let col = write_tree(&refs);
    let s = started(cx, &col);
    for i in 0..NAMES.len() {
        open_tab(cx, &s.ws, &s.root, &format!("r{i}.yml"));
    }
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| p.update(cx, |s| s.zoom = zoom));
    });
    let away = point(px(4.), px(700.));
    let mut seen = Seen::default();
    for &width in widths {
        VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(width as f32), px(800.)));
        frames(cx, s.win, 6);
        for k in 1..(1600. / step) as usize {
            let count = s.ws.read_with(cx, |w, _| w.tabs().entries().len());
            move_pointer(cx, s.win, away);
            s.ws.update(cx, |w, cx| {
                w.update_tabs(cx, |t| t.activate_index(count - 1))
            });
            frames(cx, s.win, 8);
            let back = gpui_kit::ScrollDelta::Pixels(point(px(0.), px(step * k as f32)));
            cx.update_window(s.win, |_, w, cx| {
                w.render_frame(cx);
                w.scroll("tab-strip", back, cx)
            })
            .unwrap();
            frames(cx, s.win, 4);
            let (left, right, view, slot, fade_w) = cx
                .update_window(s.win, |_, w, cx| {
                    w.render_frame(cx);
                    let view = w.find("tab-strip-viewport").bounds();
                    let slot = apinata_app::style::resolve(apinata_app::style::ROW_HEIGHT, w);
                    let fade_w =
                        apinata_app::style::resolve(apinata_app::style::STRIP_FADE_WIDTH, w);
                    let tabs: Vec<_> = (0..count)
                        .map(|ix| w.within("tab-strip").find(ix).bounds())
                        .collect();
                    let left = tabs
                        .iter()
                        .find(|b| {
                            b.left() < view.left() - px(0.5) && b.right() > view.left() + px(0.5)
                        })
                        .map(|b| (b.right() - view.left(), *b));
                    let right = tabs
                        .iter()
                        .find(|b| {
                            b.left() < view.right() - px(0.5) && b.right() > view.right() + px(0.5)
                        })
                        .map(|b| (view.right() - b.left(), *b));
                    let (fl, fr) = (
                        w.try_find("tab-strip-fade-left").map(|f| f.bounds()),
                        w.try_find("tab-strip-fade-right").map(|f| f.bounds()),
                    );
                    let edge = |cut: Option<(Pixels, gpui_kit::Bounds<Pixels>)>, fade| {
                        cut.map(|(shown, tab)| Edge { shown, tab, fade })
                            .ok_or(fade)
                    };
                    (edge(left, fl), edge(right, fr), view, slot, fade_w)
                })
                .unwrap();
            let readable = slot + fade_w * 2.;
            let near = |shown: Pixels| (shown - readable).abs() <= px(8.);
            let at = format!("at {width} px wheeled {} px", step * k as f32);
            let plain = |fade: Option<gpui_kit::Bounds<Pixels>>| {
                fade.is_none_or(|f| f.size.width <= fade_w + px(0.5))
            };
            let fade_of = |e: &Result<Edge, Option<gpui_kit::Bounds<Pixels>>>| match e {
                Ok(e) => e.fade,
                Err(f) => *f,
            };
            seen.both += usize::from(fade_of(&left).is_some() && fade_of(&right).is_some());
            // The left edge: a tab showing less than its readable width is covered whole, and
            // a press on its close slot only selects it; one showing more gets a plain fade.
            match &left {
                Err(fade) => assert!(plain(*fade), "{at}: the left fade {fade:?} blanks no tab"),
                Ok(e) if e.shown < readable - px(1.) => {
                    seen.left.0 += usize::from(near(e.shown));
                    let f = e
                        .fade
                        .expect("a tab cut at the left edge has a fade over it");
                    assert!(
                        f.size.width >= e.shown - px(0.5)
                            && f.size.width <= e.shown + fade_w + px(0.5),
                        "{at}: {:?} of tab {:?} shows beside the left fade {f:?}",
                        e.shown,
                        e.tab
                    );
                    let close = point(e.tab.right() - slot / 2., e.tab.center().y);
                    if close.x > view.left() + px(1.) {
                        let strip = cx
                            .update_window(s.win, |_, w, _| w.find("tab-strip").bounds())
                            .unwrap();
                        cx.update_window(s.win, |_, w, cx| {
                            w.click_at("tab-strip", close - strip.origin, cx)
                        })
                        .unwrap();
                        frames(cx, s.win, 4);
                        seen.clicks += 1;
                        let after = s.ws.read_with(cx, |w, _| w.tabs().entries().len());
                        assert_eq!(
                            after, count,
                            "{at}: a press on the close slot of tab {:?}, {:?} of it shown, closed it",
                            e.tab, e.shown
                        );
                    }
                }
                Ok(e) => {
                    seen.left.1 += usize::from(near(e.shown) && e.shown > readable + px(1.));
                    assert!(
                        plain(e.fade),
                        "{at}: the left fade {:?} blanks {:?} of tab {:?}",
                        e.fade,
                        e.shown,
                        e.tab
                    );
                }
            }
            // The right edge mirrors it: no fade wider than a plain one beside a readable tab.
            match &right {
                Err(fade) => assert!(plain(*fade), "{at}: the right fade {fade:?} blanks no tab"),
                Ok(e) if e.shown < readable - px(1.) => {
                    seen.right.0 += usize::from(near(e.shown));
                    let f = e
                        .fade
                        .expect("a tab cut at the right edge has a fade over it");
                    assert!(
                        f.size.width >= e.shown - px(0.5)
                            && f.size.width <= e.shown + fade_w + px(0.5),
                        "{at}: {:?} of tab {:?} shows beside the right fade {f:?}",
                        e.shown,
                        e.tab
                    );
                }
                Ok(e) => {
                    seen.right.1 += usize::from(near(e.shown) && e.shown > readable + px(1.));
                    assert!(
                        plain(e.fade),
                        "{at}: the right fade {:?} blanks {:?} of tab {:?}",
                        e.fade,
                        e.shown,
                        e.tab
                    );
                }
            }
        }
    }
    std::mem::forget(col);
    seen
}

fn assert_not_vacuous(seen: &Seen) {
    assert!(
        seen.left.0 > 0 && seen.left.1 > 0 && seen.right.0 > 0 && seen.right.1 > 0,
        "stops near the readable width: left {:?}, right {:?}",
        seen.left,
        seen.right
    );
    assert!(seen.both > 0, "no stop left tabs hidden past both edges");
    assert!(seen.clicks > 0, "no covered close slot was pressed");
}

// Fifteen tabs at 1280, 930 and 750 px, wheeled by 9 px stops from the end toward the start.
#[gpui_kit::test]
fn a_wheel_never_leaves_a_close_button_beside_an_unread_name(cx: &mut TestAppContext) {
    let seen = wheel_stops(cx, &[1280, 930, 750], 9., 1.0);
    assert_not_vacuous(&seen);
}

// The same at 125%, where the readable width grows with the close slot and the fade.
#[gpui_kit::test]
fn at_a_larger_zoom_a_wheel_never_leaves_a_close_button_beside_an_unread_name(
    cx: &mut TestAppContext,
) {
    let seen = wheel_stops(cx, &[1280, 800], 11., 1.25);
    assert_not_vacuous(&seen);
}
