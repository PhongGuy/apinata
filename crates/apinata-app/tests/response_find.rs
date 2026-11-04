//! The response find bar counts, highlights and steps through matches.

use crate::support;

use apinata_app::body_find::{find_matches, find_selection};
use apinata_app::body_view::{BodyView, SETTLE, Shown};
use std::time::Duration;

use gpui_kit::component::{ActiveTheme as _, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, Focusable as _, Hsla, Rgba, SharedString,
    TestAppContext, VisualTestContext, WindowOptions, px, size,
};
use support::*;
use wiremock::ResponseTemplate;

const XS: &str = r#"{"a":"x","b":"X","c":"xx"}"#;

fn host_with(
    cx: &mut TestAppContext,
    response: ResponseTemplate,
) -> (AnyWindowHandle, Entity<BodyView>) {
    let s = send_once(cx, response, "");
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| BodyView::new(w, cx))
            })
        })
        .unwrap();
    let r = s.result.clone();
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(r), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, win);
    (win, view)
}

fn xs(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<BodyView>) {
    host_with(
        cx,
        ResponseTemplate::new(200).set_body_raw(XS, "application/json"),
    )
}

fn query(cx: &mut TestAppContext, win: AnyWindowHandle, view: &Entity<BodyView>, q: &str) {
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| {
            v.open_find(w, cx);
            v.set_find_query(q, w, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
}

fn label(cx: &mut TestAppContext, view: &Entity<BodyView>) -> String {
    view.read_with(cx, |v, cx| v.find_label(cx))
}

fn highlighting(cx: &mut TestAppContext, view: &Entity<BodyView>) -> bool {
    let ed = view.read_with(cx, |v, _| v.editor().clone());
    ed.read_with(cx, |e, _| e.search_session().is_active())
}

#[gpui_kit::test]
fn typing_counts_matches_and_enter_steps_through_them(cx: &mut TestAppContext) {
    let (win, view) = xs(cx);
    let ed = view.read_with(cx, |v, _| v.editor().clone());
    let mut v = VisualTestContext::from_window(win, cx);
    v.update(|w, cx| ed.read(cx).focus_handle(cx).focus(w, cx));
    v.simulate_keystrokes(&secondary("f"));
    v.run_until_parked();
    cx.update_window(win, |_, w, cx| w.input("x", cx)).unwrap();
    cx.run_until_parked();
    assert_eq!(label(cx, &view), "1 of 4");
    assert!(highlighting(cx, &view));
    cx.simulate_keystrokes(win, "enter");
    assert_eq!(label(cx, &view), "2 of 4");
    cx.simulate_keystrokes(win, "shift-enter");
    assert_eq!(label(cx, &view), "1 of 4");
    cx.simulate_keystrokes(win, "shift-enter");
    assert_eq!(label(cx, &view), "4 of 4", "wraps backwards");
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("body-find-next", cx))
        .unwrap();
    assert_eq!(label(cx, &view), "1 of 4", "wraps forwards");
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("body-find-prev", cx))
        .unwrap();
    assert_eq!(label(cx, &view), "4 of 4");
    assert!(present(cx, win, "body-find-count"));
}

