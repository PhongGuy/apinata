//! Tabs past the strip's width scroll, activation brings a tab into view, and the
//! overflow menu reaches every tab.

use crate::support;

use apinata_app::tab_area::TabArea;
use apinata_app::workspace::Workspace;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, TestAppContext, WindowBounds, WindowOptions,
    point, px, size,
};
use support::*;

struct Narrow {
    _col: tempfile::TempDir,
    _state: tempfile::TempDir,
    ws: Entity<Workspace>,
    win: AnyWindowHandle,
    root: std::path::PathBuf,
}

// Ten request tabs in a 360 px window, the last one active.
fn narrow(cx: &mut TestAppContext) -> Narrow {
    strip(cx, 10, 360.)
}

// `count` request tabs in a window `width` px wide, the last one active.
fn strip(cx: &mut TestAppContext, count: usize, width: f32) -> Narrow {
    strip_opening(cx, count, count, width)
}

// `count` request files, the first `open` of them opened as tabs, in a window `width` px wide.
fn strip_opening(cx: &mut TestAppContext, count: usize, open: usize, width: f32) -> Narrow {
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
    let (state, ws, root) = open_dir(cx, &col);
    support::init(cx);
    let bounds = Bounds::new(point(px(0.), px(0.)), size(px(width), px(480.)));
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        ..Default::default()
    };
    let shared = ws.clone();
    let (win, _view) = cx
        .update(|cx| {
            gpui_kit::open_window(options, cx, |w, cx| {
                cx.new(|cx| TabArea::new(shared, w, cx))
            })
        })
        .unwrap();
    for i in 0..open {
        open_tab(cx, &ws, &root, &format!("r{i}.yml"));
    }
    cx.run_until_parked();
    Narrow {
        _col: col,
        _state: state,
        ws,
        win,
        root,
    }
}

// Two frames: the first lays out and scrolls, the second paints at the new offset.
fn visible(cx: &mut TestAppContext, win: AnyWindowHandle, ix: usize) -> bool {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.render_frame(cx);
        w.find(("tab", ix)).visible()
    })
    .unwrap()
}

// Whether tab `ix` lies wholly in the strip's viewport once the frames have settled; a tab cut
// by an edge is drawn (`visible`) but not whole.
fn whole(cx: &mut TestAppContext, win: AnyWindowHandle, ix: usize) -> bool {
    visible(cx, win, ix);
    inside(cx, win, ix)
}

fn active(cx: &mut TestAppContext, n: &Narrow) -> Option<String> {
    n.ws.read_with(cx, |w, _| {
        w.tabs()
            .active()
            .and_then(|e| e.item())
            .map(|p| p.as_str().to_owned())
    })
}

#[gpui_kit::test]
fn activating_a_tab_scrolls_it_into_view(cx: &mut TestAppContext) {
    let n = narrow(cx);
    assert!(whole(cx, n.win, 9), "the newest tab was revealed");
    assert!(!visible(cx, n.win, 0), "ten tabs do not fit in 360 px");
    n.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(0)));
    cx.run_until_parked();
    assert!(whole(cx, n.win, 0));
    assert!(!visible(cx, n.win, 9));
}

#[gpui_kit::test]
fn the_overflow_menu_activates_a_tab(cx: &mut TestAppContext) {
    let n = narrow(cx);
    cx.update_window(n.win, |_, w, cx| w.click("tab-overflow", cx))
        .unwrap();
    frame(cx, n.win);
    cx.update_window(n.win, |_, w, cx| w.within("popup-menu").click(4usize, cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(active(cx, &n).as_deref(), Some("r4.yml"));
    assert!(whole(cx, n.win, 4));
}

#[gpui_kit::test]
fn the_overflow_menu_starts_on_the_active_tab_so_one_down_moves_past_it(cx: &mut TestAppContext) {
    let n = narrow(cx);
    n.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(3)));
    cx.run_until_parked();
    cx.update_window(n.win, |_, w, cx| w.click("tab-overflow", cx))
        .unwrap();
    frame(cx, n.win);
    press_key(cx, n.win, "down");
    press_key(cx, n.win, "enter");
    cx.run_until_parked();
    assert_eq!(active(cx, &n).as_deref(), Some("r4.yml"));
}

#[gpui_kit::test]
fn the_overflow_chevron_is_named(cx: &mut TestAppContext) {
    let n = narrow(cx);
    let label = cx
        .update_window(n.win, |_, w, cx| {
            w.render_frame(cx);
            w.find("tab-overflow").label().map(|l| l.to_string())
        })
        .unwrap();
    assert_eq!(label.as_deref(), Some("All tabs"));
}

// Whether tab `ix` lies wholly in the strip's viewport, right of the inset.
fn inside(cx: &mut TestAppContext, win: AnyWindowHandle, ix: usize) -> bool {
    cx.update_window(win, |_, w, _| {
        let tab = w.find(("tab", ix)).bounds();
        let left = w.find("tab-inset").bounds().right();
        let right = w.find("tab-strip-viewport").bounds().right();
        tab.left() >= left - px(0.5) && tab.right() <= right + px(0.5)
    })
    .unwrap()
}

fn frames(cx: &mut TestAppContext, win: AnyWindowHandle, n: usize) {
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        for _ in 0..n {
            w.render_frame(cx);
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_resize_reveals_the_active_tab(cx: &mut TestAppContext) {
    let n = narrow(cx);
    n.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(9)));
    cx.simulate_window_resize(n.win, size(px(2600.), px(480.)));
    frames(cx, n.win, 2);
    assert!(inside(cx, n.win, 9), "a wide strip shows the last tab");
    cx.simulate_window_resize(n.win, size(px(360.), px(480.)));
    frames(cx, n.win, 2);
    assert!(
        inside(cx, n.win, 9),
        "narrowing keeps the active tab in view"
    );
}

#[gpui_kit::test]
fn an_oversized_tab_does_not_loop(cx: &mut TestAppContext) {
    let n = strip(cx, 3, 120.);
    cx.run_until_parked();
    // Each frame delivers the frames the previous one asked for; by the fourth none is asked.
    let asked = cx
        .update_window(n.win, |_, w, cx| {
            (0..4)
                .map(|_| {
                    w.render_frame(cx);
                    w.simulate_next_frame(cx)
                })
                .collect::<Vec<_>>()
        })
        .unwrap();
    assert_eq!(
        asked[3], 0,
        "the strip settles within three frames: {asked:?}"
    );
    let (tab, left) = cx
        .update_window(n.win, |_, w, _| {
            let tab = w.find(("tab", 2usize)).bounds();
            (tab.left(), w.find("tab-inset").bounds().right())
        })
        .unwrap();
    // A tab wider than the strip aligns left past the fade.
    let left = left + fade_width(cx, n.win);
    assert!(
        (tab - left).abs() < px(0.5),
        "aligned left past the fade: {tab:?} vs {left:?}"
    );
}

