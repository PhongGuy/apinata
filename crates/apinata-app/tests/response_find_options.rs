//! The response find bar's regex and whole-word toggles drive the count, the highlights and
//! the stepping, in the editor and in long-body rows.

use crate::support;

use apinata_app::body_find::{FIND_HIT_CAP, find_selection};
use apinata_app::body_view::{BodyView, SETTLE, Shown};
use gpui_kit::component::ThemeMode;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, Focusable as _, Hsla, TestAppContext, WindowOptions,
    px, size,
};
use std::ops::Range;
use std::time::Duration;
use support::*;
use wiremock::ResponseTemplate;

// Literal "x" hits four times; "x+" as a regex three times; whole-word "x" twice.
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

fn body(cx: &mut TestAppContext, text: &str) -> (AnyWindowHandle, Entity<BodyView>) {
    host_with(
        cx,
        ResponseTemplate::new(200).set_body_raw(text.to_owned(), "application/json"),
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
    frame(cx, win);
}

fn label(cx: &mut TestAppContext, view: &Entity<BodyView>) -> String {
    view.read_with(cx, |v, cx| v.find_label(cx))
}

fn click(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) {
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click(id, cx)).unwrap();
    cx.run_until_parked();
    frame(cx, win);
}

fn step(cx: &mut TestAppContext, win: AnyWindowHandle, view: &Entity<BodyView>, forward: bool) {
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| {
            if forward {
                v.find_next(w, cx)
            } else {
                v.find_previous(w, cx)
            }
        })
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, win);
}

// The text the editor's match highlights cover, in order.
fn marked(cx: &mut TestAppContext, view: &Entity<BodyView>) -> Vec<String> {
    view.read_with(cx, |v, cx| {
        let text = v.editor().read(cx).value();
        v.find_marks(cx)
            .into_iter()
            .map(|r: Range<usize>| text[r].to_string())
            .collect()
    })
}

fn kit_highlighting(cx: &mut TestAppContext, view: &Entity<BodyView>) -> bool {
    let ed = view.read_with(cx, |v, _| v.editor().clone());
    ed.read_with(cx, |e, _| e.search_session().is_active())
}

#[gpui_kit::test]
fn the_regex_toggle_counts_pattern_hits_and_steps_through_them(cx: &mut TestAppContext) {
    let (win, view) = body(cx, XS);
    query(cx, win, &view, "x+");
    assert_eq!(label(cx, &view), "No matches", "literal until the toggle");
    click(cx, win, "body-find-regex");
    assert!(view.read_with(cx, |v, _| v.find_regex()));
    assert_eq!(label(cx, &view), "1 of 3");
    assert_eq!(marked(cx, &view), ["x", "X", "xx"]);
    assert!(!kit_highlighting(cx, &view), "one set of highlights");
    step(cx, win, &view, true);
    assert_eq!(label(cx, &view), "2 of 3");
    step(cx, win, &view, false);
    step(cx, win, &view, false);
    assert_eq!(label(cx, &view), "3 of 3", "wraps backwards");
    step(cx, win, &view, true);
    assert_eq!(label(cx, &view), "1 of 3", "wraps forwards");
    click(cx, win, "body-find-case");
    assert_eq!(label(cx, &view), "1 of 2", "case applies to the pattern");
    click(cx, win, "body-find-regex");
    assert_eq!(label(cx, &view), "No matches");
    assert!(marked(cx, &view).is_empty());
}

#[gpui_kit::test]
fn the_whole_word_toggle_drops_hits_inside_words(cx: &mut TestAppContext) {
    let (win, view) = body(cx, XS);
    query(cx, win, &view, "x");
    assert_eq!(label(cx, &view), "1 of 4");
    click(cx, win, "body-find-word");
    assert!(view.read_with(cx, |v, _| v.find_whole_word()));
    assert_eq!(label(cx, &view), "1 of 2");
    assert_eq!(marked(cx, &view), ["x", "X"]);
    click(cx, win, "body-find-case");
    assert_eq!(label(cx, &view), "1 of 1");
    step(cx, win, &view, true);
    assert_eq!(label(cx, &view), "1 of 1");
    click(cx, win, "body-find-word");
    assert!(marked(cx, &view).is_empty(), "the kit highlights again");
    assert!(kit_highlighting(cx, &view));
    assert_eq!(label(cx, &view), "1 of 3");
}

