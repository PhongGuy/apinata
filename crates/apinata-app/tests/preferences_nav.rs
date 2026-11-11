//! The Preferences section list: order, jumping, the current-section highlight and search.

use crate::support;

use apinata_app::style::{SPACE_3, at_zoom};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Bounds, Pixels, ScrollDelta, TestAppContext, point, px};
use support::*;

const SECTIONS: [&str; 7] = [
    "prefs-general",
    "prefs-appearance",
    "prefs-network",
    "prefs-scripts",
    "prefs-updates",
    "prefs-history",
    "prefs-keyboard",
];

fn entry(section: &str) -> String {
    section.replacen("prefs-", "prefs-nav-", 1)
}

fn open_prefs(cx: &mut TestAppContext, s: &Started) {
    cx.update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap();
    frame(cx, s.win);
    frame(cx, s.win);
}

fn bounds(cx: &mut TestAppContext, s: &Started, id: &str) -> Bounds<Pixels> {
    let id = id.to_string();
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).bounds()
    })
    .unwrap()
}

fn click(cx: &mut TestAppContext, s: &Started, id: &str) {
    let id = id.to_string();
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.click(id, cx);
    })
    .unwrap();
    frame(cx, s.win);
    frame(cx, s.win);
}

fn wheel(cx: &mut TestAppContext, s: &Started, dy: f32) {
    for _ in 0..40 {
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.scroll(
                "preferences-rows",
                ScrollDelta::Pixels(point(px(0.), px(dy))),
                cx,
            )
        })
        .unwrap();
    }
    frame(cx, s.win);
    frame(cx, s.win);
}

fn scroll_rows(cx: &mut TestAppContext, s: &Started, dy: f32) {
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.scroll(
            "preferences-rows",
            ScrollDelta::Pixels(point(px(0.), px(dy))),
            cx,
        )
    })
    .unwrap();
    frame(cx, s.win);
}

/// The sections whose list entry is marked selected.
fn highlighted(cx: &mut TestAppContext, s: &Started) -> Vec<&'static str> {
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        SECTIONS
            .into_iter()
            .filter(|id| w.find(entry(id)).selected() == Some(true))
            .collect()
    })
    .unwrap()
}

/// The sections whose list entry is shown and enabled.
fn offered(cx: &mut TestAppContext, s: &Started) -> Vec<&'static str> {
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        SECTIONS
            .into_iter()
            .filter(|id| {
                w.try_find(entry(id))
                    .is_some_and(|e| e.visible() && e.disabled() != Some(true))
            })
            .collect()
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_list_names_every_section_in_order_left_of_the_rows(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    let rows = bounds(cx, &s, "preferences-rows");
    let list = bounds(cx, &s, "prefs-nav");
    assert!(list.right() <= rows.left(), "{list:?} beside {rows:?}");
    let tops: Vec<_> = SECTIONS
        .iter()
        .map(|id| bounds(cx, &s, &entry(id)).top())
        .collect();
    assert!(tops.windows(2).all(|p| p[0] < p[1]), "{tops:?}");
    assert_eq!(offered(cx, &s), SECTIONS);
}

#[gpui_kit::test]
fn scripts_get_their_own_section_after_network(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        for row in ["pref-run-scripts", "pref-script-timeout"] {
            assert!(w.within("prefs-scripts").try_find(row).is_some(), "{row}");
            assert!(w.within("prefs-network").try_find(row).is_none(), "{row}");
        }
        assert!(
            w.find("prefs-scripts-heading").bounds().top()
                > w.find("prefs-network").bounds().bottom()
        );
        assert!(w.find("prefs-scripts").bounds().bottom() < w.find("prefs-updates").bounds().top());
    })
    .unwrap();
    replace_input(cx, s.win, None, "prefs-search", "script time");
    frame(cx, s.win);
    assert!(present(cx, s.win, "pref-script-timeout"));
    assert!(present(cx, s.win, "prefs-scripts-heading"));
    assert!(!present(cx, s.win, "prefs-network"));
}

#[gpui_kit::test]
fn a_click_brings_the_section_heading_to_the_top(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    let slack = at_zoom(SPACE_3, 1.0);
    for section in ["prefs-history", "prefs-scripts", "prefs-keyboard"] {
        click(cx, &s, &entry(section));
        let rows = bounds(cx, &s, "preferences-rows");
        let heading = bounds(cx, &s, &format!("{section}-heading"));
        assert!(
            heading.top() >= rows.top() - px(1.) && heading.top() <= rows.top() + slack,
            "{section}: heading {heading:?} against rows {rows:?}"
        );
    }
}