// Picking the active tab from the overflow menu after the user wheeled it out of view
// brings it back.
#[gpui_kit::test]
fn picking_the_active_tab_from_the_menu_after_a_wheel_reveals_it(cx: &mut TestAppContext) {
    let n = narrow(cx);
    assert!(visible(cx, n.win, 9));
    for _ in 0..10 {
        cx.update_window(n.win, |_, w, cx| {
            w.render_frame(cx);
            let left = gpui_kit::ScrollDelta::Pixels(point(px(400.), px(0.)));
            w.scroll("tab-strip", left, cx)
        })
        .unwrap();
    }
    assert!(visible(cx, n.win, 0), "the wheel reached the first tab");
    assert!(!visible(cx, n.win, 9), "the wheel hid the active tab");
    cx.update_window(n.win, |_, w, cx| w.click("tab-overflow", cx))
        .unwrap();
    frame(cx, n.win);
    cx.update_window(n.win, |_, w, cx| w.within("popup-menu").click(9usize, cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(active(cx, &n).as_deref(), Some("r9.yml"));
    assert!(whole(cx, n.win, 9), "picking it from the menu reveals it");
}

// A plain vertical wheel over the strip scrolls it sideways: the strip only scrolls on x, so
// a mouse without a horizontal wheel still reaches every tab.
#[gpui_kit::test]
fn a_vertical_wheel_scrolls_the_tab_strip(cx: &mut TestAppContext) {
    let n = narrow(cx);
    assert!(visible(cx, n.win, 9), "the newest tab starts revealed");
    let wheel = |cx: &mut TestAppContext, dy: f32| {
        for _ in 0..10 {
            cx.update_window(n.win, |_, w, cx| {
                w.render_frame(cx);
                let delta = gpui_kit::ScrollDelta::Pixels(point(px(0.), px(dy)));
                w.scroll("tab-strip", delta, cx)
            })
            .unwrap();
        }
    };
    wheel(cx, 400.);
    assert!(visible(cx, n.win, 0), "wheeling up reached the first tab");
    assert!(!visible(cx, n.win, 9), "and moved the last one out");
    wheel(cx, -400.);
    assert!(visible(cx, n.win, 9), "wheeling down reached the last tab");
    assert!(!visible(cx, n.win, 0));
}

// Which of the strip's edge fades show, after two frames so a reveal has landed.
fn fades(cx: &mut TestAppContext, win: AnyWindowHandle) -> (bool, bool) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.render_frame(cx);
        (
            w.try_find("tab-strip-fade-left").is_some(),
            w.try_find("tab-strip-fade-right").is_some(),
        )
    })
    .unwrap()
}

// A fade marks each edge with tabs beyond it, and none shows on a strip that fits.
#[gpui_kit::test]
fn the_tab_strip_fades_toward_its_hidden_tabs(cx: &mut TestAppContext) {
    let n = narrow(cx);
    assert!(visible(cx, n.win, 9));
    assert_eq!(fades(cx, n.win), (true, false), "scrolled to the end");
    n.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(0)));
    cx.run_until_parked();
    assert!(visible(cx, n.win, 0));
    assert_eq!(fades(cx, n.win), (false, true), "back at the start");
    n.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(5)));
    cx.run_until_parked();
    assert!(visible(cx, n.win, 5));
    assert_eq!(fades(cx, n.win), (true, true), "tabs hidden on both sides");
}

#[gpui_kit::test]
fn a_strip_that_fits_has_no_fades(cx: &mut TestAppContext) {
    let n = strip(cx, 1, 1000.);
    assert!(visible(cx, n.win, 0));
    assert_eq!(fades(cx, n.win), (false, false));
}

// The fades cover the scrolling tabs only: never the inset before them or the chevron after.
#[gpui_kit::test]
fn the_tab_strip_fades_sit_between_the_inset_and_the_chevron(cx: &mut TestAppContext) {
    let n = narrow(cx);
    n.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(5)));
    cx.run_until_parked();
    assert_eq!(fades(cx, n.win), (true, true));
    cx.update_window(n.win, |_, w, _| {
        let left = w.find("tab-strip-fade-left").bounds();
        let right = w.find("tab-strip-fade-right").bounds();
        let inset = w.find("tab-inset").bounds();
        let chevron = w.find("tab-overflow").bounds();
        assert!(
            left.size.width > px(0.) && left.size.height > px(0.),
            "{left:?}"
        );
        assert!(
            left.left() >= inset.right() - px(0.5),
            "{left:?} over {inset:?}"
        );
        assert!(
            right.right() <= chevron.left() + px(0.5),
            "{right:?} over {chevron:?}"
        );
        assert_eq!(left.size, right.size);
    })
    .unwrap();
}

// The fades take no mouse input: a click on a fade lands on the tab beneath it.
#[gpui_kit::test]
fn a_click_through_a_fade_activates_the_tab_beneath(cx: &mut TestAppContext) {
    let n = narrow(cx);
    n.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(0)));
    cx.run_until_parked();
    assert_eq!(fades(cx, n.win), (false, true));
    let (offset, under) = cx
        .update_window(n.win, |_, w, _| {
            let fade = w.find("tab-strip-fade-right").bounds();
            let at = fade.center();
            let under = (0..10usize)
                .find(|ix| w.find(("tab", *ix)).bounds().contains(&at))
                .expect("a tab lies under the right fade");
            (at - fade.origin, under)
        })
        .unwrap();
    assert_ne!(under, 0, "the tab beneath is not already active");
    cx.update_window(n.win, |_, w, cx| {
        w.click_at("tab-strip-fade-right", offset, cx)
    })
    .unwrap();
    cx.run_until_parked();
    let expected = format!("r{under}.yml");
    assert_eq!(active(cx, &n).as_deref(), Some(expected.as_str()));
}