#[gpui_kit::test]
fn an_invalid_pattern_shows_zero_matches_and_an_invalid_field(cx: &mut TestAppContext) {
    let (win, view) = body(cx, XS);
    view.update(cx, |v, cx| v.set_find_regex(true, cx));
    query(cx, win, &view, "x+");
    assert_eq!(marked(cx, &view).len(), 3);
    assert!(!present(cx, win, "body-find-invalid-icon"));
    query(cx, win, &view, "x(");
    assert!(view.read_with(cx, |v, _| v.find_invalid()));
    assert_eq!(label(cx, &view), "Invalid pattern");
    assert!(marked(cx, &view).is_empty(), "no stale highlights");
    assert!(present(cx, win, "body-find-invalid-icon"));
    step(cx, win, &view, true);
    step(cx, win, &view, false);
    assert_eq!(label(cx, &view), "Invalid pattern");
    click(cx, win, "body-find-regex");
    assert!(!view.read_with(cx, |v, _| v.find_invalid()));
    assert_eq!(label(cx, &view), "No matches");
    assert!(!present(cx, win, "body-find-invalid-icon"));
}

#[gpui_kit::test]
fn empty_width_patterns_mark_nothing(cx: &mut TestAppContext) {
    let (win, view) = body(cx, XS);
    view.update(cx, |v, cx| v.set_find_regex(true, cx));
    for q in ["x*", "^", "$", r"\b", "(?=x)"] {
        query(cx, win, &view, q);
        assert!(marked(cx, &view).iter().all(|m| !m.is_empty()), "{q}");
    }
    query(cx, win, &view, "^");
    assert_eq!(label(cx, &view), "No matches");
    step(cx, win, &view, true);
    assert_eq!(label(cx, &view), "No matches");
}

