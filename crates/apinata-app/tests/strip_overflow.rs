//! Overflowing tab strips keep every tab reachable: the chosen tab scrolls into view and a
//! menu lists all of them.

use crate::support;

use apinata_app::command::{Command, Edit, Target};
use apinata_app::request_editor::Section;
use apinata_app::response_pane::{ResponsePane, ResponseTab};
use apinata_core::KeyValue;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, ElementId, TestAppContext, px, size};
use support::*;

// Two frames: the strip measures its overflow in one and draws the menu in the next.
fn settle(cx: &mut TestAppContext, win: AnyWindowHandle) {
    frame(cx, win);
    frame(cx, win);
}

fn inside(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    item: impl Into<ElementId>,
    strip: &'static str,
) -> bool {
    let item = item.into();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let s = w.find(strip).bounds();
        let b = w.find(item).bounds();
        b.left() >= s.left() - px(0.5) && b.right() <= s.right() + px(0.5)
    })
    .unwrap()
}

fn pick(cx: &mut TestAppContext, win: AnyWindowHandle, menu: &'static str, ix: usize) {
    cx.update_window(win, |_, w, cx| w.click(menu, cx)).unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.within("popup-menu").click(ix, cx))
        .unwrap();
    settle(cx, win);
}

#[gpui_kit::test]
fn a_narrow_section_strip_reaches_every_section(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.simulate_window_resize(win, size(px(1400.), px(700.)));
    settle(cx, win);
    assert!(
        !present(cx, win, "section-overflow"),
        "a wide strip has no menu"
    );
    cx.simulate_window_resize(win, size(px(360.), px(700.)));
    settle(cx, win);
    assert!(present(cx, win, "section-overflow"));
    assert!(
        !inside(
            cx,
            win,
            ("section", Section::Settings as usize),
            "section-strip"
        ),
        "Settings starts clipped"
    );
    pick(cx, win, "section-overflow", Section::Settings as usize);
    assert_eq!(ed.read_with(cx, |e, _| e.section()), Section::Settings);
    assert!(
        inside(
            cx,
            win,
            ("section", Section::Settings as usize),
            "section-strip"
        ),
        "the chosen section scrolls into view"
    );
}

#[gpui_kit::test]
fn a_narrow_response_strip_reaches_tests(cx: &mut TestAppContext) {
    let s = send_once(cx, wiremock::ResponseTemplate::new(200), "");
    support::init(cx);
    let model = s.model.clone();
    let (win, pane) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    cx.simulate_window_resize(win, size(px(1400.), px(600.)));
    settle(cx, win);
    assert!(
        !present(cx, win, "response-tab-overflow"),
        "a wide strip has no menu"
    );
    cx.simulate_window_resize(win, size(px(300.), px(600.)));
    settle(cx, win);
    assert!(present(cx, win, "response-tab-overflow"));
    pick(
        cx,
        win,
        "response-tab-overflow",
        ResponseTab::Tests as usize,
    );
    assert_eq!(pane.read_with(cx, |p, _| p.tab()), ResponseTab::Tests);
    assert!(
        inside(
            cx,
            win,
            ("response-tab", ResponseTab::Tests as usize),
            "response-tab-strip"
        ),
        "Tests scrolls into view"
    );
}

// A horizontal wheel over the section strip scrolls it to the clipped sections; a vertical
// one belongs to the request column (see request_scroll_cue).
#[gpui_kit::test]
fn a_horizontal_wheel_scrolls_the_section_strip(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.simulate_window_resize(win, size(px(360.), px(700.)));
    settle(cx, win);
    let last = ("section", Section::Settings as usize);
    assert!(
        !inside(cx, win, last, "section-strip"),
        "Settings starts clipped"
    );
    for _ in 0..10 {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let down = gpui_kit::ScrollDelta::Lines(gpui_kit::point(-3., 0.));
            w.scroll("section-strip", down, cx)
        })
        .unwrap();
    }
    settle(cx, win);
    assert!(
        inside(cx, win, last, "section-strip"),
        "wheeling down scrolled Settings into view"
    );
    assert_eq!(
        ed.read_with(cx, |e, _| e.section()),
        Section::Params,
        "the wheel scrolls without choosing a section"
    );
}