// A wheel that scrolls the strip to its end, without choosing a tab, moves the fade with it.
#[gpui_kit::test]
fn wheeling_to_the_end_swaps_the_fades(cx: &mut TestAppContext) {
    let n = narrow(cx);
    n.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(0)));
    cx.run_until_parked();
    assert!(visible(cx, n.win, 0));
    assert_eq!(fades(cx, n.win), (false, true), "at the start");
    for _ in 0..20 {
        cx.update_window(n.win, |_, w, cx| {
            w.render_frame(cx);
            let down = gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-400.)));
            w.scroll("tab-strip", down, cx)
        })
        .unwrap();
    }
    assert!(visible(cx, n.win, 9), "the wheel reached the last tab");
    assert_eq!(fades(cx, n.win), (true, false), "at the end");
    assert_eq!(active(cx, &n).as_deref(), Some("r0.yml"));
}

// At 200% zoom, at any width, the active tab lies wholly between the inset and the overflow
// chevron when it fits, a tab cut by an edge is drawn with that edge's fade over it, and the
// strip stays filled.
#[gpui_kit::test]
fn at_double_zoom_no_request_tab_shows_clipped(cx: &mut TestAppContext) {
    let (_col, s) = double_zoom(cx, 6, 1280.);
    for width in [1280., 1000., 800., 640.] {
        gpui_kit::VisualTestContext::from_window(s.win, cx)
            .simulate_resize(size(px(width), px(800.)));
        frames(cx, s.win, 3);
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            let left = w.find("tab-inset").bounds().right();
            let right = w.find("tab-strip-viewport").bounds().right();
            let active = w.find(("tab", 5usize)).bounds();
            assert!(
                w.find(("tab", 5usize)).visible(),
                "{width} px: the active tab shows"
            );
            if active.size.width <= right - left {
                assert!(
                    active.left() >= left - px(0.5) && active.right() <= right + px(0.5),
                    "{width} px: the active tab {active:?} is cut between {left:?} and {right:?}"
                );
            }
            let cut_left =
                (0..6usize).any(|ix| w.find(("tab", ix)).bounds().left() < left - px(0.5));
            let cut_right =
                (0..6usize).any(|ix| w.find(("tab", ix)).bounds().right() > right + px(0.5));
            assert_eq!(
                w.try_find("tab-strip-fade-left").is_some(),
                cut_left,
                "{width} px: the left fade follows the left cut"
            );
            assert_eq!(
                w.try_find("tab-strip-fade-right").is_some(),
                cut_right,
                "{width} px: the right fade follows the right cut"
            );
        })
        .unwrap();
        assert_filled(cx, s.win, 6, &format!("{width} px"));
        assert_every_tab_drawn(cx, s.win, 6, &format!("{width} px"));
    }
}

// At 200% zoom in a 360 px window the active tab is wider than the strip; it is never cut, so
// the strip still shows it, label and underline, truncating its own name.
#[gpui_kit::test]
fn an_active_tab_wider_than_the_strip_still_shows(cx: &mut TestAppContext) {
    use gpui_kit::component::ActiveTheme as _;
    let (_col, s) = double_zoom(cx, 6, 360.);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        assert!(
            w.find(("tab", 5usize)).visible(),
            "the active tab's label shows"
        );
        // The underline is a 2 px accent quad left visible by its clip.
        let scale = w.scale_factor();
        let accent = cx.theme().primary;
        let underline = w.painted_quads().into_iter().any(|q| {
            let (b, m) = (q.bounds, q.content_mask.bounds);
            q.background.as_solid() == Some(accent)
                && (b.size.height.0 / scale - 2.).abs() < 0.1
                && b.origin.x.0.max(m.origin.x.0)
                    < (b.origin.x + b.size.width)
                        .0
                        .min((m.origin.x + m.size.width).0)
        });
        assert!(underline, "the active tab keeps its underline");
    })
    .unwrap();
}

// At 200% zoom in a 360 px window every tab is wider than the strip; wheeled part way, the
// tab or tabs reaching into the strip are all drawn and the strip is never blank.
#[gpui_kit::test]
fn a_wide_tab_wheeled_part_way_leaves_the_strip_filled(cx: &mut TestAppContext) {
    let (_col, s) = double_zoom(cx, 6, 360.);
    for step in 0..6 {
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            let up = gpui_kit::ScrollDelta::Pixels(point(px(0.), px(120.)));
            w.scroll("tab-strip", up, cx)
        })
        .unwrap();
        frames(cx, s.win, 3);
        let why = format!("step {step}");
        assert_filled(cx, s.win, 6, &why);
        assert_every_tab_drawn(cx, s.win, 6, &why);
    }
}

// At 200% zoom in a 640 px window the tabs are nearly as wide as the strip; a wheel that
// stops between two tabs still shows the one with the most in view, so the strip is never blank.
#[gpui_kit::test]
fn wheeling_a_double_zoom_strip_never_leaves_it_blank(cx: &mut TestAppContext) {
    use apinata_app::app::AppState;
    let names = [
        "List posts",
        "Get post",
        "Create post",
        "Replace post",
        "Delete post",
    ];
    let files: Vec<(String, String)> = names
        .iter()
        .enumerate()
        .map(|(i, n)| (format!("r{i}.yml"), request(n, "http://x/")))
        .collect();
    let mut refs: Vec<(&str, &str)> = vec![("opencollection.yml", ROOT)];
    refs.extend(files.iter().map(|(p, t)| (p.as_str(), t.as_str())));
    let col = write_tree(&refs);
    let s = started(cx, &col);
    for i in 0..5 {
        open_tab(cx, &s.ws, &s.root, &format!("r{i}.yml"));
    }
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| p.update(cx, |s| s.zoom = 2.0));
    });
    gpui_kit::VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(640.), px(720.)));
    frames(cx, s.win, 3);
    let mut reached_start = false;
    for step in 0..30 {
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            let up = gpui_kit::ScrollDelta::Lines(point(0., 1.));
            w.scroll("tab-strip", up, cx)
        })
        .unwrap();
        frames(cx, s.win, 2);
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            let shown: Vec<usize> = (0..5usize)
                .filter(|ix| w.find(("tab", *ix)).visible())
                .collect();
            assert!(!shown.is_empty(), "step {step}: the strip shows no tab");
            reached_start |= shown.contains(&0);
        })
        .unwrap();
    }
    assert!(reached_start, "the wheel still reaches the first tab");
}

// `count` request tabs at 200% zoom in a `width` px shell, the last one active.
fn double_zoom(cx: &mut TestAppContext, count: usize, width: f32) -> (tempfile::TempDir, Started) {
    shell_strip(cx, count, width, 2.0)
}