#[gpui_kit::test]
fn highlights_sit_on_the_hits_in_a_non_ascii_body(cx: &mut TestAppContext) {
    let (win, view) = body(cx, r#"{"ü→":"😀ab","é":"😀AB xab"}"#);
    view.update(cx, |v, cx| v.set_find_regex(true, cx));
    query(cx, win, &view, "😀.");
    assert_eq!(marked(cx, &view), ["😀a", "😀A"]);
    query(cx, win, &view, "ab");
    assert_eq!(marked(cx, &view), ["ab", "AB", "ab"]);
    view.update(cx, |v, cx| {
        v.set_find_regex(false, cx);
        v.set_find_whole_word(true, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        marked(cx, &view),
        ["ab", "AB"],
        "an emoji is not a word character"
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

#[gpui_kit::test]
fn long_body_rows_honour_both_toggles(cx: &mut TestAppContext) {
    let (win, view) = host_with(
        cx,
        ResponseTemplate::new(200).set_body_string(two_needles()),
    );
    assert_eq!(view.read_with(cx, |v, _| v.shown()), Shown::Lines(30_000));
    view.update(cx, |v, cx| v.set_find_regex(true, cx));
    query(cx, win, &view, "^row 2500[0-9]$");
    assert_eq!(label(cx, &view), "1 of 9", "25000 is a needle");
    step(cx, win, &view, true);
    assert_eq!(label(cx, &view), "2 of 9");
    assert!(present(cx, win, ("line-number", 25_002usize)));
    query(cx, win, &view, "row (");
    assert_eq!(label(cx, &view), "Invalid pattern");
    view.update(cx, |v, cx| {
        v.set_find_regex(false, cx);
        v.set_find_whole_word(true, cx);
    });
    query(cx, win, &view, "needle");
    cx.run_until_parked();
    assert_eq!(label(cx, &view), "1 of 2");
    view.update(cx, |v, cx| v.set_find_case_sensitive(true, cx));
    cx.run_until_parked();
    assert_eq!(label(cx, &view), "1 of 1");
    query(cx, win, &view, "row 2");
    cx.run_until_parked();
    assert_eq!(label(cx, &view), "1 of 1", "row 2 alone, not row 20 and on");
}

// The 0-based rows of a body with "needle {i}" on every tenth row.
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

#[gpui_kit::test]
fn a_regex_step_centres_the_match_in_the_editor(cx: &mut TestAppContext) {
    let (win, view) = host_with(cx, ResponseTemplate::new(200).set_body_string(needles(200)));
    assert_eq!(view.read_with(cx, |v, _| v.shown()), Shown::Editor);
    cx.simulate_window_resize(win, size(px(800.), px(400.)));
    frame(cx, win);
    view.update(cx, |v, cx| v.set_find_regex(true, cx));
    query(cx, win, &view, r"needle \d+");
    assert_eq!(label(cx, &view), "1 of 20");
    for _ in 0..3 {
        step(cx, win, &view, true);
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

// A click on a toggle takes focus off the field; it must come back so typing goes on.
fn toggle_keeps_focus_in_the_field(cx: &mut TestAppContext, button: &'static str) {
    let (win, view) = body(cx, XS);
    query(cx, win, &view, "x");
    let input = view.read_with(cx, |v, _| v.find_input().clone());
    let ed = view.read_with(cx, |v, _| v.editor().clone());
    cx.update_window(win, |_, w, cx| ed.read(cx).focus_handle(cx).focus(w, cx))
        .unwrap();
    click(cx, win, button);
    let focused = cx
        .update_window(win, |_, w, cx| {
            input.read(cx).focus_handle(cx).is_focused(w)
        })
        .unwrap();
    assert!(focused, "{button} must leave the find field focused");
}

#[gpui_kit::test]
fn clicking_the_regex_toggle_keeps_focus_in_the_field(cx: &mut TestAppContext) {
    toggle_keeps_focus_in_the_field(cx, "body-find-regex");
}

#[gpui_kit::test]
fn clicking_the_whole_word_toggle_keeps_focus_in_the_field(cx: &mut TestAppContext) {
    toggle_keeps_focus_in_the_field(cx, "body-find-word");
}

#[cfg(not(target_os = "macos"))]
#[gpui_kit::test]
fn alt_r_and_alt_w_flip_the_toggles_from_the_field(cx: &mut TestAppContext) {
    let (win, view) = body(cx, XS);
    query(cx, win, &view, "x");
    let input = view.read_with(cx, |v, _| v.find_input().clone());
    cx.update_window(win, |_, w, cx| input.read(cx).focus_handle(cx).focus(w, cx))
        .unwrap();
    cx.simulate_keystrokes(win, "alt-w");
    cx.run_until_parked();
    assert!(view.read_with(cx, |v, _| v.find_whole_word()));
    assert_eq!(label(cx, &view), "1 of 2");
    cx.simulate_keystrokes(win, "alt-r");
    cx.run_until_parked();
    assert!(view.read_with(cx, |v, _| v.find_regex()));
    assert_eq!(
        view.read_with(cx, |v, cx| v.find_input().read(cx).value()),
        "x",
        "the chords type nothing"
    );
}

fn wait(cx: &mut TestAppContext, by: Duration) {
    cx.executor().advance_clock(by);
    cx.run_until_parked();
}

fn focus_field(cx: &mut TestAppContext, win: AnyWindowHandle, view: &Entity<BodyView>) {
    let input = view.read_with(cx, |v, _| v.find_input().clone());
    cx.update_window(win, |_, w, cx| input.read(cx).focus_handle(cx).focus(w, cx))
        .unwrap();
}

// A pattern typed into the field scans nothing on the keystroke itself: the count waits for
// the typing to pause, and the scan then lands off the UI thread.
#[gpui_kit::test]
fn a_typed_pattern_waits_for_a_pause_before_it_scans(cx: &mut TestAppContext) {
    let (win, view) = body(cx, XS);
    view.update(cx, |v, cx| v.set_find_regex(true, cx));
    query(cx, win, &view, "");
    focus_field(cx, win, &view);
    let scans = view.read_with(cx, |v, _| v.editor_find_scans());
    cx.update_window(win, |_, w, cx| w.input("x+", cx)).unwrap();
    cx.run_until_parked();
    assert_eq!(label(cx, &view), "No matches", "no count before the pause");
    assert_eq!(view.read_with(cx, |v, _| v.editor_find_scans()), scans);
    wait(cx, SETTLE - Duration::from_millis(1));
    assert_eq!(label(cx, &view), "No matches", "one millisecond early");
    wait(cx, Duration::from_millis(1));
    assert_eq!(label(cx, &view), "1 of 3");
    assert_eq!(marked(cx, &view), ["x", "X", "xx"]);
}

// Typing voids a scan still in flight, so its count never lands under the new query.
#[gpui_kit::test]
fn typing_voids_an_editor_scan_in_flight(cx: &mut TestAppContext) {
    let (win, view) = body(cx, XS);
    view.update(cx, |v, cx| v.set_find_regex(true, cx));
    query(cx, win, &view, "");
    focus_field(cx, win, &view);
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_find_query("x", w, cx))
    })
    .unwrap();
    cx.update_window(win, |_, w, cx| w.input("x", cx)).unwrap();
    while cx.executor().tick() {
        assert_ne!(label(cx, &view), "1 of 4", "the old scan landed");
    }
    wait(cx, SETTLE);
    assert_eq!(label(cx, &view), "1 of 1");
}

#[gpui_kit::test]
fn the_count_says_when_the_hits_reach_the_cap(cx: &mut TestAppContext) {
    let text = format!(r#"{{"k":"{}"}}"#, "a".repeat(FIND_HIT_CAP + 5));
    let (win, view) = body(cx, &text);
    view.update(cx, |v, cx| v.set_find_regex(true, cx));
    query(cx, win, &view, "a");
    assert_eq!(label(cx, &view), format!("1 of {FIND_HIT_CAP}+"));
    assert_eq!(
        view.read_with(cx, |v, cx| v.find_marks(cx).len()),
        FIND_HIT_CAP
    );
    step(cx, win, &view, false);
    assert_eq!(
        label(cx, &view),
        format!("{FIND_HIT_CAP} of {FIND_HIT_CAP}+"),
        "stepping wraps within the capped hits"
    );
}

#[gpui_kit::test]
fn a_theme_switch_recolours_the_highlights(cx: &mut TestAppContext) {
    let (win, view) = body(cx, XS);
    view.update(cx, |v, cx| v.set_find_regex(true, cx));
    query(cx, win, &view, "x+");
    let other = |cx: &mut TestAppContext| {
        cx.update(|cx| Hsla {
            s: 0.1,
            ..find_selection(cx)
        })
    };
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        cx.update(|cx| apinata_app::theme::set_mode(mode, None, cx));
        cx.run_until_parked();
        let want = (other(cx), cx.update(|cx| find_selection(cx)));
        assert_eq!(
            view.read_with(cx, |v, _| v.find_mark_colours()),
            Some(want),
            "{mode:?}"
        );
    }
}

#[gpui_kit::test]
fn a_new_response_is_scanned_once_under_the_toggles(cx: &mut TestAppContext) {
    let (win, view) = body(cx, XS);
    view.update(cx, |v, cx| v.set_find_regex(true, cx));
    query(cx, win, &view, "x+");
    assert_eq!(label(cx, &view), "1 of 3");
    let next = send_once(
        cx,
        ResponseTemplate::new(200).set_body_raw(r#"{"q":"xxx y x"}"#, "application/json"),
        "",
    );
    let scans = view.read_with(cx, |v, _| v.editor_find_scans());
    let r = next.result.clone();
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(r), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, win);
    assert_eq!(view.read_with(cx, |v, _| v.editor_find_scans()), scans + 1);
    assert_eq!(label(cx, &view), "1 of 2");
    assert_eq!(marked(cx, &view), ["xxx", "x"]);
}

#[gpui_kit::test]
fn the_toggles_hold_across_the_switch_between_editor_and_rows(cx: &mut TestAppContext) {
    let text = format!("{}needle 42{}", "x".repeat(20_500), "y".repeat(400));
    let (win, view) = host_with(cx, ResponseTemplate::new(200).set_body_string(text));
    assert_eq!(view.read_with(cx, |v, _| v.shown()), Shown::Editor);
    view.update(cx, |v, cx| v.set_find_regex(true, cx));
    query(cx, win, &view, r"needle \d+");
    assert_eq!(label(cx, &view), "1 of 1");
    assert_eq!(marked(cx, &view), ["needle 42"]);
    let wrap = |cx: &mut TestAppContext, on: bool| {
        cx.update_window(win, |_, w, cx| {
            view.update(cx, |v, cx| v.set_wrap(on, w, cx))
        })
        .unwrap();
        for _ in 0..3 {
            frame(cx, win);
            cx.run_until_parked();
        }
    };
    wrap(cx, false);
    assert!(matches!(
        view.read_with(cx, |v, _| v.shown()),
        Shown::Lines(_)
    ));
    assert_eq!(label(cx, &view), "1 of 1");
    assert!(marked(cx, &view).is_empty(), "rows paint their own hits");
    wrap(cx, true);
    assert_eq!(view.read_with(cx, |v, _| v.shown()), Shown::Editor);
    assert_eq!(label(cx, &view), "1 of 1");
    assert_eq!(marked(cx, &view), ["needle 42"]);
}