// Which of `strip`'s edge fades (`<strip>-fade-left`, `<strip>-fade-right`) show.
fn fades(cx: &mut TestAppContext, win: AnyWindowHandle, strip: &str) -> (bool, bool) {
    settle(cx, win);
    let (left, right) = (format!("{strip}-fade-left"), format!("{strip}-fade-right"));
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        (w.try_find(left).is_some(), w.try_find(right).is_some())
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_section_strip_fades_toward_its_hidden_sections(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.simulate_window_resize(win, size(px(1400.), px(700.)));
    assert_eq!(
        fades(cx, win, "section-strip"),
        (false, false),
        "a wide strip"
    );
    cx.simulate_window_resize(win, size(px(360.), px(700.)));
    assert_eq!(
        fades(cx, win, "section-strip"),
        (false, true),
        "at the start"
    );
    pick(cx, win, "section-overflow", Section::Settings as usize);
    assert_eq!(fades(cx, win, "section-strip"), (true, false), "at the end");
    cx.update_window(win, |_, w, _| {
        let fade = w.find("section-strip-fade-left").bounds();
        let strip = w.find("section-strip").bounds();
        assert!(fade.size.width > px(0.), "{fade:?}");
        assert!(
            fade.left() >= strip.left() - px(0.5),
            "{fade:?} in {strip:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_response_strip_fades_toward_its_hidden_tabs(cx: &mut TestAppContext) {
    let s = send_once(cx, wiremock::ResponseTemplate::new(200), "");
    support::init(cx);
    let model = s.model.clone();
    let (win, _pane) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    cx.simulate_window_resize(win, size(px(1400.), px(600.)));
    assert_eq!(
        fades(cx, win, "response-tab-strip"),
        (false, false),
        "a wide strip"
    );
    cx.simulate_window_resize(win, size(px(300.), px(600.)));
    assert_eq!(
        fades(cx, win, "response-tab-strip"),
        (false, true),
        "at the start"
    );
    pick(
        cx,
        win,
        "response-tab-overflow",
        ResponseTab::Console as usize,
    );
    assert_eq!(
        fades(cx, win, "response-tab-strip"),
        (true, false),
        "at the end"
    );
    cx.update_window(win, |_, w, _| {
        let fade = w.find("response-tab-strip-fade-left").bounds();
        let strip = w.find("response-tab-strip").bounds();
        assert!(fade.size.width > px(0.), "{fade:?}");
        assert!(
            fade.left() >= strip.left() - px(0.5),
            "{fade:?} in {strip:?}"
        );
    })
    .unwrap();
}

// A vertical wheel over the response strip scrolls it sideways to the clipped tabs, as the
// request tab strip and the section strip do; the chevron stays on offer meanwhile.
#[gpui_kit::test]
fn a_vertical_wheel_scrolls_the_response_strip(cx: &mut TestAppContext) {
    let s = send_once(cx, wiremock::ResponseTemplate::new(200), "");
    support::init(cx);
    let model = s.model.clone();
    let (win, pane) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    cx.simulate_window_resize(win, size(px(300.), px(600.)));
    settle(cx, win);
    let last = ("response-tab", ResponseTab::Console as usize);
    assert!(present(cx, win, "response-tab-overflow"));
    assert!(
        !inside(cx, win, last, "response-tab-strip"),
        "Console starts clipped"
    );
    for _ in 0..10 {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let down = gpui_kit::ScrollDelta::Lines(gpui_kit::point(0., -3.));
            w.scroll("response-tab-strip", down, cx)
        })
        .unwrap();
    }
    settle(cx, win);
    assert!(
        inside(cx, win, last, "response-tab-strip"),
        "wheeling down scrolled Console into view"
    );
    assert_eq!(
        pane.read_with(cx, |p, _| p.tab()),
        ResponseTab::Body,
        "the wheel scrolls without choosing a tab"
    );
    assert!(present(cx, win, "response-tab-overflow"));
}

// A tab cut by the strip's edge still draws its label, clipped under the fade; whole tabs
// draw theirs too, so no blank room opens beside the overflow menu.
#[gpui_kit::test]
fn a_partly_clipped_response_tab_keeps_its_label(cx: &mut TestAppContext) {
    let s = send_once(cx, wiremock::ResponseTemplate::new(200), "");
    support::init(cx);
    let model = s.model.clone();
    let (win, _pane) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    let console = ("response-tab", ResponseTab::Console as usize);
    let tests = ("response-tab", ResponseTab::Tests as usize);
    // Whether the label is drawn, and whether it reaches past the strip's viewport.
    let state = |cx: &mut TestAppContext, id: (&'static str, usize)| {
        settle(cx, win);
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let strip = w.find("response-tab-strip").bounds();
            let right = w.find("response-tab-strip-viewport").bounds().right();
            let label = w.find(id);
            let b = label.bounds();
            let cut = b.left() < strip.left() - px(0.5) || b.right() > right + px(0.5);
            (label.visible(), cut)
        })
        .unwrap()
    };
    cx.simulate_window_resize(win, size(px(1400.), px(600.)));
    assert_eq!(
        state(cx, console),
        (true, false),
        "a wide strip cuts nothing"
    );
    cx.simulate_window_resize(win, size(px(380.), px(600.)));
    assert_eq!(state(cx, console), (true, true), "Console is cut and drawn");
    assert!(state(cx, tests).0, "a whole tab keeps its label");
}

