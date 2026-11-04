//! The public assertion entry point over a collection on disk.

use crate::common;

use apinata_core::{Collection, ContextOptions, Response, VarContext, Verdict, evaluate};
use common::{request, write_tree};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nrequest:\n  variables:\n    - name: code\n      value: \"201\"\n";
const FOLDER: &str = "info:\n  name: a\n  type: folder\nrequest:\n  variables:\n    - name: kind\n      value: user\n";
const REQ: &str = concat!(
    "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: https://x.test\n",
    "runtime:\n  variables:\n    - name: who\n      value: ann\n  assertions:\n",
    "    - expression: res.status\n      operator: eq\n      value: \"{{code}}\"\n",
    "    - expression: res.body.kind\n      operator: eq\n      value: \"{{kind}}\"\n",
    "    - expression: res.body.name\n      operator: in\n      value: \"[bob, {{who}}]\"\n",
    "    - expression: res.headers['x-id']\n      operator: eq\n      value: \"'7'\"\n",
    "    - expression: res.body.name\n      operator: eq\n      value: \"{{nope}}\"\n",
    "    - expression: res.url\n      operator: isDefined\n",
    "    - expression: res.status\n      operator: eq\n      value: \"500\"\n      disabled: true\n",
);

#[test]
fn evaluate_interpolates_collection_folder_and_request_variables() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a/folder.yml", FOLDER),
        ("a/r.yml", REQ),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    assert!(c.load_errors().is_empty(), "{:?}", c.load_errors());
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let res = Response::new(201)
        .with_header("X-Id", "7")
        .with_text_body("{\"kind\":\"user\",\"name\":\"ann\"}");
    let outcomes = evaluate(request(&c, "a/r.yml"), &res, &ctx);
    // Verdict variants are non-exhaustive, so each outcome is compared through a plain shape.
    let shapes: Vec<_> = outcomes
        .iter()
        .map(|o| match o.verdict() {
            Verdict::Pass => (o.assertion().expression(), "pass", None, None),
            Verdict::Fail { actual, detail, .. } => (
                o.assertion().expression(),
                "fail",
                Some(actual.clone()),
                detail.clone(),
            ),
            Verdict::Skipped { reason, .. } => (
                o.assertion().expression(),
                "skipped",
                Some(reason.clone()),
                None,
            ),
            other => panic!("unexpected verdict {other:?}"),
        })
        .collect();
    let undefined = Some("variable `{{nope}}` is undefined".to_owned());
    let skip = Some("left side `res.url` needs a JavaScript runtime".to_owned());
    assert_eq!(
        shapes,
        [
            ("res.status", "pass", None, None),
            ("res.body.kind", "pass", None, None),
            ("res.body.name", "pass", None, None),
            ("res.headers['x-id']", "pass", None, None),
            (
                "res.body.name",
                "fail",
                Some("\"ann\"".to_owned()),
                undefined
            ),
            ("res.url", "skipped", skip, None),
        ]
    );
}

#[test]
fn a_truncated_body_skips_body_assertions_but_not_status_or_headers() {
    let req = concat!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: https://x.test\n",
        "runtime:\n  assertions:\n",
        "    - expression: res.status\n      operator: eq\n      value: \"200\"\n",
        "    - expression: res.headers['x-id']\n      operator: eq\n      value: \"'7'\"\n",
        "    - expression: res.body.kind\n      operator: eq\n      value: user\n",
        "    - expression: res.body\n      operator: isDefined\n",
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", req)]);
    let c = Collection::open(dir.path()).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let res = Response::new(200)
        .with_header("x-id", "7")
        .with_text_body("{\"kind\":\"us")
        .with_truncated(4096);
    assert!(res.truncated());
    assert_eq!(res.size(), 4096);
    let verdicts: Vec<_> = evaluate(request(&c, "r.yml"), &res, &ctx)
        .iter()
        .map(|o| match o.verdict() {
            Verdict::Pass => "pass".to_owned(),
            Verdict::Skipped { reason, .. } => reason.clone(),
            other => panic!("unexpected verdict {other:?}"),
        })
        .collect();
    assert_eq!(
        verdicts,
        [
            "pass",
            "pass",
            "the response body was truncated at the size limit, so `res.body.kind` was not evaluated",
            "the response body was truncated at the size limit, so `res.body` was not evaluated",
        ]
    );
}