#[gpui_kit::test]
fn the_case_toggle_counts_only_exact_matches(cx: &mut TestAppContext) {
    let (win, view) = xs(cx);
    query(cx, win, &view, "x");
    assert_eq!(label(cx, &view), "1 of 4");
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("body-find-case", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(view.read_with(cx, |v, _| v.find_case_sensitive()));
    assert_eq!(label(cx, &view), "1 of 3");
}

#[gpui_kit::test]
fn no_match_says_so_and_an_empty_query_says_nothing(cx: &mut TestAppContext) {
    let (win, view) = xs(cx);
    query(cx, win, &view, "zzz");
    assert_eq!(label(cx, &view), "No matches");
    query(cx, win, &view, "");
    assert_eq!(label(cx, &view), "");
    assert!(!highlighting(cx, &view));
}

#[gpui_kit::test]
fn closing_ends_the_highlights(cx: &mut TestAppContext) {
    let (win, view) = xs(cx);
    query(cx, win, &view, "x");
    assert!(highlighting(cx, &view));
    cx.update_window(win, |_, w, cx| view.update(cx, |v, cx| v.close_find(w, cx)))
        .unwrap();
    assert!(!highlighting(cx, &view));
}

#[gpui_kit::test]
fn a_new_text_reruns_the_query(cx: &mut TestAppContext) {
    let (win, view) = xs(cx);
    query(cx, win, &view, "x");
    assert_eq!(label(cx, &view), "1 of 4");
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_filter("$.c", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(label(cx, &view), "1 of 2");
    assert!(highlighting(cx, &view));
}

#[test]
fn row_matches_run_left_to_right_and_fold_ascii_case() {
    let rows: Vec<SharedString> = ["abcabc", "ABC", "", "xabcx"]
        .map(SharedString::from)
        .to_vec();
    assert_eq!(
        find_matches(&rows, "abc", false),
        vec![(0, 0..3), (0, 3..6), (1, 0..3), (3, 1..4)]
    );
    assert_eq!(
        find_matches(&rows, "abc", true),
        vec![(0, 0..3), (0, 3..6), (3, 1..4)]
    );
    assert!(find_matches(&rows, "", false).is_empty());
    let a: Vec<SharedString> = vec!["aaaa".into()];
    assert_eq!(find_matches(&a, "aa", true), vec![(0, 0..2), (0, 2..4)]);
}

#[test]
fn case_folds_beyond_ascii_on_char_boundaries() {
    let rows = |s: &str| vec![SharedString::from(s.to_owned())];
    assert_eq!(find_matches(&rows("xÄBCx"), "äbc", false), vec![(0, 1..5)]);
    let strasse = rows("Straße STRASSE");
    let hits = find_matches(&strasse, "straße", false);
    assert!(hits.contains(&(0, 0..7)), "{hits:?}");
    let dotted = rows("İi");
    for (_, r) in find_matches(&dotted, "i", false) {
        assert!(dotted[0].is_char_boundary(r.start) && dotted[0].is_char_boundary(r.end));
    }
}

#[gpui_kit::test]
fn find_in_long_rows_scrolls_to_each_match(cx: &mut TestAppContext) {
    let body = (0..30_000)
        .map(|i| match i {
            100 => "NEEDLE".to_owned(),
            25_000 => "a needle".to_owned(),
            _ => format!("row {i}"),
        })
        .collect::<Vec<_>>()
        .join("\n");
    let (win, view) = host_with(cx, ResponseTemplate::new(200).set_body_string(body));
    assert_eq!(view.read_with(cx, |v, _| v.shown()), Shown::Lines(30_000));
    assert!(!present(cx, win, ("line-number", 100usize)));
    query(cx, win, &view, "needle");
    assert_eq!(label(cx, &view), "1 of 2");
    assert!(present(cx, win, ("line-number", 100usize)));
    cx.update_window(win, |_, w, cx| view.update(cx, |v, cx| v.find_next(w, cx)))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(label(cx, &view), "2 of 2");
    assert!(present(cx, win, ("line-number", 25_000usize)));
    view.update(cx, |v, cx| v.set_find_case_sensitive(true, cx));
    cx.run_until_parked();
    assert_eq!(label(cx, &view), "1 of 1");
}

#[gpui_kit::test]
fn a_new_row_query_never_shows_the_old_count(cx: &mut TestAppContext) {
    let body = (0..30_000)
        .map(|i| {
            if i == 100 {
                "needle".to_owned()
            } else {
                format!("row {i}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let (win, view) = host_with(cx, ResponseTemplate::new(200).set_body_string(body));
    query(cx, win, &view, "needle");
    assert_eq!(label(cx, &view), "1 of 1");
    // Read before the new scan lands: the label must not pair the new query with the old count.
    let pending = cx
        .update_window(win, |_, w, cx| {
            view.update(cx, |v, cx| {
                v.set_find_query("zzz", w, cx);
                v.find_label(cx)
            })
        })
        .unwrap();
    assert_eq!(pending, "No matches");
    cx.run_until_parked();
    assert_eq!(label(cx, &view), "No matches");
}

// A real click on a bar button takes focus off the field; the click must hand it back so
// typing keeps editing the query. The test moves focus away first to stand in for that.
fn click_returns_focus_to_the_field(cx: &mut TestAppContext, button: &'static str) {
    let (win, view) = xs(cx);
    query(cx, win, &view, "x");
    let input = view.read_with(cx, |v, _| v.find_input().clone());
    let ed = view.read_with(cx, |v, _| v.editor().clone());
    cx.update_window(win, |_, w, cx| ed.read(cx).focus_handle(cx).focus(w, cx))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click(button, cx))
        .unwrap();
    cx.run_until_parked();
    let focused = cx
        .update_window(win, |_, w, cx| {
            input.read(cx).focus_handle(cx).is_focused(w)
        })
        .unwrap();
    assert!(focused, "{button} must leave the find field focused");
    cx.update_window(win, |_, w, cx| w.input("x", cx)).unwrap();
    cx.run_until_parked();
    let typed = view.read_with(cx, |v, cx| v.find_input().read(cx).value().to_string());
    assert_eq!(typed, "xx");
}

#[gpui_kit::test]
fn clicking_the_case_toggle_keeps_focus_in_the_field(cx: &mut TestAppContext) {
    click_returns_focus_to_the_field(cx, "body-find-case");
}

#[gpui_kit::test]
fn clicking_next_keeps_focus_in_the_field(cx: &mut TestAppContext) {
    click_returns_focus_to_the_field(cx, "body-find-next");
}

#[gpui_kit::test]
fn clicking_previous_keeps_focus_in_the_field(cx: &mut TestAppContext) {
    click_returns_focus_to_the_field(cx, "body-find-prev");
}

// `top` painted over `bottom`, by source-over alpha compositing.
fn over(top: Rgba, bottom: Rgba) -> Rgba {
    let a = top.a + bottom.a * (1. - top.a);
    let mix = |t: f32, b: f32| (t * top.a + b * bottom.a * (1. - top.a)) / a;
    Rgba {
        r: mix(top.r, bottom.r),
        g: mix(top.g, bottom.g),
        b: mix(top.b, bottom.b),
        a,
    }
}

// WCAG relative luminance.
fn luminance(c: Rgba) -> f32 {
    let f = |v: f32| {
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * f(c.r) + 0.7152 * f(c.g) + 0.0722 * f(c.b)
}

#[gpui_kit::test]
fn find_colours_are_visible_and_distinct(cx: &mut TestAppContext) {
    support::init(cx);
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        cx.update(|cx| apinata_app::theme::set_mode(mode, None, cx));
        let (bg, sel) = cx.update(|cx| {
            let theme = cx.theme();
            let bg = theme
                .highlight_theme
                .style
                .editor_background
                .unwrap_or_else(|| theme.input_background());
            (bg, find_selection(cx))
        });
        let other = Hsla { s: 0.1, ..sel };
        let o = over(other.to_rgb(), bg.to_rgb());
        let cur = over(sel.to_rgb(), o);
        let lift = (luminance(o) - luminance(bg.to_rgb())).abs();
        assert!(lift >= 0.06, "{mode:?}: other matches barely show ({lift})");
        let step = (luminance(cur) - luminance(o)).abs();
        let sat = Hsla::from(cur).s - Hsla::from(o).s;
        assert!(
            step >= 0.06 || sat >= 0.3,
            "{mode:?}: the current match is not distinct ({step}, {sat})"
        );
    }
}

#[gpui_kit::test]
fn the_response_editor_uses_the_find_colour(cx: &mut TestAppContext) {
    let (win, view) = xs(cx);
    query(cx, win, &view, "x");
    frame(cx, win);
    let want = cx.update(|cx| find_selection(cx));
    assert_eq!(view.read_with(cx, |v, _| v.editor_selection()), Some(want));
}

// The 0-based rows of a body with "needle" on every tenth row.
fn needles(lines: usize) -> String {
    (0..lines)
        .map(|i| {
            if i % 10 == 0 {
                format!("needle {i}")
            } else {
                format!("row {i}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn step_three(cx: &mut TestAppContext, win: AnyWindowHandle, view: &Entity<BodyView>) {
    for _ in 0..3 {
        cx.update_window(win, |_, w, cx| view.update(cx, |v, cx| v.find_next(w, cx)))
            .unwrap();
        cx.run_until_parked();
        frame(cx, win);
    }
    assert_eq!(label(cx, view).split(' ').next(), Some("4"));
}

#[gpui_kit::test]
fn the_current_match_is_centred(cx: &mut TestAppContext) {
    // Editor mode: the fourth match sits on row 30.
    let (win, view) = host_with(cx, ResponseTemplate::new(200).set_body_string(needles(200)));
    assert_eq!(view.read_with(cx, |v, _| v.shown()), Shown::Editor);
    cx.simulate_window_resize(win, size(px(800.), px(400.)));
    frame(cx, win);
    query(cx, win, &view, "needle");
    frame(cx, win);
    step_three(cx, win, &view);
    let ed = view.read_with(cx, |v, _| v.editor().clone());
    let (top, height) = ed.read_with(cx, |e, _| {
        let lh = e.line_height().expect("laid out");
        let view_h = e.text_bounds().expect("laid out").size.height;
        (lh * 30. + e.scroll_offset().y, view_h)
    });
    assert!(
        top >= height / 3. && top <= height * 2. / 3.,
        "row 30 at {top:?} of {height:?}"
    );

    // Rows mode: the same rows past the editor's line limit.
    let (win, view) = host_with(
        cx,
        ResponseTemplate::new(200).set_body_string(needles(20_100)),
    );
    assert!(matches!(
        view.read_with(cx, |v, _| v.shown()),
        Shown::Lines(_)
    ));
    cx.simulate_window_resize(win, size(px(800.), px(400.)));
    frame(cx, win);
    query(cx, win, &view, "needle");
    frame(cx, win);
    step_three(cx, win, &view);
    let (row, frame_b) = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            (
                w.find(("line-number", 30usize)).bounds(),
                w.find("body-lines-frame").bounds(),
            )
        })
        .unwrap();
    let top = row.top() - frame_b.top();
    let height = frame_b.size.height;
    assert!(
        top >= height / 3. && top <= height * 2. / 3.,
        "row 30 at {top:?} of {height:?}"
    );
}

#[gpui_kit::test]
fn moving_the_caret_in_the_body_does_not_recentre_it(cx: &mut TestAppContext) {
    let (win, view) = host_with(cx, ResponseTemplate::new(200).set_body_string(needles(200)));
    cx.simulate_window_resize(win, size(px(800.), px(400.)));
    frame(cx, win);
    let ed = view.read_with(cx, |v, _| v.editor().clone());
    let before = ed.read_with(cx, |e, _| e.scroll_offset());
    // A click on a visible row near the bottom, then a step down: both stay on screen.
    let lower = cx
        .update_window(win, |_, w, _| {
            w.find("body-editor").bounds().size.height * 0.85
        })
        .unwrap();
    cx.update_window(win, |_, w, cx| {
        w.click_at("body-editor", gpui_kit::point(px(120.), lower), cx)
    })
    .unwrap();
    frame(cx, win);
    assert_eq!(ed.read_with(cx, |e, _| e.scroll_offset()), before, "click");
    cx.update_window(win, |_, w, cx| w.press("down", cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(ed.read_with(cx, |e, _| e.scroll_offset()), before, "down");
}

#[gpui_kit::test]
fn enter_in_the_find_field_centres_the_match(cx: &mut TestAppContext) {
    let (win, view) = host_with(cx, ResponseTemplate::new(200).set_body_string(needles(200)));
    cx.simulate_window_resize(win, size(px(800.), px(400.)));
    frame(cx, win);
    query(cx, win, &view, "needle");
    frame(cx, win);
    for _ in 0..3 {
        cx.simulate_keystrokes(win, "enter");
        cx.run_until_parked();
        frame(cx, win);
    }
    assert_eq!(label(cx, &view), "4 of 20");
    let ed = view.read_with(cx, |v, _| v.editor().clone());
    let (top, height) = ed.read_with(cx, |e, _| {
        let lh = e.line_height().expect("laid out");
        (
            lh * 30. + e.scroll_offset().y,
            e.text_bounds().expect("laid out").size.height,
        )
    });
    assert!(
        top >= height / 3. && top <= height * 2. / 3.,
        "row 30 at {top:?} of {height:?}"
    );
}

// 30,000 rows "row {i}", with "NEEDLE" on row 100 and "a needle" on row 25,000.
fn two_needles() -> String {
    (0..30_000)
        .map(|i| match i {
            100 => "NEEDLE".to_owned(),
            25_000 => "a needle".to_owned(),
            _ => format!("row {i}"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn open_find(cx: &mut TestAppContext, win: AnyWindowHandle, view: &Entity<BodyView>) {
    cx.update_window(win, |_, w, cx| view.update(cx, |v, cx| v.open_find(w, cx)))
        .unwrap();
    frame(cx, win);
}

// Types into the focused field and runs what is ready, without moving the clock.
fn type_text(cx: &mut TestAppContext, win: AnyWindowHandle, typed: &str) {
    cx.update_window(win, |_, w, cx| w.input(typed, cx))
        .unwrap();
    cx.run_until_parked();
}

fn wait(cx: &mut TestAppContext, by: Duration) {
    cx.executor().advance_clock(by);
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_typed_row_query_counts_after_the_settle_delay(cx: &mut TestAppContext) {
    let (win, view) = host_with(
        cx,
        ResponseTemplate::new(200).set_body_string(two_needles()),
    );
    assert_eq!(view.read_with(cx, |v, _| v.shown()), Shown::Lines(30_000));
    open_find(cx, win, &view);
    type_text(cx, win, "needle");
    assert_eq!(label(cx, &view), "No matches", "no count before the delay");
    wait(cx, SETTLE - Duration::from_millis(1));
    assert_eq!(label(cx, &view), "No matches", "one millisecond early");
    wait(cx, Duration::from_millis(1));
    assert_eq!(label(cx, &view), "1 of 2");
}

#[gpui_kit::test]
fn a_typed_row_query_never_shows_the_old_count(cx: &mut TestAppContext) {
    let rows = (0..30_000)
        .map(|i| format!("row {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (win, view) = host_with(cx, ResponseTemplate::new(200).set_body_string(rows));
    open_find(cx, win, &view);
    // "row 2" starts rows 2, 20-29, 200-299, 2000-2999 and 20000-29999.
    type_text(cx, win, "row 2");
    wait(cx, SETTLE);
    assert_eq!(label(cx, &view), "1 of 11111");
    type_text(cx, win, "9");
    assert_eq!(label(cx, &view), "No matches", "the old count goes at once");
    wait(cx, SETTLE);
    assert_eq!(label(cx, &view), "1 of 1111");
}

#[gpui_kit::test]
fn the_case_toggle_reruns_a_typed_row_query_at_once(cx: &mut TestAppContext) {
    let (win, view) = host_with(
        cx,
        ResponseTemplate::new(200).set_body_string(two_needles()),
    );
    open_find(cx, win, &view);
    type_text(cx, win, "needle");
    wait(cx, SETTLE);
    assert_eq!(label(cx, &view), "1 of 2");
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("body-find-case", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(label(cx, &view), "1 of 1", "the toggle does not wait");
}

#[gpui_kit::test]
fn enter_while_a_row_query_waits_steps_nothing(cx: &mut TestAppContext) {
    let (win, view) = host_with(
        cx,
        ResponseTemplate::new(200).set_body_string(two_needles()),
    );
    open_find(cx, win, &view);
    type_text(cx, win, "needle");
    cx.simulate_keystrokes(win, "enter");
    assert_eq!(label(cx, &view), "No matches");
    wait(cx, SETTLE);
    assert_eq!(
        label(cx, &view),
        "1 of 2",
        "the scan starts at the first match"
    );
}

// Typing voids a row scan still in flight: its count must never land under the new
// query. The clock moves timers and runs everything due, so the old scan is started
// by `set_find_query`, which runs at once, then each task is stepped by hand.
#[gpui_kit::test]
fn typing_voids_a_row_scan_in_flight(cx: &mut TestAppContext) {
    let rows = (0..30_000)
        .map(|i| format!("row {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (win, view) = host_with(cx, ResponseTemplate::new(200).set_body_string(rows));
    open_find(cx, win, &view);
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_find_query("row 2", w, cx))
    })
    .unwrap();
    cx.update_window(win, |_, w, cx| w.input("9", cx)).unwrap();
    while cx.executor().tick() {
        assert_eq!(label(cx, &view), "No matches", "the old scan landed");
    }
    wait(cx, SETTLE);
    assert_eq!(label(cx, &view), "1 of 1111");
}

#[gpui_kit::test]
fn emptying_a_row_query_clears_at_once(cx: &mut TestAppContext) {
    let (win, view) = host_with(
        cx,
        ResponseTemplate::new(200).set_body_string(two_needles()),
    );
    open_find(cx, win, &view);
    type_text(cx, win, "needle");
    wait(cx, SETTLE);
    assert_eq!(label(cx, &view), "1 of 2");
    cx.simulate_keystrokes(
        win,
        "backspace backspace backspace backspace backspace backspace",
    );
    assert_eq!(
        view.read_with(cx, |v, cx| v.find_input().read(cx).value()),
        ""
    );
    assert_eq!(label(cx, &view), "", "no settle wait for an empty query");
}