// Renders once, then only the frames the views ask for, as a window with a frame loop does.
fn requested_frames(cx: &mut TestAppContext, win: AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        for _ in 0..6 {
            if w.simulate_next_frame(cx) == 0 {
                break;
            }
            w.render_frame(cx);
        }
    })
    .unwrap();
}

// Whether any section label reaches past the strip's left edge or its viewport's right edge.
fn section_cut(cx: &mut TestAppContext, win: AnyWindowHandle) -> bool {
    cx.update_window(win, |_, w, _| {
        let strip = w.find("section-strip").bounds();
        let right = w.find("section-strip-viewport").bounds().right();
        (0..Section::ALL.len()).any(|ix| {
            let b = w.find(("section", ix)).bounds();
            b.left() < strip.left() - px(0.5) || b.right() > right + px(0.5)
        })
    })
    .unwrap()
}

fn headers(n: usize) -> Command {
    Command::Edit {
        target: Target::Request(ip("a.yml")),
        edit: Edit::Headers(
            (0..n)
                .map(|i| KeyValue::new(format!("H{i}"), "v"))
                .collect(),
        ),
    }
}

// Counts that widen a tab move the tabs after it without moving the strip's edges; a tab cut
// by an edge still draws its label. (Frames come freely in the harness.)
#[gpui_kit::test]
fn a_section_tab_cut_by_a_wider_neighbour_keeps_its_label(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let mut cut = false;
    for width in [300., 320., 340., 360., 380., 400., 420.] {
        ws.update(cx, |w, cx| w.send(&root, headers(0), cx));
        cx.simulate_window_resize(win, size(px(width), px(700.)));
        settle(cx, win);
        ws.update(cx, |w, cx| w.send(&root, headers(12), cx));
        requested_frames(cx, win);
        cut |= section_cut(cx, win);
        cx.update_window(win, |_, w, _| {
            let strip = w.find("section-strip").bounds();
            let right = w.find("section-strip-viewport").bounds().right();
            for ix in 0..Section::ALL.len() {
                let b = w.find(("section", ix)).bounds();
                let shown = b.right().min(right) - b.left().max(strip.left());
                assert!(
                    shown < px(1.) || w.find(("section", ix)).visible(),
                    "{width} px: section {ix} {b:?} reaches into the strip and is not drawn"
                );
            }
        })
        .unwrap();
    }
    assert!(cut, "some narrow strip cut a tab");
}

