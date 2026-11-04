//! `RequestResult::is_untested`: true when no assertion and no script test reached a verdict.

use apinata_core::{
    Assertion, AssertionOutcome, ItemPath, Outcome, RequestResult, ScriptPhase, TestOutcome,
    Verdict,
};

fn result(assertions: Vec<Verdict>, tests: Vec<TestOutcome>) -> RequestResult {
    let path = ItemPath::new("a.yml").expect("a valid item path");
    let assertions = assertions
        .into_iter()
        .map(|v| AssertionOutcome::new(Assertion::new("res.status", "eq", Some("200".into())), v))
        .collect();
    let mut r = RequestResult::from_parts(path, "a", Outcome::Passed, None, assertions);
    r.tests = tests;
    r
}

fn test(verdict: Verdict) -> TestOutcome {
    TestOutcome::new(ScriptPhase::Tests, "t", verdict)
}

#[test]
fn a_request_with_no_checks_is_untested() {
    assert!(result(vec![], vec![]).is_untested());
}

#[test]
fn skipped_checks_alone_leave_it_untested() {
    let r = result(
        vec![Verdict::skipped("needs JavaScript")],
        vec![test(Verdict::skipped("scripts are off"))],
    );
    assert!(r.is_untested());
}

#[test]
fn a_pass_or_a_fail_of_either_kind_is_a_verdict() {
    assert!(!result(vec![Verdict::Pass], vec![]).is_untested());
    assert!(!result(vec![Verdict::fail("500", None)], vec![]).is_untested());
    assert!(!result(vec![], vec![test(Verdict::Pass)]).is_untested());
    let mixed = result(
        vec![Verdict::skipped("x")],
        vec![test(Verdict::fail(
            "",
            Some("expected 1 to equal 2".into()),
        ))],
    );
    assert!(!mixed.is_untested());
}

#[test]
fn a_script_error_entry_is_not_a_test() {
    let mut e = test(Verdict::fail("", Some("boom".into())));
    e.script_error = true;
    assert!(result(vec![], vec![e]).is_untested());
}

#[test]
fn a_passed_request_whose_checks_were_all_skipped_is_unverified() {
    let r = result(vec![Verdict::skipped("needs JavaScript")], vec![]);
    assert!(r.is_unverified());
    let by_test = result(vec![], vec![test(Verdict::skipped("scripts are off"))]);
    assert!(by_test.is_unverified());
}

#[test]
fn a_request_with_no_checks_or_a_verdict_is_not_unverified() {
    assert!(!result(vec![], vec![]).is_unverified());
    assert!(!result(vec![Verdict::Pass, Verdict::skipped("x")], vec![]).is_unverified());
    let mut e = test(Verdict::fail("", Some("boom".into())));
    e.script_error = true;
    assert!(!result(vec![], vec![e]).is_unverified());
}

#[test]
fn a_passed_request_with_no_checks_at_all_has_no_checks() {
    assert!(result(vec![], vec![]).has_no_checks());
    let mut e = test(Verdict::fail("", Some("boom".into())));
    e.script_error = true;
    assert!(result(vec![], vec![e]).has_no_checks());
}

#[test]
fn any_check_or_a_non_passed_outcome_means_it_has_checks() {
    assert!(!result(vec![Verdict::Pass], vec![]).has_no_checks());
    assert!(!result(vec![Verdict::skipped("x")], vec![]).has_no_checks());
    assert!(!result(vec![], vec![test(Verdict::skipped("x"))]).has_no_checks());
    let mut r = result(vec![], vec![]);
    r.outcome = Outcome::Failed;
    assert!(!r.has_no_checks());
}

#[test]
fn only_a_passed_outcome_can_be_unverified() {
    let mut r = result(vec![Verdict::skipped("x")], vec![]);
    r.outcome = Outcome::Failed;
    assert!(!r.is_unverified());
}