// `count` request tabs at `zoom` in a `width` px shell with its sidebar, the last one active.
fn shell_strip(
    cx: &mut TestAppContext,
    count: usize,
    width: f32,
    zoom: f32,
) -> (tempfile::TempDir, Started) {
    use apinata_app::app::AppState;
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
    gpui_kit::VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(width), px(800.)));
    frames(cx, s.win, 3);
    (col, s)
}

// Scrolls the tab strip by `delta` `steps` times, asserting after each that some tab shows.
fn wheel_never_blank(
    cx: &mut TestAppContext,
    s: &Started,
    count: usize,
    steps: usize,
    delta: gpui_kit::ScrollDelta,
) -> Vec<usize> {
    let mut seen = Vec::new();
    for step in 0..steps {
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.scroll("tab-strip", delta, cx)
        })
        .unwrap();
        frames(cx, s.win, 2);
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            let shown: Vec<usize> = (0..count)
                .filter(|ix| w.find(("tab", *ix)).visible())
                .collect();
            assert!(!shown.is_empty(), "step {step}: the strip shows no tab");
            seen.extend(shown);
        })
        .unwrap();
    }
    seen
}

// Wheeled forward from the first tab to past the end, a 360 px strip at 200% always shows a
// tab, the end filler beside the overflow menu included.
#[gpui_kit::test]
fn wheeling_a_double_zoom_strip_to_its_end_never_leaves_it_blank(cx: &mut TestAppContext) {
    let (_col, s) = double_zoom(cx, 4, 360.);
    s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(0)));
    frames(cx, s.win, 3);
    let down = gpui_kit::ScrollDelta::Lines(point(0., -1.));
    let seen = wheel_never_blank(cx, &s, 4, 80, down);
    assert!(seen.contains(&3), "the wheel reaches the last tab");
}

// A trackpad's sideways delta goes to the kit's own scroller, not the wheel handler; it
// never leaves the strip blank either.
#[gpui_kit::test]
fn a_sideways_trackpad_scroll_never_leaves_the_strip_blank(cx: &mut TestAppContext) {
    let (_col, s) = double_zoom(cx, 4, 360.);
    let back = gpui_kit::ScrollDelta::Pixels(point(px(37.), px(0.)));
    let seen = wheel_never_blank(cx, &s, 4, 60, back);
    assert!(seen.contains(&0), "the trackpad reaches the first tab");
}

// Scrolled to the active tab at 200% zoom, the active tab sits on the strip's left edge so no
// cut tab leaves blank room before it, until the strip's end stops it: the tabs after the
// active one then fill the strip to its end.
#[gpui_kit::test]
fn scrolled_to_the_active_tab_the_strip_leaves_no_gap_at_its_left(cx: &mut TestAppContext) {
    let (_col, s) = double_zoom(cx, 8, 800.);
    for active in [7usize, 4, 2, 0] {
        s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(active)));
        frames(cx, s.win, 6);
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            let left = w.find("tab-inset").bounds().left();
            let first = w.find(("tab", active)).bounds();
            let view = w.find("tab-strip-viewport").bounds().right();
            let last = w.find(("tab", 7usize)).bounds();
            let at_left = (first.left() - w.find("tab-inset").bounds().right()).abs() < px(1.);
            let at_end = last.right() > view - px(120.);
            assert!(
                at_left || at_end,
                "active {active}: neither on the left edge (from {left:?}) nor against the end"
            );
            assert!(w.find(("tab", active)).visible(), "active {active} shows");
        })
        .unwrap();
    }
}
// Seven tabs with a preview tab last and a collection Settings tab active (what a restore
// leaves): the strip never scrolls past its content, so while earlier tabs are hidden the
// last tab ends no earlier than it does when the strip is scrolled to its end.
#[gpui_kit::test]
fn a_restored_strip_with_a_last_preview_tab_leaves_no_trailing_space(cx: &mut TestAppContext) {
    use apinata_app::tabs::TabKind;
    let n = strip_opening(cx, 7, 5, 480.);
    let root = n.root.clone();
    n.ws.update(cx, |w, cx| w.open_settings(&root, TabKind::Collection, cx));
    n.ws.update(cx, |w, cx| w.open_item(&root, ip("r5.yml"), true, cx));
    cx.run_until_parked();
    let last_right = |cx: &mut TestAppContext, active: usize| {
        n.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(active)));
        frames(cx, n.win, 6);
        cx.update_window(n.win, |_, w, cx| {
            w.render_frame(cx);
            let hidden = !w.find(("tab", 0usize)).visible();
            (hidden, w.find(("tab", 6usize)).bounds().right())
        })
        .unwrap()
    };
    let (_, end) = last_right(cx, 6);
    let (hidden, right) = last_right(cx, 5);
    assert!(hidden, "earlier tabs are scrolled out of view");
    assert!(
        right >= end - px(1.),
        "the last tab ends {:?} before where the strip's end puts it",
        end - right
    );
}

// Eight tabs in 480 px (six requests, Settings, a preview request last). Activating a middle
// request tab at runtime, after the strip has laid out, puts it on the strip's left edge; the
// tabs after it do not all fit, so the last one is cut by a few pixels past the strip's right
// edge. The cut tab must still be drawn, so no blank room shows right of the active tab's
// neighbour.
#[gpui_kit::test]
fn activating_a_middle_tab_at_runtime_leaves_no_blank_trailing_space(cx: &mut TestAppContext) {
    use apinata_app::tabs::TabKind;
    let n = strip_opening(cx, 7, 6, 480.);
    let root = n.root.clone();
    n.ws.update(cx, |w, cx| w.open_settings(&root, TabKind::Collection, cx));
    n.ws.update(cx, |w, cx| w.open_item(&root, ip("r6.yml"), true, cx));
    frames(cx, n.win, 6);
    for (from, to) in [(0usize, 5usize), (6, 5), (0, 6), (7, 6)] {
        n.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(from)));
        frames(cx, n.win, 6);
        n.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(to)));
        frames(cx, n.win, 6);
        cx.update_window(n.win, |_, w, cx| {
            w.render_frame(cx);
            assert!(
                w.find(("tab", to)).visible(),
                "{from}->{to}: the active tab shows"
            );
        })
        .unwrap();
        assert_filled(cx, n.win, 8, &format!("{from}->{to}"));
        assert_every_tab_drawn(cx, n.win, 8, &format!("{from}->{to}"));
    }
}

