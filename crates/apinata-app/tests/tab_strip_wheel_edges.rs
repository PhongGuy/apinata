//! Wherever a wheel leaves the main tab strip, a tab offers its close button only while that
//! button lies clear of the fades and enough of its name shows to read; no fade is wider than
//! a plain one; and a click on a tab never moves the strip while the pointer stays on it.

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

type Area = gpui_kit::Bounds<Pixels>;

// Fifteen request tabs named as above at `zoom`, the last one active.
fn fifteen(cx: &mut TestAppContext, zoom: f32) -> (tempfile::TempDir, Started) {
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
    (col, s)
}

// What one stop of the strip shows: the viewport, the fades, each tab and its close button,
// and the close slot, fade and readable widths at this zoom.
struct Stop {
    view: Area,
    left: Option<Area>,
    right: Option<Area>,
    tabs: Vec<Area>,
    closes: Vec<Option<Area>>,
    slot: Pixels,
    fade: Pixels,
    readable: Pixels,
}

fn stop(cx: &mut TestAppContext, s: &Started) -> Stop {
    let count = tab_count(cx, s);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        use apinata_app::style::{ROW_HEIGHT, STRIP_FADE_WIDTH, TAB_READABLE_WIDTH, resolve};
        Stop {
            view: w.find("tab-strip-viewport").bounds(),
            left: w.try_find("tab-strip-fade-left").map(|f| f.bounds()),
            right: w.try_find("tab-strip-fade-right").map(|f| f.bounds()),
            tabs: (0..count)
                .map(|ix| w.within("tab-strip").find(ix).bounds())
                .collect(),
            closes: (0..count)
                .map(|ix| w.try_find(("tab-close", ix)).map(|c| c.bounds()))
                .collect(),
            slot: resolve(ROW_HEIGHT, w),
            fade: resolve(STRIP_FADE_WIDTH, w),
            readable: resolve(TAB_READABLE_WIDTH, w),
        }
    })
    .unwrap()
}

fn tab_count(cx: &mut TestAppContext, s: &Started) -> usize {
    s.ws.read_with(cx, |w, _| w.tabs().entries().len())
}

// Where tab `ix` stands against the rule: `Some(true)` when its close slot lies clear of the
// fades and the tab shows whole or readable, `Some(false)` when it plainly does not, and
// `None` within a pixel and a half of a boundary, or wholly out of view.
fn may_close(st: &Stop, ix: usize) -> Option<bool> {
    let b = st.tabs[ix];
    let (vl, vr) = (st.view.left(), st.view.right());
    if b.right() <= vl || b.left() >= vr {
        return None;
    }
    let from = vl + st.left.map_or(px(0.), |_| st.fade);
    let to = vr - st.right.map_or(px(0.), |_| st.fade);
    let shown = b.right().min(vr) - b.left().max(vl);
    let whole = b.left() >= vl - px(1.) && b.right() <= vr + px(1.);
    let margins = [
        b.right() - st.slot - from,
        to - b.right(),
        if whole { px(10.) } else { shown - st.readable },
    ];
    if margins.iter().any(|m| m.abs() < px(1.5)) {
        return None;
    }
    Some(margins.iter().all(|m| *m > px(0.)))
}

// Clicks window point `at` on the strip with the pointer moved there first, as a hand would.
fn click(cx: &mut TestAppContext, s: &Started, at: gpui_kit::Point<Pixels>) {
    move_pointer(cx, s.win, at);
    frames(cx, s.win, 2);
    let strip = cx
        .update_window(s.win, |_, w, _| w.find("tab-strip").bounds())
        .unwrap();
    cx.update_window(s.win, |_, w, cx| {
        w.click_at("tab-strip", at - strip.origin, cx)
    })
    .unwrap();
    frames(cx, s.win, 6);
}

const AWAY: (f32, f32) = (4., 700.);

// A fresh stop `by` px back from the strip's end: the last tab chosen with the pointer away,
// then one wheel toward the start.
fn wheel_back(cx: &mut TestAppContext, s: &Started, by: f32) {
    let count = tab_count(cx, s);
    move_pointer(cx, s.win, point(px(AWAY.0), px(AWAY.1)));
    s.ws.update(cx, |w, cx| {
        w.update_tabs(cx, |t| t.activate_index(count - 1))
    });
    frames(cx, s.win, 8);
    let back = gpui_kit::ScrollDelta::Pixels(point(px(0.), px(by)));
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.scroll("tab-strip", back, cx)
    })
    .unwrap();
    frames(cx, s.win, 4);
}

