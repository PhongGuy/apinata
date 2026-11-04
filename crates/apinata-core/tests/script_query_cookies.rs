//! res(path) matches the reference query suite; bru.cookies reads the jar for the request URL.

use crate::script_support;

use apinata_core::{Outcome, VarResult};
use script_support::*;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const CASES: &str = r#"const cases = [
  ["customer.address.city", "bangalore"],
  ["customer.orders.items.amount", [10, 20, 30, 40]],
  ["customer.orders.items.amount[0]", 10],
  ["..items.amount", [10, 20, 30, 40]],
  ["..amount", [10, 20, 30, 40]],
  ["..items.amount[0]", 10],
  ["..items[0].amount", 10],
  ["..items[5].amount", undefined],
  ["..id", ["order-1", 1, 2, "order-2", 3, 4]],
  ["customer.orders.foo", undefined],
  ["..customer.foo", undefined],
  ["..address", [{ city: "bangalore" }]],
  ["..address[0]", { city: "bangalore" }],
  ["..items[?].amount", [40], (i) => i.amount > 30],
  ["..items[?].amount", [40], { id: 4, amount: 40 }],
  ["..items[?].amount", undefined, { id: 5, amount: 40 }],
  ["..items..amount[?][0]", 40, (amt) => amt > 30],
  ["..items..amount[0][?]", undefined, (amt) => amt > 30],
  ["..items..amount[?]", [11, 21, 31, 41], (amt) => amt + 1],
  ["..items..amount[0][?]", 11, (amt) => amt + 1],
];
const bad = cases
  .filter(([expr, want, fn]) => JSON.stringify(fn === undefined ? res(expr) : res(expr, fn)) !== JSON.stringify(want))
  .map(([expr]) => expr);
console.log(bad.length ? bad.join(" | ") : "all match");"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn res_queries_match_the_reference_suite() {
    let s = MockServer::start().await;
    let data = serde_json::json!({"customer": {"address": {"city": "bangalore"}, "orders": [
        {"id": "order-1", "items": [{"id": 1, "amount": 10}, {"id": 2, "amount": 20}]},
        {"id": "order-2", "items": [{"id": 3, "amount": 30}, {"id": 4, "amount": 40}]}]}});
    Mock::given(path("/r"))
        .respond_with(ResponseTemplate::new(200).set_body_json(data))
        .mount(&s)
        .await;
    let text = scripted("R", &format!("{}/r", s.uri()), &script("tests", CASES), "");
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(console(&result), ["log:all match"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_post_response_variable_reads_a_res_query() {
    let s = MockServer::start().await;
    let body = serde_json::json!({"data": {"items": [{"id": 7}]}});
    Mock::given(path("/r"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&s)
        .await;
    let extra = "  actions:\n    - type: set-variable\n      phase: after-response\n      selector:\n        expression: \"res('data.items.0.id')\"\n        method: jsonq\n      variable:\n        name: id\n        scope: runtime\n";
    let text = scripted(
        "R",
        &format!("{}/r", s.uri()),
        &script("tests", "console.log(bru.getVar('id'))"),
        extra,
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.post_response_vars[0].result(), &VarResult::Set);
    assert_eq!(console(&result), ["log:7"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cookies_read_the_jar_for_the_request_url() {
    let s = MockServer::start().await;
    Mock::given(path("/login"))
        .respond_with(
            ResponseTemplate::new(200)
                .append_header("set-cookie", "sid=abc; Path=/")
                .append_header("set-cookie", "theme=dark; Path=/; HttpOnly"),
        )
        .mount(&s)
        .await;
    let before = script("before-request", "console.log(bru.cookies.count())");
    let after = script(
        "after-response",
        r#"const c = bru.cookies;
console.log(c.get("sid"), c.has("sid"), c.has("sid", "zzz"), c.count(), JSON.stringify(Object.keys(c.toObject()).sort()), c.one("theme").httpOnly, c.all().length, c.get("none"));"#,
    );
    let text = scripted(
        "R",
        &format!("{}/login", s.uri()),
        &format!("{before}{after}"),
        "",
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.tests);
    assert_eq!(
        console(&result),
        [
            "log:0",
            r#"log:abc true false 2 ["sid","theme"] true 2 undefined"#,
        ]
    );
}