// The same runtime reveal at 200% zoom, where each tab is twice as wide: six requests in a
// shell whose width varies by a few pixels, so the last tab passes the strip's edge by
// differing slivers; the strip stays filled and draws every tab it reaches into.
#[gpui_kit::test]
fn activating_a_middle_tab_at_runtime_leaves_no_blank_trailing_space_at_double_zoom(
    cx: &mut TestAppContext,
) {
    for width in [716., 740., 764., 788.] {
        let (_col, s) = double_zoom(cx, 6, width);
        for (from, to) in [(0usize, 4usize), (5, 4)] {
            s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(from)));
            frames(cx, s.win, 6);
            s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(to)));
            frames(cx, s.win, 6);
            cx.update_window(s.win, |_, w, cx| {
                w.render_frame(cx);
                assert!(
                    w.find(("tab", to)).visible(),
                    "{width} {from}->{to}: the active tab shows"
                );
            })
            .unwrap();
            let why = format!("{width} {from}->{to}");
            assert_filled(cx, s.win, 6, &why);
            assert_every_tab_drawn(cx, s.win, 6, &why);
        }
    }
}

// The strip's viewport, between the inset and the chevron, and the whole kit tab `ix`'s bounds
// (label and close slot).
fn view_and_tab(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    ix: usize,
) -> (gpui_kit::Pixels, gpui_kit::Pixels, Bounds<gpui_kit::Pixels>) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let left = w.find("tab-inset").bounds().right();
        let right = w.find("tab-strip-viewport").bounds().right();
        (left, right, w.within("tab-strip").find(ix).bounds())
    })
    .unwrap()
}

// Every tab with any part in the strip's viewport is drawn, a cut one included, so an edge
// never leaves blank room where a tab belongs.
fn assert_every_tab_drawn(cx: &mut TestAppContext, win: AnyWindowHandle, count: usize, why: &str) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let left = w.find("tab-inset").bounds().right();
        let right = w.find("tab-strip-viewport").bounds().right();
        for ix in 0..count {
            let b = w.find(("tab", ix)).bounds();
            let shown = b.right().min(right) - b.left().max(left);
            assert!(
                shown < px(1.) || w.find(("tab", ix)).visible(),
                "{why}: tab {ix} {b:?} reaches into {left:?}..{right:?} and is not drawn"
            );
        }
    })
    .unwrap();
}

// Whether tabs `0..count` cover the viewport from its left edge to its right one, allowing
// `slack` for the gaps between tabs and the strip's end filler: no blank room.
fn assert_filled(cx: &mut TestAppContext, win: AnyWindowHandle, count: usize, why: &str) {
    let slack = px(40.);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let (left, right) = (
            w.find("tab-inset").bounds().right(),
            w.find("tab-strip-viewport").bounds().right(),
        );
        let spans: Vec<_> = (0..count)
            .map(|ix| {
                let b = w.within("tab-strip").find(ix).bounds();
                (b.left(), b.right())
            })
            .collect();
        let mut x = left;
        while x < right {
            match spans.iter().find(|(l, r)| *l - slack <= x && x < *r) {
                Some((_, r)) => x = *r + px(1.),
                None => {
                    let end = spans.last().map_or(left, |s| s.1);
                    assert!(
                        x >= end,
                        "{why}: blank room at {x:?} in {left:?}..{right:?}"
                    );
                    assert!(end + slack >= right, "{why}: blank room after {end:?}");
                    break;
                }
            }
        }
    })
    .unwrap();
}

// Activating each tab in turn, forward then back: every tab stays drawn, a tab already whole
// in the strip does not move it unless another is cut at the left edge, a tab cut on the left
// lands on the left edge and one cut on the right on the right edge (the last tab only reaches
// the strip's end) unless a neighbour left cut is then scrolled whole or out, and the tabs
// still reach the strip's right end.
fn check_min_scroll(cx: &mut TestAppContext, width: f32, zoom: f32) {
    let count = 9;
    let (_col, s) = shell_strip(cx, count, width, zoom);
    let fade = fade_width(cx, s.win);
    let order: Vec<usize> = (0..count).chain((0..count).rev()).collect();
    for ix in order {
        let (view_left, view_right, before) = view_and_tab(cx, s.win, ix);
        // Whole means clear of the fades shown now, which the strip scrolls a tab out from under.
        let (fl, fr) = cx
            .update_window(s.win, |_, w, _| {
                (
                    w.try_find("tab-strip-fade-left").is_some(),
                    w.try_find("tab-strip-fade-right").is_some(),
                )
            })
            .unwrap();
        let whole = before.left() >= view_left + if fl { fade } else { px(0.) } - px(1.)
            && before.right() <= view_right - if fr { fade } else { px(0.) } + px(1.);
        let strip_before: Vec<_> = (0..count).map(|k| view_and_tab(cx, s.win, k).2).collect();
        // Whether the strip moved by `shift` from here leaves some tab cut at its left edge.
        let cut_after = |shift: gpui_kit::Pixels| {
            strip_before.iter().any(|b| {
                b.left() + shift < view_left - px(0.5) && b.right() + shift > view_left + px(0.5)
            })
        };
        s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(ix)));
        frames(cx, s.win, 6);
        let why = format!("{width} px, zoom {zoom}, tab {ix}");
        assert_every_tab_drawn(cx, s.win, count, &why);
        let (_, _, after) = view_and_tab(cx, s.win, ix);
        // The least scroll may leave a neighbour cut at the left edge; only then may the strip
        // land elsewhere, with that tab scrolled whole or out and the chosen one clear of the fades.
        let snap = |cx: &mut TestAppContext, least: gpui_kit::Pixels| {
            cut_after(least)
                && clear_of_fades(cx, s.win, ix, fade)
                && left_sliver(cx, s.win, count).is_none()
        };
        if whole && (after.left() - before.left()).abs() >= px(1.) {
            assert!(
                snap(cx, px(0.)),
                "{why}: a whole tab moved the strip by {:?} with no tab to snap",
                after.left() - before.left()
            );
        } else if whole {
            assert!(
                (after.left() - before.left()).abs() < px(1.),
                "{why}: a whole tab moved the strip by {:?}",
                after.left() - before.left()
            );
            let moved: Vec<_> = (0..count)
                .map(|k| view_and_tab(cx, s.win, k).2.left())
                .collect();
            for (k, (a, b)) in strip_before.iter().zip(&moved).enumerate() {
                assert!((a.left() - *b).abs() < px(1.), "{why}: tab {k} moved");
            }
        } else if before.left() < view_left + if fl { fade } else { px(0.) }
            || after.size.width > view_right - view_left
        {
            // Clears the left fade, except the first tab, which only reaches the strip's start.
            let want = if ix == 0 { view_left } else { view_left + fade };
            assert!(
                (after.left() - want).abs() < px(1.) || snap(cx, want - before.left()),
                "{why}: not clear of the left fade: {:?} vs {want:?}",
                after.left()
            );
        } else if ix + 1 < count {
            let want = view_right - fade;
            assert!(
                (after.right() - want).abs() < px(1.) || snap(cx, want - before.right()),
                "{why}: not clear of the right fade: {:?} vs {:?}",
                after.right(),
                view_right - fade
            );
        } else {
            assert!(after.right() <= view_right + px(1.), "{why}: past the end");
        }
        assert_filled(cx, s.win, count, &why);
    }
}