// What the stops met, so the checks cannot pass on a strip that never cut a tab.
#[derive(Default)]
struct Seen {
    left_under: usize,
    left_over: usize,
    right_hidden: usize,
    both: usize,
    pressed: usize,
    closed: usize,
}

// Wheels the strip back from its end by `step` px stops, a fresh wheel each, at each width.
// At every stop each tab in view shows its close button exactly when the rule allows; a press
// on a hidden button's place only selects the tab; both fades are plain and some tab reaches
// into each, so no blank band shows. Once per width a readable cut tab's button closes it,
// leaving one tab fewer for the stops after.
fn wheel_stops(cx: &mut TestAppContext, widths: &[u32], step: f32, zoom: f32) -> Seen {
    let (col, s) = fifteen(cx, zoom);
    let mut seen = Seen::default();
    for &width in widths {
        VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(width as f32), px(800.)));
        frames(cx, s.win, 6);
        let mut closable_at = None;
        let mut seen_closed_here = 0;
        for k in 1..(1600. / step) as usize {
            let by = step * k as f32;
            wheel_back(cx, &s, by);
            let st = stop(cx, &s);
            let at = format!("at {width} px wheeled {by} px");
            for (side, fade) in [("left", st.left), ("right", st.right)] {
                assert!(
                    fade.is_none_or(|f| (f.size.width - st.fade).abs() < px(0.5)),
                    "{at}: the {side} fade {fade:?} is wider than a plain fade"
                );
            }
            // Tabs lie beyond a fade, so some tab reaches under it: the strip is never blank
            // for more than a fade at either edge.
            let reaches = |lo: Pixels, hi: Pixels| {
                st.tabs
                    .iter()
                    .any(|b| b.right() > lo + px(0.5) && b.left() < hi - px(0.5))
            };
            let (vl, vr) = (st.view.left(), st.view.right());
            assert!(
                st.left.is_none() || reaches(vl, vl + st.fade),
                "{at}: no tab reaches under the left fade"
            );
            assert!(
                st.right.is_none() || reaches(vr - st.fade, vr),
                "{at}: no tab reaches under the right fade"
            );
            seen.both += usize::from(st.left.is_some() && st.right.is_some());
            let mut press = None;
            for ix in 0..st.tabs.len() {
                let Some(may) = may_close(&st, ix) else {
                    continue;
                };
                let b = st.tabs[ix];
                let close = st.closes[ix];
                assert_eq!(
                    close.is_some(),
                    may,
                    "{at}: tab {ix} {b:?} in {:?}, fades {:?} {:?}, close button {close:?}",
                    st.view,
                    st.left,
                    st.right
                );
                let left_cut = b.left() < vl - px(0.5) && b.right() > vl + px(0.5);
                let shown = b.right() - vl;
                let near = (shown - st.readable).abs() <= px(8.);
                if left_cut && near && !may {
                    seen.left_under += 1;
                }
                if left_cut && near && may {
                    seen.left_over += 1;
                    if closable_at.is_none() {
                        closable_at = Some((ix, close));
                    }
                }
                if !may && b.right() > vr - st.fade {
                    seen.right_hidden += 1;
                }
                let spot = point(b.right() - st.slot / 2., b.center().y);
                if !may && press.is_none() && spot.x > vl + px(1.) && spot.x < vr - px(1.) {
                    press = Some((ix, spot));
                }
            }
            // Once per width, a cut tab past the readable width closes from its own button.
            if let Some((ix, Some(close))) = closable_at.filter(|_| seen_closed_here == 0) {
                let count = st.tabs.len();
                click(cx, &s, close.center());
                assert_eq!(
                    tab_count(cx, &s),
                    count - 1,
                    "{at}: the close button of tab {ix} did not close it"
                );
                seen.closed += 1;
                seen_closed_here += 1;
                continue;
            }
            if let Some((ix, spot)) = press {
                let count = st.tabs.len();
                click(cx, &s, spot);
                seen.pressed += 1;
                assert_eq!(
                    tab_count(cx, &s),
                    count,
                    "{at}: a press where tab {ix}'s hidden close button would be closed it"
                );
                let tip = cx
                    .update_window(s.win, |_, w, _| w.try_find(("tab-close", ix)).is_some())
                    .unwrap();
                assert!(
                    !tip,
                    "{at}: tab {ix} shows a close button under the pointer"
                );
            }
        }
    }
    std::mem::forget(col);
    seen
}

