//! `ResultsReport`: the one report model the CLI and the app render from.

use apinata_core::{
    Assertion, AssertionOutcome, ItemPath, Outcome, RequestResult, ResultsReport, Verdict,
};

const TS: &str = "2026-10-12T10:00:00.000";

fn results() -> Vec<RequestResult> {
    let path = ItemPath::new("users/list.yml").expect("a valid item path");
    let passed = AssertionOutcome::new(
        Assertion::new("res.status", "eq", Some("200".into())),
        Verdict::Pass,
    );
    let failed = AssertionOutcome::new(
        Assertion::new("res.status", "eq", Some("200".into())),
        Verdict::fail("500", None),
    );
    vec![
        RequestResult::from_parts(
            path.clone(),
            "List <users>",
            Outcome::Passed,
            None,
            vec![passed],
        ),
        RequestResult::from_parts(path, "Broken", Outcome::Failed, None, vec![failed]),
    ]
}

fn report() -> ResultsReport {
    let load_errors = vec![("bad.yml".to_string(), "bad yaml".to_string())];
    ResultsReport::new(load_errors, &results())
}

#[test]
fn json_is_pretty_with_a_trailing_newline_and_the_spec_keys() {
    let json = report().to_json();
    assert!(json.ends_with("}\n"), "{json}");
    let value: serde_json::Value = serde_json::from_str(&json).expect("valid json");
    assert_eq!(value["apinata"], 1);
    assert_eq!(value["summary"]["requests"], 2);
    assert_eq!(value["summary"]["failed"], 1);
    assert_eq!(value["load_errors"][0]["path"], "bad.yml");
    assert_eq!(value["results"][1]["assertions"][0]["verdict"], "fail");
}

#[test]
fn junit_carries_the_caller_supplied_stamp() {
    let xml = report().to_junit(TS, "ci-box", false);
    assert!(
        xml.starts_with("<?xml version=\"1.0\"?>\n<testsuites>"),
        "{xml}"
    );
    assert!(xml.contains(r#"timestamp="2026-10-12T10:00:00.000" hostname="ci-box""#));
    assert!(xml.contains(r#"<testsuite name="Broken""#));
}

#[test]
fn html_is_one_escaped_page_stamped_by_the_caller() {
    let html = report().to_html("2026-10-12 10:00:00 UTC");
    assert!(html.starts_with("<!doctype html>"));
    assert!(html.contains("2026-10-12 10:00:00 UTC"));
    assert!(html.contains("List &lt;users&gt;"));
}

#[test]
fn exit_code_follows_load_errors_and_failures() {
    assert_eq!(report().exit_code(), 3);
    let failing = ResultsReport::new(Vec::new(), &results());
    assert_eq!(failing.exit_code(), 1);
    assert_eq!(
        ResultsReport::new(Vec::new(), &results()[..1]).exit_code(),
        0
    );
}