// The first tab cut by the strip's left edge, if any, even one the left fade covers.
fn left_sliver(cx: &mut TestAppContext, win: AnyWindowHandle, count: usize) -> Option<usize> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let view = w.find("tab-strip-viewport").bounds();
        (0..count).find(|&ix| {
            let b = w.within("tab-strip").find(ix).bounds();
            b.left() < view.left() - px(0.5) && b.right() > view.left() + px(0.5)
        })
    })
    .unwrap()
}

// Whether tab `ix` lies whole between the fades now shown.
fn clear_of_fades(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    ix: usize,
    fade: gpui_kit::Pixels,
) -> bool {
    let (left, right, tab) = view_and_tab(cx, win, ix);
    let (fl, fr) = cx
        .update_window(win, |_, w, _| {
            (
                w.try_find("tab-strip-fade-left").is_some(),
                w.try_find("tab-strip-fade-right").is_some(),
            )
        })
        .unwrap();
    tab.left() >= left + if fl { fade } else { px(0.) } - px(1.)
        && tab.right() <= right - if fr { fade } else { px(0.) } + px(1.)
}

#[gpui_kit::test]
fn activating_any_tab_scrolls_least_and_draws_every_tab_at_440(cx: &mut TestAppContext) {
    check_min_scroll(cx, 440., 1.0);
}

#[gpui_kit::test]
fn activating_any_tab_scrolls_least_and_draws_every_tab_at_480(cx: &mut TestAppContext) {
    check_min_scroll(cx, 480., 1.0);
}

#[gpui_kit::test]
fn activating_any_tab_scrolls_least_and_draws_every_tab_at_double_zoom(cx: &mut TestAppContext) {
    check_min_scroll(cx, 800., 2.0);
}

// The same through the overflow menu, which activates by index like a tab click.
#[gpui_kit::test]
fn the_overflow_menu_scrolls_least_and_draws_every_tab(cx: &mut TestAppContext) {
    let (_col, s) = shell_strip(cx, 9, 440., 1.0);
    for pick in [0usize, 3, 7] {
        cx.update_window(s.win, |_, w, cx| w.click("tab-overflow", cx))
            .unwrap();
        frame(cx, s.win);
        cx.update_window(s.win, |_, w, cx| w.within("popup-menu").click(pick, cx))
            .unwrap();
        frames(cx, s.win, 6);
        assert_every_tab_drawn(cx, s.win, 9, &format!("menu pick {pick}"));
        let (l, r, tab) = view_and_tab(cx, s.win, pick);
        assert!(
            tab.left() >= l - px(1.) && tab.right() <= r + px(1.),
            "menu pick {pick}: {tab:?} not within {l:?}..{r:?}"
        );
    }
}

// The fade width in pixels at the window's zoom.
fn fade_width(cx: &mut TestAppContext, win: AnyWindowHandle) -> gpui_kit::Pixels {
    cx.update_window(win, |_, w, _| {
        apinata_app::style::resolve(apinata_app::style::STRIP_FADE_WIDTH, w)
    })
    .unwrap()
}

// Wheels the tab strip `k` notches along as one event. The strip clamps its offset and
// nothing else moves it while it wheels, so this lands where `k` single notches would.
fn wheel_notches(cx: &mut TestAppContext, win: AnyWindowHandle, k: usize) {
    if k == 0 {
        return;
    }
    let down = gpui_kit::ScrollDelta::Lines(point(0., -(k as f32)));
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.scroll("tab-strip", down, cx)
    })
    .unwrap();
    frames(cx, win, 2);
}

// With each tab activated in turn, clicks along every other tab's close slot (the last
// row-height of its wrapper) that is not wholly outside both fades. Such a click never closes
// the tab; it selects it. The slot's own button may be hidden there, so the slot is
// measured from the tab's wrapper. Returns how many clicks were made; each width is its own
// test to keep tests short.
fn check_close_under_fade(cx: &mut TestAppContext, width: f32, zoom: f32) -> usize {
    let count = 9;
    let (_col, s) = shell_strip(cx, count, width, zoom);
    let slot_w = cx
        .update_window(s.win, |_, w, _| {
            apinata_app::style::resolve(apinata_app::style::ROW_HEIGHT, w)
        })
        .unwrap();
    let mut probed = 0;
    // Each round starts from the first tab and wheels the strip `k` notches along.
    for k in 0..14 {
        let mut n = 0;
        loop {
            s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(0)));
            frames(cx, s.win, 6);
            wheel_notches(cx, s.win, k);
            let why = format!("{width} px, zoom {zoom}, wheeled {k}, probe {n}");
            // The strip's fades and every cut slot with the points to click along it.
            let state = cx
                .update_window(s.win, |_, w, cx| {
                    w.render_frame(cx);
                    let fr = w.try_find("tab-strip-fade-right")?.bounds();
                    let fl = w.try_find("tab-strip-fade-left").map(|f| f.bounds());
                    let strip = w.find("tab-strip").bounds();
                    let inset = w.find("tab-inset").bounds().right();
                    let mut probes = Vec::new();
                    for ix in 0..count {
                        let b = w.within("tab-strip").find(ix).bounds();
                        let (l, r) = (b.right() - slot_w, b.right());
                        let clean =
                            r <= fr.left() + px(1.) && fl.is_none_or(|f| l >= f.right() - px(1.));
                        let (from, to) = (l.max(inset), r.min(fr.right() - px(2.)));
                        let mut x = from + px(1.);
                        while !clean && x < to {
                            probes.push((ix, x));
                            x += px(8.);
                        }
                    }
                    Some((strip, probes))
                })
                .unwrap();
            let Some((strip, probes)) = state else { break };
            let Some(&(ix, x)) = probes.get(n) else { break };
            n += 1;
            probed += 1;
            let before = s.ws.read_with(cx, |w, _| w.tabs().entries().len());
            cx.update_window(s.win, |_, w, cx| {
                w.click_at("tab-strip", point(x, strip.center().y) - strip.origin, cx)
            })
            .unwrap();
            frames(cx, s.win, 3);
            let after = s.ws.read_with(cx, |w, _| w.tabs().entries().len());
            assert_eq!(
                before, after,
                "{why}: a click at {x:?} on tab {ix}'s cut close slot closed a tab"
            );
            let got = s.ws.read_with(cx, |w, _| {
                w.tabs()
                    .active()
                    .and_then(|e| e.item())
                    .map(|p| p.as_str().to_owned())
            });
            let want = format!("r{ix}.yml");
            assert_eq!(
                got.as_deref(),
                Some(want.as_str()),
                "{why}: a click at {x:?} on tab {ix}'s cut close slot did not select it"
            );
        }
    }
    probed
}

