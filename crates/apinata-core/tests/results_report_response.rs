//! The opt-in `response` object of the JSON report.

use apinata_core::{ItemPath, Outcome, RequestResult, Response, ResultsReport};

fn result(response: Option<Response>) -> RequestResult {
    let path = ItemPath::new("users/get.yml").expect("a valid item path");
    RequestResult::from_parts(path, "Get", Outcome::Passed, response, Vec::new())
}

fn json(report: &ResultsReport) -> serde_json::Value {
    serde_json::from_str(&report.to_json()).expect("the report is JSON")
}

fn answer() -> Response {
    Response::new(200)
        .with_status_text("OK")
        .with_header("X-A", "1")
        .with_text_body("{\"id\":7}")
}

#[test]
fn without_the_builder_the_report_has_no_response_key() {
    let results = [result(Some(answer()))];
    let report = ResultsReport::new(vec![], results.iter());
    assert!(json(&report)["results"][0].get("response").is_none());
}

#[test]
fn the_builder_adds_status_headers_body_and_size() {
    let results = [result(Some(answer())), result(None)];
    let report = ResultsReport::new(vec![], results.iter()).with_responses(results.iter());
    let v = json(&report);
    assert_eq!(
        v["results"][0]["response"],
        serde_json::json!({
            "status": 200,
            "headers": [["x-a", "1"]],
            "body": "{\"id\":7}",
            "body_size": 8,
            "truncated": false
        })
    );
    assert!(v["results"][1].get("response").is_none());
}

#[test]
fn a_non_utf8_body_is_null_and_a_cut_body_says_so() {
    let cut = Response::new(200)
        .with_body(vec![0xff, 0xfe], "")
        .with_truncated(99);
    let results = [result(Some(cut))];
    let report = ResultsReport::new(vec![], results.iter()).with_responses(results.iter());
    let r = &json(&report)["results"][0]["response"];
    assert_eq!(r["body"], serde_json::Value::Null);
    assert_eq!(r["body_size"], 99);
    assert_eq!(r["truncated"], true);
}
