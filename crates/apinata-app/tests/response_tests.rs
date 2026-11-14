//! The Tests tab: assertions, then each script test with its phase.

use crate::support;

use apinata_app::response_pane::{
    ResponsePane, ResponseTab, shows_no_tests, test_line, test_source, test_summary, tests_total,
    verdict_colour,
};
use apinata_app::ui::{Tone, success_count_badge};
use apinata_core::{
    Assertion, AssertionOutcome, ItemPath, Outcome, RequestResult, ScriptPhase, TestOutcome,
    Verdict,
};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, WindowOptions};
use support::*;
use wiremock::ResponseTemplate;

fn error(name: &str, message: &str) -> TestOutcome {
    let mut t = TestOutcome::new(
        ScriptPhase::Tests,
        name,
        Verdict::fail("", Some(message.into())),
    );
    t.script_error = true;
    t
}

fn outcomes() -> Vec<TestOutcome> {
    vec![
        TestOutcome::new(ScriptPhase::Tests, "ok", Verdict::Pass),
        TestOutcome::new(
            ScriptPhase::PostResponse,
            "bad",
            Verdict::fail("", Some("expected 1 to equal 2".into())),
        ),
        TestOutcome::new(
            ScriptPhase::Tests,
            "later",
            Verdict::skipped("bru.runRequest is not supported by Apinata"),
        ),
        error("Test script error", "boom"),
    ]
}

// Returns the `Sent` too, so its mock server and temp dirs live as long as the window.
fn shown(
    cx: &mut TestAppContext,
    tests: Vec<TestOutcome>,
    extra: &str,
) -> (Sent, AnyWindowHandle, Entity<ResponsePane>) {
    let s = send_once(cx, ResponseTemplate::new(200), extra);
    let mut r = (*s.result).clone();
    r.tests = tests;
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
    view.update(cx, |p, cx| p.set_tab(ResponseTab::Tests, cx));
    cx.run_until_parked();
    (s, win, view)
}

#[test]
fn each_entry_reads_as_a_verdict_and_a_line() {
    let lines: Vec<_> = outcomes().iter().map(test_line).collect();
    assert_eq!(
        lines,
        [
            ("pass", "ok".to_string()),
            ("fail", "bad: expected 1 to equal 2".to_string()),
            (
                "skipped",
                "later: bru.runRequest is not supported by Apinata".to_string()
            ),
            ("error", "Test script error, boom".to_string()),
        ]
    );
}

#[test]
fn each_entry_names_the_script_it_came_from() {
    let sources: Vec<_> = [
        ScriptPhase::PreRequest,
        ScriptPhase::PostResponse,
        ScriptPhase::Tests,
    ]
    .into_iter()
    .map(test_source)
    .collect();
    assert_eq!(
        sources,
        ["Pre-request script", "Post-response script", "Test script"]
    );
}

#[test]
fn script_errors_are_not_counted_as_tests() {
    assert_eq!(
        test_summary(&outcomes()).as_deref(),
        Some("1 passed, 1 failed, 1 skipped")
    );
    assert_eq!(
        test_summary(&[error("Test script error", "boom")]).as_deref(),
        Some("0 passed, 0 failed, 0 skipped")
    );
    assert_eq!(test_summary(&[]), None);
}

#[gpui_kit::test]
fn the_tab_lists_assertions_then_tests_with_phases(cx: &mut TestAppContext) {
    let extra = "runtime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n";
    let (_s, win, view) = shown(cx, outcomes(), extra);
    assert!(present(cx, win, ("assertion", 0usize)));
    assert!(present(cx, win, "tests-summary"));
    for ix in 0..3usize {
        assert!(present(cx, win, ("test", ix)) && present(cx, win, ("test-phase", ix)));
    }
    // A script error's own name says which script it was, so its row has no phase cell.
    assert!(present(cx, win, ("test", 3usize)) && !present(cx, win, ("test-phase", 3usize)));
    assert_eq!(
        view.read_with(cx, |p, cx| p.tab_count(ResponseTab::Tests, cx)),
        Some(5)
    );
    assert!(view.read_with(cx, |p, cx| p.count_is_danger(ResponseTab::Tests, cx)));
    cx.update(|cx| assert_eq!(verdict_colour("error", cx), cx.theme().danger));
}

#[gpui_kit::test]
fn passing_tests_alone_are_not_red(cx: &mut TestAppContext) {
    let (_s, win, view) = shown(
        cx,
        vec![TestOutcome::new(ScriptPhase::Tests, "ok", Verdict::Pass)],
        "",
    );
    assert!(!present(cx, win, "assertions-summary"));
    assert!(present(cx, win, ("test", 0usize)));
    assert!(!view.read_with(cx, |p, cx| p.count_is_danger(ResponseTab::Tests, cx)));
}