// The accent underline segments the last frame shows inside `strip`, as (left, right) in px:
// 2 px tall accent quads, cut to their clip.
fn underlines(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    strip: &'static str,
) -> Vec<(f32, f32)> {
    use gpui_kit::component::ActiveTheme as _;
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let s = w.find(strip).bounds();
        let scale = w.scale_factor();
        let accent = cx.theme().primary;
        w.painted_quads()
            .into_iter()
            .filter_map(|q| {
                let height = q.bounds.size.height.0 / scale;
                if q.background.as_solid() != Some(accent) || (height - 2.).abs() > 0.1 {
                    return None;
                }
                let (b, m) = (q.bounds, q.content_mask.bounds);
                let left = b.origin.x.0.max(m.origin.x.0) / scale;
                let right =
                    (b.origin.x.0 + b.size.width.0).min(m.origin.x.0 + m.size.width.0) / scale;
                let top = b.origin.y.0 / scale;
                let inside = top >= f32::from(s.top()) - 0.5 && top <= f32::from(s.bottom()) + 0.5;
                (right - left > 0.01 && inside).then_some((left, right))
            })
            .collect()
    })
    .unwrap()
}

// The selected tab, wheeled partly past the strip's edge, keeps its label and paints its
// underline clipped to the strip.
#[gpui_kit::test]
fn a_cut_selected_response_tab_paints_a_clipped_underline(cx: &mut TestAppContext) {
    let s = send_once(cx, wiremock::ResponseTemplate::new(200), "");
    support::init(cx);
    let model = s.model.clone();
    let (win, pane) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    cx.simulate_window_resize(win, size(px(300.), px(600.)));
    settle(cx, win);
    pane.update(cx, |p, cx| p.set_tab(ResponseTab::Console, cx));
    settle(cx, win);
    assert!(
        !underlines(cx, win, "response-tab-strip").is_empty(),
        "the revealed Console tab shows its underline"
    );
    let console = ("response-tab", ResponseTab::Console as usize);
    let mut partly = false;
    for _ in 0..80 {
        cx.update_window(win, |_, w, cx| {
            let up = gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(6.)));
            w.scroll("response-tab-strip", up, cx)
        })
        .unwrap();
        settle(cx, win);
        let (shows, label, strip, right) = cx
            .update_window(win, |_, w, cx| {
                w.render_frame(cx);
                let strip = w.find("response-tab-strip").bounds();
                let right = w.find("response-tab-strip-viewport").bounds().right();
                let label = w.find(console);
                (label.visible(), label.bounds(), strip, right)
            })
            .unwrap();
        if label.left() >= right {
            break;
        }
        if label.right() > strip.left() {
            assert!(shows, "the cut Console tab keeps its label");
            partly |= label.left() < strip.left() || label.right() > right;
            let lines = underlines(cx, win, "response-tab-strip");
            assert!(!lines.is_empty(), "the selected tab keeps an underline");
            for (l, r) in lines {
                assert!(
                    l >= f32::from(strip.left()) - 0.5 && r <= f32::from(strip.right()) + 0.5,
                    "the underline {l}..{r} stays inside the strip {strip:?}"
                );
            }
        }
    }
    assert!(partly, "the wheel passed a partly cut Console tab");
}

