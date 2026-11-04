//! Planning the requests under a set of items: run order, skipped members, load errors.

use crate::common;

use apinata_core::{Collection, ItemPath, RunPlan};
use common::write_tree;
use std::path::Path;

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";
const SOCKET: &str = "info:\n  name: Socket\n  type: websocket\n  seq: 2\n";

fn req(name: &str, seq: u32) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: https://x.test/{name}\n"
    )
}

fn ip(p: &str) -> ItemPath {
    ItemPath::new(p).unwrap()
}

fn open() -> (tempfile::TempDir, Collection) {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r1.yml", &req("r1", 1)),
        ("r2.yml", &req("r2", 2)),
        ("w.yml", SOCKET),
        (
            "f/folder.yml",
            "info:\n  name: f\n  type: folder\n  seq: 1\n",
        ),
        ("f/x.yml", &req("x", 1)),
        ("f/sub/y.yml", &req("y", 1)),
        ("f/bad.yml", "http: [broken\n"),
        ("g/folder.yml", "info: [broken\n"),
        ("g/z.yml", &req("z", 1)),
        ("h/bad.yml", "http: [broken\n"),
        ("h/q.yml", &req("q", 1)),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    (dir, c)
}

fn names(plan: &RunPlan) -> Vec<String> {
    plan.requests().map(|r| r.name().to_owned()).collect()
}

fn errors(plan: &RunPlan, root: &Path) -> Vec<String> {
    let mut out: Vec<String> = plan
        .load_errors()
        .iter()
        .map(|e| {
            e.path()
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    out.sort();
    out
}

#[test]
fn a_request_a_folder_and_its_descendants_are_planned_in_run_order() {
    let (_dir, c) = open();
    assert_eq!(names(&c.plan_selected(&[ip("r2.yml")])), ["r2"]);
    // Subfolders run before a folder's own requests.
    assert_eq!(names(&c.plan_selected(&[ip("f")])), ["y", "x"]);
    assert_eq!(
        names(&c.plan_selected(&[ip("r1.yml"), ip("f")])),
        ["y", "x", "r1"]
    );
}

#[test]
fn the_order_and_overlap_of_the_argument_never_matter() {
    let (_dir, c) = open();
    let a = names(&c.plan_selected(&[ip("r2.yml"), ip("f"), ip("r1.yml")]));
    let b = names(&c.plan_selected(&[ip("f"), ip("r1.yml"), ip("r2.yml"), ip("f/x.yml"), ip("f")]));
    assert_eq!(a, ["y", "x", "r1", "r2"]);
    assert_eq!(
        b, a,
        "a child of a selected folder and a repeat add nothing"
    );
}

#[test]
fn a_deleted_member_and_an_opaque_member_are_skipped_and_the_survivors_planned() {
    let (_dir, c) = open();
    let plan = c.plan_selected(&[
        ip("gone.yml"),
        ip("w.yml"),
        ip("r1.yml"),
        ip("nope/deeper.yml"),
    ]);
    assert_eq!(names(&plan), ["r1"]);
    assert!(
        c.plan_selected(&[ip("gone.yml"), ip("w.yml")])
            .requests()
            .next()
            .is_none()
    );
    assert!(c.plan_selected(&[]).requests().next().is_none());
}

#[test]
fn a_draft_member_is_skipped() {
    let (_dir, mut c) = open();
    let draft = c.create_draft(&ItemPath::root()).expect("a draft");
    assert!(draft.is_draft());
    assert!(c.plan_selected(&[draft]).requests().next().is_none());
}

#[test]
fn load_errors_are_kept_per_selected_item_and_never_for_a_skipped_one() {
    let (dir, c) = open();
    let root = dir.path();
    // A broken file under the selected folder, and a broken ancestor folder.yml of a selected request.
    let plan = c.plan_selected(&[ip("f"), ip("g/z.yml")]);
    assert_eq!(names(&plan), ["y", "x", "z"]);
    assert_eq!(errors(&plan, root), ["f/bad.yml", "g/folder.yml"]);
    // h/bad.yml sits under an unselected sibling folder: dropped.
    let only_q = c.plan_selected(&[ip("h/q.yml")]);
    assert!(
        errors(&only_q, root).is_empty(),
        "{:?}",
        only_q.load_errors()
    );
    // A stale member under a broken ancestor must not keep that ancestor's error.
    let stale = c.plan_selected(&[ip("g/gone.yml"), ip("w.yml")]);
    assert!(errors(&stale, root).is_empty(), "{:?}", stale.load_errors());
}
