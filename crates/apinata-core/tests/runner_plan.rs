//! Selecting what a run sends: Bruno's order, the load errors that bear on it, and refusals.

use crate::common;

use apinata_core::{Collection, RunPlan, SelectError};
use common::write_tree;
use std::path::Path;

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

fn req(name: &str, seq: u32) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: https://x.test/{name}\n"
    )
}

/// Request names in run order and sorted load error paths of `plan`, relative to `root`.
fn shape(plan: &RunPlan, root: &Path) -> (Vec<String>, Vec<String>) {
    let names = plan.requests().map(|r| r.name().to_owned()).collect();
    let mut errors: Vec<String> = plan
        .load_errors()
        .iter()
        .map(|e| {
            let rel = e.path().strip_prefix(root).expect("inside root");
            rel.to_string_lossy().replace('\\', "/")
        })
        .collect();
    errors.sort();
    (names, errors)
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn plans_follow_brunos_order_and_keep_only_the_load_errors_that_bear_on_them() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r2.yml", &req("r2", 2)),
        (
            "r1.yml",
            &format!(
                "{}  headers:\n    - name: Authorization\n      value: Bearer SECRET\n",
                req("r1", 1)
            ),
        ),
        ("f/folder.yml", "info: [broken\n"),
        ("f/x.yml", &req("x", 1)),
        ("f/sub/y.yml", &req("y", 1)),
        ("f/bad.yml", "http: [broken\n"),
        ("g/bad.yml", "http: [broken\n"),
        ("environments/dev.yml", "name: dev\nvariables: []\n"),
    ]);
    let root = dir.path();
    let c = Collection::open(root).expect("opens");
    let plan = |rel: &str| c.plan(&root.join(rel));
    // A plan keeps the collection's own load-error order, which the CLI prints as is.
    let in_order = |plan: &RunPlan, keep: &[&str]| {
        let kept: Vec<&Path> = plan.load_errors().iter().map(|e| e.path()).collect();
        let expected: Vec<&Path> = c
            .load_errors()
            .iter()
            .map(|e| e.path())
            .filter(|p| keep.iter().any(|k| *p == root.join(k)))
            .collect();
        assert_eq!(kept, expected);
    };
    in_order(
        &plan("").expect("root"),
        &["f/bad.yml", "f/folder.yml", "g/bad.yml"],
    );
    in_order(&plan("f/sub").expect("folder"), &["f/folder.yml"]);

    assert_eq!(
        shape(&plan("").expect("root"), root),
        (
            strings(&["y", "x", "r1", "r2"]),
            strings(&["f/bad.yml", "f/folder.yml", "g/bad.yml"])
        )
    );
    assert_eq!(
        shape(&plan("f/sub").expect("folder"), root),
        (strings(&["y"]), strings(&["f/folder.yml"]))
    );
    let one = plan("r1.yml").expect("request");
    assert_eq!(shape(&one, root), (strings(&["r1"]), strings(&[])));
    let shown = format!("{one:?}");
    assert!(
        shown.contains("r1.yml") && !shown.contains("SECRET"),
        "{shown}"
    );
    // A request file that failed to load selects nothing; its error is the whole plan.
    assert_eq!(
        shape(&plan("g/bad.yml").expect("broken request"), root),
        (strings(&[]), strings(&["g/bad.yml"]))
    );
    match plan("environments/dev.yml") {
        Err(e @ SelectError::Environment { .. }) => assert_eq!(
            e.to_string(),
            format!(
                "{} is an environment, not a request",
                root.join("environments/dev.yml").display()
            )
        ),
        other => panic!("expected Environment, got {other:?}"),
    }
    match plan("nope.yml") {
        Err(e @ SelectError::NotAnItem { .. }) => assert_eq!(
            e.to_string(),
            format!(
                "{} is not the collection, a folder or a request file",
                root.join("nope.yml").display()
            )
        ),
        other => panic!("expected NotAnItem, got {other:?}"),
    }
}

/// A request whose file name cannot be an item path is reported, never dropped in silence.
#[cfg(unix)]
#[test]
fn a_request_without_an_item_path_becomes_a_load_error() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a:b.yml", &req("ab", 1)),
        ("ok.yml", &req("ok", 2)),
    ]);
    let root = dir.path();
    let c = Collection::open(root).expect("opens");
    assert_eq!(c.tree().requests().len(), 2);
    let plan = c.plan(root).expect("root");
    assert_eq!(
        shape(&plan, root),
        (strings(&["ok"]), strings(&["a:b.yml"]))
    );
    assert!(
        plan.load_errors()[0].message().contains("item path"),
        "{:?}",
        plan.load_errors()
    );
}

#[test]
fn items_name_each_planned_request_by_its_item_path_in_run_order() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r2.yml", &req("r2", 2)),
        ("r1.yml", &req("r1", 1)),
        ("f/x.yml", &req("x", 1)),
        ("f/sub/y.yml", &req("y", 1)),
    ]);
    let root = dir.path();
    let c = Collection::open(root).expect("opens");
    let items =
        |plan: &RunPlan| -> Vec<String> { plan.items().map(|p| p.as_str().to_owned()).collect() };
    let whole = c.plan(root).expect("plan");
    assert_eq!(
        items(&whole),
        ["f/sub/y.yml", "f/x.yml", "r1.yml", "r2.yml"]
    );
    // Items pair one to one with the requests, in the same order.
    let names: Vec<&str> = whole.requests().map(|r| r.name()).collect();
    assert_eq!(names, ["y", "x", "r1", "r2"]);
    // A folder or a single request still names its items from the collection root.
    let folder = c.plan(&root.join("f")).expect("plan");
    assert_eq!(items(&folder), ["f/sub/y.yml", "f/x.yml"]);
    let one = c.plan(&root.join("r1.yml")).expect("plan");
    assert_eq!(items(&one), ["r1.yml"]);
}
