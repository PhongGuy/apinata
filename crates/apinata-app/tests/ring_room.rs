//! Room for the kit's keyboard ring around controls that sit at a clipping edge, and the
//! app's accent ring width across zoom levels.

use crate::support::*;

use apinata_app::request_editor::Section;
use apinata_app::style::FOCUS_RING_ROOM;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, px};

// The section body scrolls, so it clips on every side; the Body select is its first control.
#[gpui_kit::test]
fn the_body_select_keeps_its_ring_inside_the_section_body(cx: &mut TestAppContext) {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x")),
    ]);
    let (_state, ws, root) = open_dir(cx, &dir);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    ed.update(cx, |e, cx| e.show(Section::Body, cx));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let clip = w.find("section-body").bounds();
        let b = w.find("body-mode").bounds();
        assert!(b.top() - FOCUS_RING_ROOM >= clip.top(), "{b:?} in {clip:?}");
        assert!(
            b.left() - FOCUS_RING_ROOM >= clip.left(),
            "{b:?} in {clip:?}"
        );
    })
    .unwrap();
}

// The accent ring follows zoom like the kit's band plus border, never thinner than 2 px.
#[test]
fn the_accent_ring_scales_with_zoom_from_two_pixels() {
    use apinata_app::ui::accent_ring_width;
    use apinata_app::zoom::ui_font_size;
    assert_eq!(accent_ring_width(ui_font_size(1.0)), px(2.));
    assert_eq!(accent_ring_width(ui_font_size(1.5)), px(3.));
    assert_eq!(accent_ring_width(ui_font_size(2.0)), px(4.));
    assert_eq!(accent_ring_width(ui_font_size(0.5)), px(2.));
}

// The breadcrumb clips its shrinking crumbs, and the request column clips on every side, so
// the first crumb's ring needs room on its left inside both.
#[gpui_kit::test]
fn the_first_crumb_keeps_its_ring_inside_the_breadcrumb(cx: &mut TestAppContext) {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x")),
    ]);
    let (_state, ws, root) = open_dir(cx, &dir);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let b = w.find("breadcrumb-collection").bounds();
        for clip in ["breadcrumb", "request-column"] {
            let c = w.find(clip).bounds();
            assert!(
                b.left() - FOCUS_RING_ROOM >= c.left(),
                "{b:?} in {clip} {c:?}"
            );
            assert!(
                b.top() - FOCUS_RING_ROOM >= c.top(),
                "{b:?} in {clip} {c:?}"
            );
        }
        let c = w.find("breadcrumb").bounds();
        assert!(b.bottom() + FOCUS_RING_ROOM <= c.bottom(), "{b:?} in {c:?}");
    })
    .unwrap();
}

// The method select starts the URL row at the request column's clipped left edge.
#[gpui_kit::test]
fn the_method_select_keeps_its_ring_inside_the_request_column(cx: &mut TestAppContext) {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x")),
    ]);
    let (_state, ws, root) = open_dir(cx, &dir);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let b = w.find("method").bounds();
        let c = w.find("request-column").bounds();
        assert!(b.left() - FOCUS_RING_ROOM >= c.left(), "{b:?} in {c:?}");
    })
    .unwrap();
}

// Whether `id`'s ring, its bounds grown by the ring's room, lies inside the window and inside
// each of the `clips` ancestors that cut it.
fn assert_ring_inside(
    cx: &mut TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    id: &'static str,
    clips: &[&'static str],
) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let b = w.find(id).bounds();
        let view = w.viewport_size();
        let mut boxes = vec![(
            "window",
            gpui_kit::Bounds::new(gpui_kit::point(px(0.), px(0.)), view),
        )];
        boxes.extend(clips.iter().map(|c| (*c, w.find(*c).bounds())));
        for (name, c) in boxes {
            assert!(
                b.left() - FOCUS_RING_ROOM >= c.left()
                    && b.right() + FOCUS_RING_ROOM <= c.right()
                    && b.top() - FOCUS_RING_ROOM >= c.top()
                    && b.bottom() + FOCUS_RING_ROOM <= c.bottom(),
                "{id} {b:?} rings past {name} {c:?}"
            );
        }
    })
    .unwrap();
}

// The section strip's chevron ends the strip at the request column's right edge, which the
// window cuts in a narrow editor.
#[gpui_kit::test]
fn the_section_chevron_keeps_its_ring_inside_the_window(cx: &mut TestAppContext) {
    use gpui_kit::size;
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.simulate_window_resize(win, size(px(360.), px(700.)));
    frame(cx, win);
    frame(cx, win);
    assert!(present(cx, win, "section-overflow"), "the strip overflows");
    assert_ring_inside(cx, win, "section-overflow", &["request-column"]);
}