#[gpui_kit::test]
fn a_cut_close_button_under_a_fade_selects_its_tab_at_440(cx: &mut TestAppContext) {
    assert!(
        check_close_under_fade(cx, 440., 1.0) > 0,
        "no close slot lay under a fade"
    );
}

#[gpui_kit::test]
fn a_cut_close_button_under_a_fade_selects_its_tab_at_double_zoom_500(cx: &mut TestAppContext) {
    assert!(
        check_close_under_fade(cx, 500., 2.0) > 0,
        "no close slot lay under a fade"
    );
}

#[gpui_kit::test]
fn a_cut_close_button_under_a_fade_selects_its_tab_at_double_zoom_560(cx: &mut TestAppContext) {
    assert!(
        check_close_under_fade(cx, 560., 2.0) > 0,
        "no close slot lay under a fade"
    );
}

#[gpui_kit::test]
fn a_cut_close_button_under_a_fade_selects_its_tab_at_double_zoom_640(cx: &mut TestAppContext) {
    assert!(
        check_close_under_fade(cx, 640., 2.0) > 0,
        "no close slot lay under a fade"
    );
}

#[gpui_kit::test]
fn a_cut_close_button_under_a_fade_selects_its_tab_at_double_zoom_720(cx: &mut TestAppContext) {
    assert!(
        check_close_under_fade(cx, 720., 2.0) > 0,
        "no close slot lay under a fade"
    );
}

// Nothing of the strip is painted past the overflow button's left edge: every quad the strip
// draws is clipped to end there, with the first tab at scroll 0 and with each tab revealed.
fn check_clear_of_overflow(cx: &mut TestAppContext, width: f32, zoom: f32) {
    let (_col, s) = shell_strip(cx, 9, width, zoom);
    for ix in [0usize, 4, 8, 0] {
        s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(ix)));
        frames(cx, s.win, 6);
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            let chevron = w.find("tab-overflow").bounds();
            let strip = w.find("tab-strip").bounds();
            let scale = w.scale_factor();
            for q in w.painted_quads() {
                let (b, m) = (q.bounds, q.content_mask.bounds);
                let (top, bottom) = (b.origin.y.0 / scale, (b.origin.y + b.size.height).0 / scale);
                if top < f32::from(strip.top()) || bottom > f32::from(strip.bottom()) + 0.5 {
                    continue;
                }
                let shown_right = (b.origin.x + b.size.width).0.min((m.origin.x + m.size.width).0) / scale;
                let shown_left = b.origin.x.0.max(m.origin.x.0) / scale;
                // The chevron's own quads and the full-width bar rules start at or before it.
                if shown_left >= f32::from(chevron.left()) - 0.5 || b.size.width.0 / scale >= f32::from(strip.size.width) - 1. {
                    continue;
                }
                assert!(
                    shown_right <= f32::from(chevron.left()) + 0.5,
                    "{width} px, zoom {zoom}, tab {ix}: a quad shows to {shown_right} past the chevron at {:?}",
                    chevron.left()
                );
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn no_tab_is_drawn_under_the_overflow_button_at_440(cx: &mut TestAppContext) {
    check_clear_of_overflow(cx, 440., 1.0);
}

#[gpui_kit::test]
fn no_tab_is_drawn_under_the_overflow_button_at_double_zoom(cx: &mut TestAppContext) {
    check_clear_of_overflow(cx, 440., 2.0);
}

#[derive(Clone, Copy, PartialEq)]
enum Under {
    Right,
    Left,
    Neither,
}

// For each wheel position, clicks a tab that lies under a shown fade on `kind`'s side (or
// wholly between the fades), then checks the tab is clear of both shown fades, or has not
// moved when it already was. Returns how many clicks were made. Each width is its own test
// to keep tests short; the 440 px strip always has such a tab, so it asserts a click.
fn check_click_under_fade(cx: &mut TestAppContext, width: f32, zoom: f32, kind: Under) -> usize {
    let count = 9;
    let (_col, s) = shell_strip(cx, count, width, zoom);
    let fade = fade_width(cx, s.win);
    let mut clicked = 0;
    for k in 0..14 {
        s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(0)));
        frames(cx, s.win, 6);
        wheel_notches(cx, s.win, k);
        // The edges of the shown fades as (left fade's right edge, right fade's left edge).
        let edges = |cx: &mut TestAppContext| {
            cx.update_window(s.win, |_, w, cx| {
                w.render_frame(cx);
                let strip = w.find("tab-strip").bounds();
                let fl = w
                    .try_find("tab-strip-fade-left")
                    .map(|f| f.bounds().right());
                let fr = w
                    .try_find("tab-strip-fade-right")
                    .map(|f| f.bounds().left());
                (strip, fl, fr)
            })
            .unwrap()
        };
        let tab = |cx: &mut TestAppContext, ix: usize| {
            cx.update_window(s.win, |_, w, _| w.within("tab-strip").find(ix).bounds())
                .unwrap()
        };
        let view = {
            let (l, r, _) = view_and_tab(cx, s.win, 0);
            (l, r)
        };
        let (strip, fl, fr) = edges(cx);
        let (lo, hi) = (fl.unwrap_or(strip.left()), fr.unwrap_or(strip.right()));
        let pick = (0..count).find(|&ix| {
            let b = tab(cx, ix);
            let (l, r) = (b.left(), b.right());
            // A tab that cannot fit between the fades, or sits at a strip end it cannot
            // scroll past, has no clear position.
            let end = (kind == Under::Right && ix == 0) || (kind == Under::Left && ix + 1 == count);
            if end || b.size.width > view.1 - view.0 - fade * 2. {
                return false;
            }
            match kind {
                Under::Right => {
                    fr.is_some() && r > hi + px(1.) && r < hi + px(24.) && l < hi - px(8.)
                }
                Under::Left => {
                    fl.is_some() && l < lo - px(1.) && l > lo - px(24.) && r > lo + px(8.)
                }
                Under::Neither => l >= lo && r <= hi,
            }
        });
        let Some(ix) = pick else { continue };
        let before = tab(cx, ix);
        let sliver_before = left_sliver(cx, s.win, count);
        let b = before;
        let x = (b.left().max(lo) + b.right().min(hi)) / 2.;
        cx.update_window(s.win, |_, w, cx| {
            w.click_at("tab-strip", point(x, strip.center().y) - strip.origin, cx)
        })
        .unwrap();
        frames(cx, s.win, 6);
        clicked += 1;
        let why = format!("{width} px, zoom {zoom}, wheeled {k}, tab {ix}");
        let (_, fl, fr) = edges(cx);
        let after = tab(cx, ix);
        // A whole tab stays put unless the wheel left another cut at the left edge, which the
        // choice then scrolls whole or out of view.
        if kind == Under::Neither && (after.left() - before.left()).abs() >= px(1.) {
            assert!(
                sliver_before.is_some() && left_sliver(cx, s.win, count).is_none(),
                "{why}: a tab already clear of the fades moved"
            );
        }
        if let Some(f) = fl {
            assert!(
                after.left() >= f - px(1.5),
                "{why}: left edge still under the left fade: {:?} vs {f:?}, before {:?}, strip {:?}",
                after.left(),
                before.left(),
                strip
            );
        }
        if let Some(f) = fr {
            assert!(
                after.right() <= f + px(1.5),
                "{why}: right edge still under the right fade"
            );
        }
    }
    clicked
}

