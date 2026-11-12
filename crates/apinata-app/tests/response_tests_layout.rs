//! The Tests tab in a narrow pane: the tab strip never shows a sliver of its first tab, long
//! result lines wrap inside the pane, and a script parse error reads as a sentence.

use crate::support;

use apinata_app::response_pane::{ResponsePane, ResponseTab, assertion_line};
use apinata_core::{Assertion, AssertionOutcome, Verdict};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, TestAppContext, VisualTestContext, WindowOptions, px,
    size,
};
use support::*;
use wiremock::ResponseTemplate;

const FAILING: &str = concat!(
    "runtime:\n  assertions:\n",
    "    - expression: res.status\n      operator: eq\n      value: \"500\"\n",
);

// Lets the kit's spring reveal run to its end, a frame at a time.
fn settle_motion(cx: &mut TestAppContext, win: AnyWindowHandle) {
    for _ in 0..60 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        frame(cx, win);
    }
}

// Side by side, at every width where the response strip scrolls, choosing Tests leaves the
// Body tab either whole in the strip's viewport or wholly scrolled out of it, never a sliver.
#[gpui_kit::test]
fn side_by_side_choosing_tests_never_cuts_the_first_tab(cx: &mut TestAppContext) {
    use apinata_app::app::AppState;
    use apinata_app::settings::Orientation;
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200).set_body_string("{}"));
    let col = collection_for(&server.uri(), FAILING);
    let s = started(cx, &col);
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| {
            p.update(cx, |st| st.response_orientation = Orientation::Horizontal)
        });
    });
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1400.), px(800.)));
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("send", cx))
        .unwrap();
    wait_real(cx, |cx| present(cx, s.win, "response-status"));
    let pane: Entity<ResponsePane> =
        editor_of(cx, &s, tab).read_with(cx, |e, _| e.response().clone());
    let tests = ("response-tab", ResponseTab::Tests as usize);
    let body = ("response-tab", ResponseTab::Body as usize);
    let mut scrolled = 0;
    for width in (700..=1600).rev().step_by(20) {
        VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(width as f32), px(800.)));
        pane.update(cx, |p, cx| p.set_tab(ResponseTab::Body, cx));
        settle_motion(cx, s.win);
        // A click on the Tests label when it shows, as a user would; else the overflow menu.
        let clickable = cx
            .update_window(s.win, |_, w, cx| {
                w.render_frame(cx);
                let view = w.try_find("response-tab-strip-viewport")?.bounds();
                let c = w.find(tests).bounds().center();
                Some(c.x > view.left() && c.x < view.right())
            })
            .unwrap();
        match clickable {
            Some(false) => {
                cx.update_window(s.win, |_, w, cx| w.click("response-tab-overflow", cx))
                    .unwrap();
                frame(cx, s.win);
                cx.update_window(s.win, |_, w, cx| {
                    w.within("popup-menu")
                        .click(ResponseTab::Tests as usize, cx)
                })
                .unwrap();
            }
            _ => cx
                .update_window(s.win, |_, w, cx| w.click(tests, cx))
                .unwrap(),
        }
        settle_motion(cx, s.win);
        assert_eq!(pane.read_with(cx, |p, _| p.tab()), ResponseTab::Tests);
        let (cut, at) = cx
            .update_window(s.win, |_, w, cx| {
                w.render_frame(cx);
                let Some(view) = w.try_find("response-tab-strip-viewport") else {
                    return (false, String::new());
                };
                let (view, b) = (view.bounds(), w.find(body).bounds());
                let cut = b.left() < view.left() - px(0.5) && b.right() > view.left() + px(0.5)
                    || b.left() < view.right() - px(0.5) && b.right() > view.right() + px(0.5);
                (cut, format!("Body {b:?} in {view:?}"))
            })
            .unwrap();
        if !at.is_empty() && present(cx, s.win, "response-tab-overflow") {
            scrolled += 1;
        }
        assert!(!cut, "at {width} px the first tab is cut: {at}");
    }
    assert!(scrolled > 0, "the strip never overflowed");
    std::mem::forget(col);
}

// A standalone pane `width` px wide showing the Tests tab over `assertions`.
fn tests_pane(
    cx: &mut TestAppContext,
    width: f32,
    assertions: Vec<AssertionOutcome>,
) -> (Sent, AnyWindowHandle) {
    let s = send_once(cx, ResponseTemplate::new(200), "");
    let mut r = (*s.result).clone();
    r.assertions = assertions;
    s.model
        .update(cx, |m, cx| m.show(std::sync::Arc::new(r), cx));
    support::init(cx);
    let model = s.model.clone();
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    cx.simulate_window_resize(win, size(px(width), px(600.)));
    view.update(cx, |p, cx| p.set_tab(ResponseTab::Tests, cx));
    for _ in 0..3 {
        frame(cx, win);
    }
    (s, win)
}

// The tooltip `id` shows after hovering `target`, read by its label.
fn tooltip(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    target: (&'static str, usize),
    id: &'static str,
) -> Option<String> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.hover(target, cx)
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(800));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

// A failing line longer than a narrow pane wraps inside it rather than running past its edge.
#[gpui_kit::test]
fn a_long_failure_line_wraps_inside_the_pane(cx: &mut TestAppContext) {
    let long = "https://jsonplaceholder.typicode.com/posts/1/comments?postId=1&sort=desc&limit=50";
    let check = Assertion::new("res.url", "eq", Some(long.into()));
    let actual = format!("\"{long}/and/more\"");
    let outcome = AssertionOutcome::new(check, Verdict::fail(actual.clone(), None));
    let (_s, win) = tests_pane(cx, 360., vec![outcome]);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let frame = w.find("response-assertions").bounds();
        let text = w
            .try_find(("assertion-text", 0usize))
            .expect("the line's text is its own element");
        let b = text.bounds();
        assert!(
            b.right() <= frame.right() + px(0.5),
            "the line {b:?} runs past the pane {frame:?}"
        );
        assert!(
            b.size.height > w.find(("assertion-verdict", 0usize)).bounds().size.height * 1.5,
            "the line {b:?} does not wrap"
        );
        assert!(text.label().unwrap_or_default().contains(&actual));
    })
    .unwrap();
}

// An expected value the engine cannot parse reads as a sentence on the line, and the engine's
// own message stays on offer in the line's tooltip.
#[gpui_kit::test]
fn a_parse_error_reads_as_a_sentence_with_the_engine_text_in_a_tooltip(cx: &mut TestAppContext) {
    let check = Assertion::new("res.status", "eq", Some("7a4".into()));
    let raw = "SyntaxError: expecting ';'";
    let outcome = AssertionOutcome::new(check, Verdict::fail("74", Some(raw.into())));
    assert_eq!(
        assertion_line(&outcome),
        (
            "fail",
            "res.status equals 7a4: actual 74; the expected value is not valid JavaScript"
                .to_string()
        )
    );
    let (_s, win) = tests_pane(cx, 600., vec![outcome]);
    let tip = tooltip(cx, win, ("assertion-text", 0), "assertion-tooltip").unwrap_or_default();
    assert!(tip.contains(raw), "{tip:?}");
}