// In a strip narrower than two sections, a wheel that stops between sections still shows the
// one with the most in view, so the strip is never blank.
#[gpui_kit::test]
fn wheeling_a_tight_section_strip_never_leaves_it_blank(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.simulate_window_resize(win, size(px(150.), px(700.)));
    settle(cx, win);
    let mut reached_end = false;
    for step in 0..40 {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let down = gpui_kit::ScrollDelta::Lines(gpui_kit::point(-1., 0.));
            w.scroll("section-strip", down, cx)
        })
        .unwrap();
        settle(cx, win);
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let strip = w.find("section-strip").bounds();
            let right = w.find("section-strip-viewport").bounds().right();
            let shown: Vec<usize> = (0..Section::ALL.len())
                .filter(|ix| {
                    let b = w.find(("section", *ix)).bounds();
                    b.right().min(right) > b.left().max(strip.left()) + px(1.)
                })
                .collect();
            assert!(!shown.is_empty(), "step {step}: the strip shows no section");
            reached_end |= shown.contains(&(Section::Settings as usize));
        })
        .unwrap();
    }
    assert!(reached_end, "the wheel still reaches Settings");
}

// Whether `item` lies wholly inside the tab viewport (`<strip>-viewport`) of `strip`.
fn in_viewport(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    item: impl Into<ElementId>,
    strip: &'static str,
) -> (bool, String) {
    let item = item.into();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let v = w.find(format!("{strip}-viewport")).bounds();
        let b = w.find(item).bounds();
        let inside = b.left() >= v.left() - px(0.5) && b.right() <= v.right() + px(0.5);
        (inside, format!("{b:?} in {v:?}"))
    })
    .unwrap()
}

// Tab onto a section hidden past the strip's right edge scrolls the strip to show it.
#[gpui_kit::test]
fn tab_onto_a_hidden_section_scrolls_it_into_view(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.simulate_window_resize(win, size(px(360.), px(700.)));
    settle(cx, win);
    let last = Section::Settings as usize;
    let item = ("section", last);
    assert!(
        !in_viewport(cx, win, item, "section-strip").0,
        "Settings starts hidden"
    );
    let before = ed.read_with(cx, |e, _| e.section_focus(last - 1));
    cx.update_window(win, |_, w, cx| before.focus(w, cx))
        .unwrap();
    cx.simulate_keystrokes(win, "tab");
    cx.run_until_parked();
    let focus = ed.read_with(cx, |e, _| e.section_focus(last));
    assert!(
        cx.update_window(win, |_, w, _| focus.is_focused(w))
            .unwrap()
    );
    settle(cx, win);
    let (inside, at) = in_viewport(cx, win, item, "section-strip");
    assert!(inside, "the focused Settings tab scrolls into view: {at}");
    assert_eq!(ed.read_with(cx, |e, _| e.section()), Section::Params);
}

#[gpui_kit::test]
fn tab_onto_a_hidden_response_tab_scrolls_it_into_view(cx: &mut TestAppContext) {
    let s = send_once(cx, wiremock::ResponseTemplate::new(200), "");
    support::init(cx);
    let model = s.model.clone();
    let (win, pane) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    cx.simulate_window_resize(win, size(px(300.), px(600.)));
    settle(cx, win);
    let item = ("response-tab", ResponseTab::Tests as usize);
    assert!(
        !in_viewport(cx, win, item, "response-tab-strip").0,
        "Tests starts hidden"
    );
    let before = pane.read_with(cx, |p, _| p.tab_focus(ResponseTab::Timeline));
    cx.update_window(win, |_, w, cx| before.focus(w, cx))
        .unwrap();
    cx.simulate_keystrokes(win, "tab");
    cx.run_until_parked();
    let focus = pane.read_with(cx, |p, _| p.tab_focus(ResponseTab::Tests));
    assert!(
        cx.update_window(win, |_, w, _| focus.is_focused(w))
            .unwrap()
    );
    settle(cx, win);
    let (inside, at) = in_viewport(cx, win, item, "response-tab-strip");
    assert!(inside, "the focused Tests tab scrolls into view: {at}");
    assert_eq!(pane.read_with(cx, |p, _| p.tab()), ResponseTab::Body);
}