fn assert_not_vacuous(seen: &Seen) {
    assert!(
        seen.left_under > 0 && seen.left_over > 0,
        "left cuts near the readable width: {} under, {} over",
        seen.left_under,
        seen.left_over
    );
    assert!(
        seen.right_hidden > 0,
        "no close slot lay under the right fade"
    );
    assert!(seen.both > 0, "no stop left tabs hidden past both edges");
    assert!(
        seen.pressed > 0,
        "no hidden close button's place was pressed"
    );
    assert!(
        seen.closed > 0,
        "no readable cut tab was closed from its button"
    );
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

// From wheel stops that cut the active last tab at the right edge, or the active first tab at
// the left, a click on what shows of its name leaves the strip where it is while the pointer
// stays on the strip, even moved along it; once the pointer leaves, the tab is revealed clear
// of the fades.
#[gpui_kit::test]
fn clicking_a_cut_active_tab_holds_the_strip_until_the_pointer_leaves(cx: &mut TestAppContext) {
    let (_col, s) = fifteen(cx, 1.0);
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1280.), px(800.)));
    frames(cx, s.win, 6);
    let count = tab_count(cx, &s);
    let mut clicks = 0;
    for (pick, dir) in [(count - 1, 1.), (0, -1.)] {
        for k in 1..40 {
            move_pointer(cx, s.win, point(px(AWAY.0), px(AWAY.1)));
            let home = if pick == 0 { count - 1 } else { 0 };
            s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(home)));
            frames(cx, s.win, 8);
            s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(pick)));
            frames(cx, s.win, 8);
            let wheel = gpui_kit::ScrollDelta::Pixels(point(px(0.), px(dir * 13. * k as f32)));
            cx.update_window(s.win, |_, w, cx| {
                w.render_frame(cx);
                w.scroll("tab-strip", wheel, cx)
            })
            .unwrap();
            frames(cx, s.win, 4);
            let st = stop(cx, &s);
            let b = st.tabs[pick];
            let (vl, vr) = (st.view.left(), st.view.right());
            let cut = (b.left() < vl - px(1.)) || (b.right() > vr + px(1.));
            // What shows of the name, clear of the fades and the close slot.
            let (lo, hi) = (
                b.left().max(vl + st.fade),
                (b.right() - st.slot).min(vr - st.fade),
            );
            if !cut || hi - lo < px(12.) {
                continue;
            }
            let at = point((lo + hi) / 2., b.center().y);
            let why = format!("tab {pick} wheeled {} px", dir * 13. * k as f32);
            click(cx, &s, at);
            clicks += 1;
            assert_eq!(
                stop(cx, &s).tabs[pick],
                b,
                "{why}: the strip moved under the pointer"
            );
            move_pointer(cx, s.win, point(st.view.center().x, b.center().y));
            frames(cx, s.win, 8);
            assert_eq!(
                stop(cx, &s).tabs[pick],
                b,
                "{why}: the strip moved with the pointer still on it"
            );
            move_pointer(cx, s.win, point(px(AWAY.0), px(AWAY.1)));
            frames(cx, s.win, 8);
            let after = stop(cx, &s);
            let t = after.tabs[pick];
            let from = vl + after.left.map_or(px(0.), |_| after.fade);
            let to = vr - after.right.map_or(px(0.), |_| after.fade);
            assert!(
                t.left() >= from - px(1.) && t.right() <= to + px(1.),
                "{why}: once the pointer left, tab {t:?} is not clear of the fades {from:?}..{to:?}"
            );
        }
    }
    assert!(clicks > 1, "only {clicks} wheel stops cut the active tab");
}