#[gpui_kit::test]
fn the_section_at_the_top_is_highlighted(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    assert_eq!(highlighted(cx, &s), ["prefs-general"]);
    click(cx, &s, &entry("prefs-history"));
    assert_eq!(highlighted(cx, &s), ["prefs-history"]);
    wheel(cx, &s, 400.);
    assert_eq!(
        highlighted(cx, &s),
        ["prefs-general"],
        "the wheel moves it too"
    );
    wheel(cx, &s, -400.);
    assert_eq!(highlighted(cx, &s), ["prefs-keyboard"]);
}

#[gpui_kit::test]
fn a_search_withdraws_the_entries_it_filters_out(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    replace_input(cx, s.win, None, "prefs-search", "tls");
    frame(cx, s.win);
    assert_eq!(offered(cx, &s), ["prefs-network"]);
    replace_input(cx, s.win, None, "prefs-search", "");
    frame(cx, s.win);
    assert_eq!(offered(cx, &s), SECTIONS);
}

#[gpui_kit::test]
fn clicking_an_entry_the_search_hides_is_harmless(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    replace_input(cx, s.win, None, "prefs-search", "tls");
    frame(cx, s.win);
    let before = bounds(cx, &s, "prefs-network-heading");
    // A withdrawn entry may be absent; when it is shown, its click must change nothing.
    let shown = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find(entry("prefs-history")).is_some()
        })
        .unwrap();
    if shown {
        click(cx, &s, &entry("prefs-history"));
    }
    assert!(!present(cx, s.win, "prefs-history"));
    assert_eq!(bounds(cx, &s, "prefs-network-heading"), before);
    let network = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.find(entry("prefs-network")).selected()
        })
        .unwrap();
    assert_eq!(network, Some(true));
}

// A window too narrow for the whole dialog gives the room to the rows and drops the list.
#[gpui_kit::test]
fn the_list_hides_in_a_narrow_window(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    gpui_kit::VisualTestContext::from_window(s.win, cx)
        .simulate_resize(gpui_kit::size(px(520.), px(800.)));
    open_prefs(cx, &s);
    for (width, shown) in [(520., false), (1280., true)] {
        gpui_kit::VisualTestContext::from_window(s.win, cx)
            .simulate_resize(gpui_kit::size(px(width), px(800.)));
        frame(cx, s.win);
        assert_eq!(present(cx, s.win, "prefs-nav"), shown, "at {width} px");
        assert!(present(cx, s.win, "preferences-rows"));
    }
}

// A window too short for the whole list keeps the list inside the dialog body, and the wheel
// over the list brings its last entry into view.
#[gpui_kit::test]
fn the_list_scrolls_in_a_short_window(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    gpui_kit::VisualTestContext::from_window(s.win, cx)
        .simulate_resize(gpui_kit::size(px(1280.), px(300.)));
    open_prefs(cx, &s);
    let rows = bounds(cx, &s, "preferences-rows");
    let list = bounds(cx, &s, "prefs-nav");
    assert!(
        list.bottom() <= rows.bottom() + px(1.),
        "the list {list:?} ends with the rows {rows:?}"
    );
    let last = entry("prefs-keyboard");
    assert!(
        bounds(cx, &s, &last).bottom() > list.bottom() + px(1.),
        "the list overflows"
    );
    for _ in 0..20 {
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.scroll(
                "prefs-nav",
                ScrollDelta::Pixels(point(px(0.), px(-100.))),
                cx,
            )
        })
        .unwrap();
    }
    frame(cx, s.win);
    let list = bounds(cx, &s, "prefs-nav");
    let shown = bounds(cx, &s, &last);
    assert!(
        shown.top() >= list.top() - px(1.) && shown.bottom() <= list.bottom() + px(1.),
        "the wheel shows {shown:?} inside {list:?}"
    );
}

// At 1280x360 and 200% the first heading must show above the fade, not under it.
#[gpui_kit::test]
fn the_first_heading_shows_in_a_short_window_at_200_percent(cx: &mut TestAppContext) {
    use apinata_app::app::AppState;
    let col = two_requests();
    let s = started(cx, &col);
    gpui_kit::VisualTestContext::from_window(s.win, cx)
        .simulate_resize(gpui_kit::size(px(1280.), px(360.)));
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| p.update(cx, |s| s.zoom = 2.0));
    });
    cx.run_until_parked();
    open_prefs(cx, &s);
    let rows = bounds(cx, &s, "preferences-rows");
    let heading = bounds(cx, &s, "prefs-general-heading");
    let fade = bounds(cx, &s, "prefs-scroll-fade");
    assert!(
        fade.size.height <= rows.size.height / 4. + px(1.),
        "the fade {fade:?} takes at most a quarter of the rows {rows:?}"
    );
    assert!(
        heading.bottom() <= fade.top() + px(1.),
        "the heading {heading:?} ends above the fade {fade:?}"
    );
}

