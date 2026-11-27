//! The opt-in `response` object of the JSON report.

use apinata_core::{ItemPath, Outcome, RequestResult, Response, ResponseDetail, ResultsReport};

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
    let report = ResultsReport::new(vec![], results.iter())
        .with_responses(results.iter(), &ResponseDetail::default());
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
    let report = ResultsReport::new(vec![], results.iter())
        .with_responses(results.iter(), &ResponseDetail::default());
    let r = &json(&report)["results"][0]["response"];
    assert_eq!(r["body"], serde_json::Value::Null);
    assert_eq!(r["body_size"], 99);
    assert_eq!(r["truncated"], true);
}

fn response_with(detail: &ResponseDetail) -> serde_json::Value {
    let answer = answer().with_header("Set-Cookie", "sid=1");
    let results = [result(Some(answer))];
    let report = ResultsReport::new(vec![], results.iter()).with_responses(results.iter(), detail);
    json(&report)["results"][0]["response"].clone()
}

#[test]
fn omitting_headers_drops_the_headers_key_and_keeps_the_body() {
    let r = response_with(&ResponseDetail::new().omit_headers());
    assert!(r.get("headers").is_none(), "{r}");
    assert_eq!(r["body"], "{\"id\":7}");
}

#[test]
fn omitting_a_named_header_drops_only_that_pair_whatever_its_case() {
    let r = response_with(&ResponseDetail::new().omit_header("SET-cookie"));
    assert_eq!(r["headers"], serde_json::json!([["x-a", "1"]]));
}

#[test]
fn omitting_the_body_drops_the_body_key_and_keeps_its_size() {
    let r = response_with(&ResponseDetail::new().omit_body());
    assert!(r.get("body").is_none(), "{r}");
    assert_eq!(r["body_size"], 8);
    assert_eq!(r["truncated"], false);
    assert_eq!(r["headers"].as_array().map(Vec::len), Some(2));
}

#[test]
fn the_default_detail_keeps_headers_and_body() {
    let r = response_with(&ResponseDetail::default());
    assert_eq!(r["headers"].as_array().map(Vec::len), Some(2));
    assert_eq!(r["body"], "{\"id\":7}");
}