// The response strip's chevron sits at the pane's right edge, flush with the window.
#[gpui_kit::test]
fn the_response_chevron_keeps_its_ring_inside_the_window(cx: &mut TestAppContext) {
    use apinata_app::response_pane::ResponsePane;
    use gpui_kit::size;
    let s = send_once(cx, wiremock::ResponseTemplate::new(200), "");
    crate::support::init(cx);
    let model = s.model.clone();
    let (win, _pane) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    cx.simulate_window_resize(win, size(px(380.), px(600.)));
    frame(cx, win);
    frame(cx, win);
    assert!(
        present(cx, win, "response-tab-overflow"),
        "the strip overflows"
    );
    assert_ring_inside(cx, win, "response-tab-overflow", &[]);
}

// The main tab strip's chevron ends the shell's top row at the window's right edge.
#[gpui_kit::test]
fn the_tab_chevron_keeps_its_ring_inside_the_window(cx: &mut TestAppContext) {
    use gpui_kit::size;
    let files: Vec<(String, String)> = (0..9)
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
    for i in 0..9 {
        open_tab(cx, &s.ws, &s.root, &format!("r{i}.yml"));
    }
    gpui_kit::VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(440.), px(800.)));
    frame(cx, s.win);
    frame(cx, s.win);
    assert_ring_inside(cx, s.win, "tab-overflow", &["tab-strip"]);
}

// Whether `chevron`'s ring, its bounds grown by the ring's room, stays clear of what the strip
// `strip` paints before it: the tab viewport (`<strip>-viewport`) and the right fade, which
// is drawn over the strip after the chevron. `fade` says whether tabs are hidden right now.
fn assert_ring_clear_of_strip(
    cx: &mut TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    chevron: &'static str,
    strip: &'static str,
    fade: bool,
) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let b = w.find(chevron).bounds();
        let ring = gpui_kit::Bounds::from_corners(
            gpui_kit::point(b.left() - FOCUS_RING_ROOM, b.top() - FOCUS_RING_ROOM),
            gpui_kit::point(b.right() + FOCUS_RING_ROOM, b.bottom() + FOCUS_RING_ROOM),
        );
        let right = format!("{strip}-fade-right");
        assert_eq!(w.try_find(right.clone()).is_some(), fade, "{right} shows");
        let mut covers = vec![("viewport", w.find(format!("{strip}-viewport")).bounds())];
        if fade {
            covers.push(("fade", w.find(right.clone()).bounds()));
        }
        for (name, c) in covers {
            assert!(
                !ring.intersects(&c),
                "{chevron} ring {ring:?} meets the {name} {c:?}"
            );
        }
    })
    .unwrap();
}

// While sections are hidden right, the right fade ends where the tab viewport does, so the
// chevron after it needs its ring's room on the left too. Checked at the start and the end.
#[gpui_kit::test]
fn the_section_chevron_ring_clears_the_fade_and_the_tabs(cx: &mut TestAppContext) {
    use gpui_kit::size;
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.simulate_window_resize(win, size(px(360.), px(700.)));
    frame(cx, win);
    frame(cx, win);
    assert_ring_clear_of_strip(cx, win, "section-overflow", "section-strip", true);
    ed.update(cx, |e, cx| e.show(Section::Settings, cx));
    frame(cx, win);
    frame(cx, win);
    assert_ring_clear_of_strip(cx, win, "section-overflow", "section-strip", false);
}

#[gpui_kit::test]
fn the_response_chevron_ring_clears_the_fade_and_the_tabs(cx: &mut TestAppContext) {
    use apinata_app::response_pane::{ResponsePane, ResponseTab};
    use gpui_kit::size;
    let s = send_once(cx, wiremock::ResponseTemplate::new(200), "");
    crate::support::init(cx);
    let model = s.model.clone();
    let (win, pane) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    cx.simulate_window_resize(win, size(px(380.), px(600.)));
    frame(cx, win);
    frame(cx, win);
    let strip = "response-tab-strip";
    assert_ring_clear_of_strip(cx, win, "response-tab-overflow", strip, true);
    pane.update(cx, |p, cx| p.set_tab(ResponseTab::Console, cx));
    frame(cx, win);
    frame(cx, win);
    assert_ring_clear_of_strip(cx, win, "response-tab-overflow", strip, false);
}

#[gpui_kit::test]
fn the_tab_chevron_ring_clears_the_fade_and_the_tabs(cx: &mut TestAppContext) {
    use gpui_kit::size;
    let files: Vec<(String, String)> = (0..9)
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
    for i in 0..9 {
        open_tab(cx, &s.ws, &s.root, &format!("r{i}.yml"));
    }
    gpui_kit::VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(440.), px(800.)));
    s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(0)));
    cx.run_until_parked();
    frame(cx, s.win);
    frame(cx, s.win);
    assert_ring_clear_of_strip(cx, s.win, "tab-overflow", "tab-strip", true);
    s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(8)));
    cx.run_until_parked();
    frame(cx, s.win);
    frame(cx, s.win);
    assert_ring_clear_of_strip(cx, s.win, "tab-overflow", "tab-strip", false);
}