#[gpui_kit::test]
fn a_tab_whose_right_edge_lies_under_a_fade_scrolls_clear_at_440(cx: &mut TestAppContext) {
    assert!(
        check_click_under_fade(cx, 440., 1.0, Under::Right) > 0,
        "no tab had its right edge under a fade"
    );
}

#[gpui_kit::test]
fn a_tab_whose_right_edge_lies_under_a_fade_scrolls_clear_at_double_zoom_500(
    cx: &mut TestAppContext,
) {
    check_click_under_fade(cx, 500., 2.0, Under::Right);
}

#[gpui_kit::test]
fn a_tab_whose_right_edge_lies_under_a_fade_scrolls_clear_at_double_zoom_560(
    cx: &mut TestAppContext,
) {
    check_click_under_fade(cx, 560., 2.0, Under::Right);
}

#[gpui_kit::test]
fn a_tab_whose_right_edge_lies_under_a_fade_scrolls_clear_at_double_zoom_640(
    cx: &mut TestAppContext,
) {
    check_click_under_fade(cx, 640., 2.0, Under::Right);
}

#[gpui_kit::test]
fn a_tab_whose_right_edge_lies_under_a_fade_scrolls_clear_at_double_zoom_720(
    cx: &mut TestAppContext,
) {
    check_click_under_fade(cx, 720., 2.0, Under::Right);
}

#[gpui_kit::test]
fn a_tab_whose_left_edge_lies_under_a_fade_scrolls_clear_at_440(cx: &mut TestAppContext) {
    assert!(
        check_click_under_fade(cx, 440., 1.0, Under::Left) > 0,
        "no tab had its left edge under a fade"
    );
}

#[gpui_kit::test]
fn a_tab_whose_left_edge_lies_under_a_fade_scrolls_clear_at_double_zoom_500(
    cx: &mut TestAppContext,
) {
    check_click_under_fade(cx, 500., 2.0, Under::Left);
}

#[gpui_kit::test]
fn a_tab_whose_left_edge_lies_under_a_fade_scrolls_clear_at_double_zoom_560(
    cx: &mut TestAppContext,
) {
    check_click_under_fade(cx, 560., 2.0, Under::Left);
}

#[gpui_kit::test]
fn a_tab_whose_left_edge_lies_under_a_fade_scrolls_clear_at_double_zoom_640(
    cx: &mut TestAppContext,
) {
    check_click_under_fade(cx, 640., 2.0, Under::Left);
}

#[gpui_kit::test]
fn a_tab_whose_left_edge_lies_under_a_fade_scrolls_clear_at_double_zoom_720(
    cx: &mut TestAppContext,
) {
    check_click_under_fade(cx, 720., 2.0, Under::Left);
}

#[gpui_kit::test]
fn a_tab_clear_of_the_fades_does_not_move_when_clicked_at_440(cx: &mut TestAppContext) {
    assert!(
        check_click_under_fade(cx, 440., 1.0, Under::Neither) > 0,
        "no tab lay between the fades"
    );
}

#[gpui_kit::test]
fn a_tab_clear_of_the_fades_does_not_move_when_clicked_at_double_zoom_500(cx: &mut TestAppContext) {
    check_click_under_fade(cx, 500., 2.0, Under::Neither);
}

#[gpui_kit::test]
fn a_tab_clear_of_the_fades_does_not_move_when_clicked_at_double_zoom_560(cx: &mut TestAppContext) {
    check_click_under_fade(cx, 560., 2.0, Under::Neither);
}

#[gpui_kit::test]
fn a_tab_clear_of_the_fades_does_not_move_when_clicked_at_double_zoom_640(cx: &mut TestAppContext) {
    check_click_under_fade(cx, 640., 2.0, Under::Neither);
}

#[gpui_kit::test]
fn a_tab_clear_of_the_fades_does_not_move_when_clicked_at_double_zoom_720(cx: &mut TestAppContext) {
    check_click_under_fade(cx, 720., 2.0, Under::Neither);
}

// The All tabs menu leads each request with the method tag its tab shows.
#[gpui_kit::test]
fn the_overflow_menu_shows_each_requests_method(cx: &mut TestAppContext) {
    let n = strip(cx, 3, 360.);
    cx.update_window(n.win, |_, w, cx| w.click("tab-overflow", cx))
        .unwrap();
    frame(cx, n.win);
    cx.update_window(n.win, |_, w, _| {
        for ix in 0..3usize {
            let tag = w.find(("tab-menu-method", ix));
            assert_eq!(tag.label(), Some("GET"), "row {ix}");
            let item = w.find(("tab-menu-item", ix));
            assert!(tag.bounds().right() <= item.bounds().right(), "row {ix}");
        }
    })
    .unwrap();
}