const PASSES: &str = "runtime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n";

fn label(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> Option<String> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

fn tone(cx: &mut TestAppContext, view: &Entity<ResponsePane>) -> Tone {
    view.read_with(cx, |p, cx| p.count_tone(ResponseTab::Tests, cx))
}

fn result_with(
    outcome: Outcome,
    assertions: Vec<Verdict>,
    tests: Vec<TestOutcome>,
) -> RequestResult {
    let assertions = assertions
        .into_iter()
        .map(|v| AssertionOutcome::new(Assertion::new("res.status", "eq", Some("200".into())), v))
        .collect();
    let mut r = RequestResult::from_parts(
        ItemPath::new("a.yml").unwrap(),
        "a",
        outcome,
        None,
        assertions,
    );
    r.tests = tests;
    r
}

#[test]
fn the_total_counts_assertions_and_tests_together() {
    let r = result_with(
        Outcome::Failed,
        vec![Verdict::Pass, Verdict::fail("500", None)],
        outcomes(),
    );
    assert_eq!(
        tests_total(&r).as_deref(),
        Some("2 passed, 2 failed, 1 skipped, 1 errored")
    );
    let r = result_with(Outcome::Passed, vec![Verdict::Pass], vec![]);
    assert_eq!(tests_total(&r).as_deref(), Some("1 passed, 0 failed"));
    let r = result_with(
        Outcome::Passed,
        vec![Verdict::skipped("needs JavaScript")],
        vec![],
    );
    // Every check skipped is not "No tests": the total says what was skipped.
    assert!(!shows_no_tests(&r));
    assert_eq!(
        tests_total(&r).as_deref(),
        Some("0 passed, 0 failed, 1 skipped")
    );
    let none = result_with(Outcome::Passed, vec![], vec![]);
    assert!(shows_no_tests(&none));
    assert_eq!(tests_total(&none), None);
    let errored = result_with(
        Outcome::Failed,
        vec![],
        vec![error("Test script error", "boom")],
    );
    assert!(
        !shows_no_tests(&errored),
        "a failed request never reads as no tests"
    );
    assert_eq!(
        tests_total(&errored).as_deref(),
        Some("0 passed, 0 failed, 1 errored")
    );
}

#[test]
fn a_success_badge_skips_zero() {
    assert!(success_count_badge(0).is_none());
    assert!(success_count_badge(2).is_some());
}

#[gpui_kit::test]
fn one_total_line_sits_over_labelled_groups(cx: &mut TestAppContext) {
    let (_s, win, view) = shown(cx, outcomes(), PASSES);
    assert_eq!(
        label(cx, win, "tests-total").as_deref(),
        Some("2 passed, 1 failed, 1 skipped, 1 errored")
    );
    assert_eq!(
        label(cx, win, "assertions-heading").as_deref(),
        Some("Assertions")
    );
    assert_eq!(label(cx, win, "tests-heading").as_deref(), Some("Tests"));
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let total = w.find("tests-total").bounds();
        let a_head = w.find("assertions-heading").bounds();
        let a_row = w.find(("assertion", 0usize)).bounds();
        let t_head = w.find("tests-heading").bounds();
        let t_row = w.find(("test", 0usize)).bounds();
        assert!(total.bottom() <= a_head.top() && a_head.bottom() <= a_row.top());
        assert!(a_row.bottom() <= t_head.top() && t_head.bottom() <= t_row.top());
    })
    .unwrap();
    assert_eq!(tone(cx, &view), Tone::Danger);
}

#[gpui_kit::test]
fn all_passing_turns_the_badge_green(cx: &mut TestAppContext) {
    let pass = vec![TestOutcome::new(ScriptPhase::Tests, "ok", Verdict::Pass)];
    let (_s, win, view) = shown(cx, pass, "");
    assert_eq!(
        label(cx, win, "tests-total").as_deref(),
        Some("1 passed, 0 failed")
    );
    assert!(
        !present(cx, win, "assertions-heading"),
        "an empty group has no heading"
    );
    assert!(present(cx, win, "tests-heading"));
    assert_eq!(tone(cx, &view), Tone::Success);
}

#[gpui_kit::test]
fn skipped_checks_alone_read_as_skipped_not_no_tests(cx: &mut TestAppContext) {
    let skipped = vec![TestOutcome::new(
        ScriptPhase::Tests,
        "later",
        Verdict::skipped("scripts are off"),
    )];
    let (_s, win, view) = shown(cx, skipped, "");
    assert!(!present(cx, win, "tests-none"));
    assert_eq!(
        label(cx, win, "tests-total").as_deref(),
        Some("0 passed, 0 failed, 1 skipped")
    );
    assert!(
        present(cx, win, ("test", 0usize)),
        "the skipped entry still says why"
    );
    assert_eq!(tone(cx, &view), Tone::Muted);
}

