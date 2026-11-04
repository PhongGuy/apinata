use apinata_core::{
    Assertion, AssertionOutcome, ItemPath, Outcome, RequestResult, Response, Verdict,
};

#[test]
fn a_result_is_rebuilt_from_its_parts() {
    let response = Response::new(201).with_text_body("hi");
    let pass = AssertionOutcome::new(
        Assertion::new("res.status", "eq", Some("201".into())),
        Verdict::Pass,
    );
    let r = RequestResult::from_parts(
        ItemPath::new("users/u.yml").unwrap(),
        "u",
        Outcome::Passed,
        Some(response.clone()),
        vec![pass.clone()],
    );
    assert_eq!(r.path.as_str(), "users/u.yml");
    assert_eq!(r.name, "u");
    assert_eq!(r.outcome, Outcome::Passed);
    assert!(r.error.is_none());
    assert!(r.warnings.is_empty());
    assert_eq!(r.response, Some(response));
    assert_eq!(r.assertions, vec![pass]);
}

#[test]
fn verdict_constructors_fill_their_variants() {
    match Verdict::fail("404", Some("why".into())) {
        Verdict::Fail { actual, detail, .. } => {
            assert_eq!(actual, "404");
            assert_eq!(detail.as_deref(), Some("why"));
        }
        other => panic!("{other:?}"),
    }
    match Verdict::skipped("needs js") {
        Verdict::Skipped { reason, .. } => assert_eq!(reason, "needs js"),
        other => panic!("{other:?}"),
    }
    let o = AssertionOutcome::new(
        Assertion::new("res.body", "isJson", None),
        Verdict::skipped("x"),
    );
    assert_eq!(o.assertion().expression(), "res.body");
    assert_eq!(o.assertion().value(), None);
    assert_eq!(o.verdict(), &Verdict::skipped("x"));
}

#[test]
fn with_size_sets_the_count_without_marking_truncation() {
    let r = Response::new(200).with_text_body("").with_size(1234);
    assert_eq!(r.size(), 1234);
    assert!(!r.truncated());
    assert!(r.body().is_empty());
}
