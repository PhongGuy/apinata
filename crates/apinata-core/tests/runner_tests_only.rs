//! Narrowing a run plan to the requests that check something.

use crate::common;

use apinata_core::{Collection, RunPlan, TagFilter};
use common::write_tree;

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

fn req(name: &str, seq: u32, extra: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"http://127.0.0.1:1/{name}\"\n{extra}"
    )
}

fn script(kind: &str, code: &str) -> String {
    format!("runtime:\n  scripts:\n    - type: {kind}\n      code: |-\n        {code}\n")
}

const ASSERTION: &str = "runtime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n";
const DISABLED: &str = "runtime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n      disabled: true\n";

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "folder/folder.yml",
            "info:\n  name: folder\n  type: folder\n  tags:\n    - smoke\nrequest:\n  scripts:\n    - type: tests\n      code: |-\n        test(\"folder\", () => {});\n",
        ),
        ("folder/inherits.yml", &req("inherits", 1, "")),
        ("a_assert.yml", &req("a_assert", 1, ASSERTION)),
        ("b_disabled.yml", &req("b_disabled", 2, DISABLED)),
        (
            "c_tests.yml",
            &req("c_tests", 3, &script("tests", "test(\"ok\", () => {});")),
        ),
        (
            "d_pre.yml",
            &req(
                "d_pre",
                4,
                &script("before-request", "test (\"pre\", () => {});"),
            ),
        ),
        (
            "e_post.yml",
            &req(
                "e_post",
                5,
                &script("after-response", "if (x) { test(\"p\", f) }"),
            ),
        ),
        (
            "f_comment.yml",
            &req("f_comment", 6, &script("tests", "// test(\"no\")")),
        ),
        (
            "g_string.yml",
            &req("g_string", 7, &script("tests", "const s = \"test(1)\";")),
        ),
        (
            "h_property.yml",
            &req(
                "h_property",
                8,
                &script("tests", "pm.test(\"x\", f); /a/.test(s);"),
            ),
        ),
        (
            "i_ident.yml",
            &req(
                "i_ident",
                9,
                &script("tests", "testFoo(); mytest(); const test = 1;"),
            ),
        ),
        ("j_plain.yml", &req("j_plain", 10, "")),
        ("bad.yml", "http: [broken\n"),
    ])
}

fn names(plan: &RunPlan) -> Vec<&str> {
    plan.items().map(|i| i.as_str()).collect()
}

#[test]
fn only_requests_with_an_enabled_assertion_or_a_test_call_are_kept() {
    let dir = tree();
    let c = Collection::open(dir.path()).expect("opens");
    let plan = c.plan(dir.path()).expect("plan");
    let total = plan.items().count();
    let kept = plan.retain_tested();
    assert_eq!(
        names(&kept),
        ["a_assert.yml", "c_tests.yml", "d_pre.yml", "e_post.yml"]
    );
    assert_eq!(kept.untested_dropped(), total - 4);
    assert_eq!(kept.load_errors().len(), 1, "load errors are kept");
}

#[test]
fn the_count_covers_only_requests_the_tag_filter_kept() {
    let dir = tree();
    let c = Collection::open(dir.path()).expect("opens");
    let filter = TagFilter::new(vec!["smoke".to_owned()], Vec::new());
    let plan = c
        .plan(&dir.path().join("folder"))
        .expect("plan")
        .filter_tags(&filter)
        .retain_tested();
    assert!(names(&plan).is_empty(), "folder-level tests do not count");
    assert_eq!(plan.untested_dropped(), 1);
    assert_eq!(
        plan.empty_message(&filter).as_deref(),
        Some("no requests matched: tags smoke; with tests only (1 skipped)")
    );
}

#[test]
fn a_single_request_without_tests_leaves_nothing_to_run() {
    let dir = tree();
    let c = Collection::open(dir.path()).expect("opens");
    let plan = c
        .plan(&dir.path().join("j_plain.yml"))
        .expect("plan")
        .retain_tested();
    assert_eq!(plan.untested_dropped(), 1);
    assert_eq!(
        plan.empty_message(&TagFilter::default()).as_deref(),
        Some("no requests with tests to run (1 skipped)")
    );
}

#[test]
fn an_unfiltered_plan_drops_nothing() {
    let dir = tree();
    let c = Collection::open(dir.path()).expect("opens");
    let plan = c.plan(dir.path()).expect("plan");
    assert_eq!(plan.untested_dropped(), 0);
}