#[gpui_kit::test]
fn a_pass_beside_a_skip_is_a_warning_not_green(cx: &mut TestAppContext) {
    let mixed = vec![
        TestOutcome::new(ScriptPhase::Tests, "ok", Verdict::Pass),
        TestOutcome::new(
            ScriptPhase::Tests,
            "later",
            Verdict::skipped("scripts are off"),
        ),
    ];
    let (_s, win, view) = shown(cx, mixed, "");
    assert_eq!(
        label(cx, win, "tests-total").as_deref(),
        Some("1 passed, 0 failed, 1 skipped")
    );
    assert_eq!(tone(cx, &view), Tone::Warning);
    let badge = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find(("response-count", ResponseTab::Tests as usize))
                .and_then(|e| e.label().map(str::to_owned))
        })
        .unwrap();
    assert_eq!(badge.as_deref(), Some("1 passed, 1 skipped"));
}

fn tests_badge(cx: &mut TestAppContext, win: AnyWindowHandle) -> Option<String> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(("response-count", ResponseTab::Tests as usize))
            .and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

// A failure badge says how many failed and how many scripts errored, counted as the total
// line counts them, rather than the count of every check.
#[gpui_kit::test]
fn a_failure_badge_says_how_many_failed(cx: &mut TestAppContext) {
    let (_s, win, view) = shown(cx, outcomes(), PASSES);
    assert_eq!(tone(cx, &view), Tone::Danger);
    assert_eq!(tests_badge(cx, win).as_deref(), Some("1 failed, 1 errored"));
}

#[gpui_kit::test]
fn a_script_error_alone_reads_as_errored_not_failed(cx: &mut TestAppContext) {
    let tests = vec![
        TestOutcome::new(ScriptPhase::Tests, "ok", Verdict::Pass),
        error(
            "Post-response script error",
            "this request's script, line 3: boom",
        ),
    ];
    let (_s, win, view) = shown(cx, tests, "");
    assert_eq!(tone(cx, &view), Tone::Danger);
    assert_eq!(
        label(cx, win, "tests-total").as_deref(),
        Some("1 passed, 0 failed, 1 errored")
    );
    assert_eq!(tests_badge(cx, win).as_deref(), Some("1 errored"));
    let row = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find(("test-text", 1usize))
                .and_then(|e| e.label().map(str::to_owned))
        })
        .unwrap();
    assert_eq!(
        row.as_deref(),
        Some("Post-response script error, this request's script, line 3: boom")
    );
}

#[gpui_kit::test]
fn a_request_without_checks_says_no_tests(cx: &mut TestAppContext) {
    let (_s, win, view) = shown(cx, Vec::new(), "");
    assert_eq!(label(cx, win, "tests-none").as_deref(), Some("No tests"));
    assert!(!present(cx, win, "assertions-heading") && !present(cx, win, "tests-heading"));
    assert_eq!(tone(cx, &view), Tone::Muted);
}

#[gpui_kit::test]
fn a_warning_reads_as_a_sentence(cx: &mut TestAppContext) {
    let s = send_once(cx, ResponseTemplate::new(200), "");
    let mut r = (*s.result).clone();
    r.warnings = vec![apinata_core::Warning::ScriptsNotRun];
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
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |p, cx| p.warnings(cx)),
        ["Scripts not executed"]
    );
    assert!(present(cx, win, ("response-warning", 0usize)));
}

#[gpui_kit::test]
fn a_warning_that_starts_with_a_file_key_keeps_its_case(cx: &mut TestAppContext) {
    let s = send_once(cx, ResponseTemplate::new(200), "");
    let mut r = (*s.result).clone();
    r.warnings = vec![
        apinata_core::Warning::EncodeUrlIgnored,
        apinata_core::Warning::ScriptsNotRun,
    ];
    s.model
        .update(cx, |m, cx| m.show(std::sync::Arc::new(r), cx));
    support::init(cx);
    let model = s.model.clone();
    let (_win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |p, cx| p.warnings(cx)),
        [
            "encodeUrl: false is not supported; the URL is normalized",
            "Scripts not executed"
        ]
    );
}

#[gpui_kit::test]
fn dropped_url_credentials_read_as_a_sentence(cx: &mut TestAppContext) {
    let s = send_once(cx, ResponseTemplate::new(200), "");
    let mut r = (*s.result).clone();
    r.warnings = vec![apinata_core::Warning::UrlCredentialsDropped];
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
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |p, cx| p.warnings(cx)),
        ["Credentials in the URL were not sent because the request has path params"]
    );
    assert!(present(cx, win, ("response-warning", 0usize)));
}