// At 1280x360 and 200% the section list is hidden, so the wheel is the only way to
// Appearance. With its heading at the rows' top, the heading and the Theme row are both whole
// inside the rows box (the fade may soften the row, which is under it only by its last pixels).
#[gpui_kit::test]
fn the_appearance_heading_and_theme_row_show_whole_in_a_short_window(cx: &mut TestAppContext) {
    use apinata_app::app::AppState;
    let col = two_requests();
    let s = started(cx, &col);
    gpui_kit::VisualTestContext::from_window(s.win, cx)
        .simulate_resize(gpui_kit::size(px(1280.), px(360.)));
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| p.update(cx, |s| s.zoom = 2.0));
    });
    cx.run_until_parked();
    open_prefs(cx, &s);
    // Two passes: the first lands near, the second absorbs what the first left.
    for _ in 0..2 {
        let rows = bounds(cx, &s, "preferences-rows");
        let heading = bounds(cx, &s, "prefs-appearance-heading");
        scroll_rows(cx, &s, -f32::from(heading.top() - rows.top()));
    }
    let rows = bounds(cx, &s, "preferences-rows");
    let heading = bounds(cx, &s, "prefs-appearance-heading");
    let theme = bounds(cx, &s, "pref-theme");
    assert!(
        heading.top() >= rows.top() - px(1.) && heading.top() <= rows.top() + px(20.),
        "{heading:?} at the top of {rows:?}"
    );
    assert!(
        theme.bottom() <= rows.bottom() + px(1.),
        "Theme {theme:?} ends inside {rows:?}"
    );
}

/// The sections whose entry the frame just drawn marks selected, read without drawing again.
fn drawn_highlight(w: &gpui_kit::Window) -> Vec<&'static str> {
    SECTIONS
        .into_iter()
        .filter(|id| w.find(entry(id)).selected() == Some(true))
        .collect()
}

/// The bounds of the text the last frame painted inside element `id`.
fn drawn_text_in(w: &gpui_kit::Window, id: &str) -> Vec<(String, Bounds<Pixels>)> {
    let id = gpui_kit::ElementId::Name(id.to_string().into());
    gpui_kit::base::test_support::texts(w)
        .into_iter()
        .filter(|t| t.path().contains(&id))
        .map(|t| (t.text().to_string(), t.bounds()))
        .collect()
}

// A click on an entry marks it in the very frame that answers the click, not a frame later.
#[gpui_kit::test]
fn a_click_highlights_its_entry_in_the_same_frame(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    for section in [
        "prefs-history",
        "prefs-scripts",
        "prefs-keyboard",
        "prefs-updates",
        "prefs-general",
    ] {
        let drawn = cx
            .update_window(s.win, |_, w, cx| {
                w.click(entry(section), cx);
                drawn_highlight(w)
            })
            .unwrap();
        assert_eq!(drawn, [section], "the frame after clicking {section}");
    }
}

// With only the end of the previous section left as a thin band at the top, the section that
// fills the view is the one marked, in the frame the wheel draws.
#[gpui_kit::test]
fn the_wheel_highlights_the_section_filling_the_view(cx: &mut TestAppContext) {
    const BAND: f32 = 24.;
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    for section in ["prefs-scripts", "prefs-keyboard"] {
        let heading = format!("{section}-heading");
        // The first pass lands near the band; the second, checked in its own frame, on it.
        let rows = bounds(cx, &s, "preferences-rows");
        let at = bounds(cx, &s, &heading);
        scroll_rows(cx, &s, -f32::from(at.top() - rows.top() - px(BAND)));
        let (drawn, at, rows) = cx
            .update_window(s.win, |_, w, cx| {
                w.render_frame(cx);
                let rows = w.find("preferences-rows").bounds();
                let at = w.find(heading.clone()).bounds();
                let dy = -f32::from(at.top() - rows.top() - px(BAND));
                w.scroll(
                    "preferences-rows",
                    ScrollDelta::Pixels(point(px(0.), px(dy))),
                    cx,
                );
                (drawn_highlight(w), w.find(heading.clone()).bounds(), rows)
            })
            .unwrap();
        let band = at.top() - rows.top();
        assert!(
            band >= px(BAND - 4.) && band <= px(BAND + 4.),
            "{section}: heading {at:?} sits {band:?} below the top of {rows:?}"
        );
        assert_eq!(
            drawn,
            [section],
            "the wheel frame with {section} filling the view"
        );
    }
}

// Each entry's name starts at the entry's left edge rather than sitting centred in it.
#[gpui_kit::test]
fn the_entries_read_from_the_left(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        for id in SECTIONS {
            let e = w.find(entry(id)).bounds();
            let texts = drawn_text_in(w, &entry(id));
            let (text, t) = texts.first().expect("the entry draws its name");
            assert!(
                t.left() - e.left() < (e.right() - t.right()) / 2.,
                "{text:?} at {t:?} in {e:?}"
            );
        }
    })
    .unwrap();
}
