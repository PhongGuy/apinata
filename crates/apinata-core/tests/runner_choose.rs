//! `RunPlan::choose`: an explicit list of requests runs exactly those, in the plan's order.

use crate::common;

use apinata_core::{
    CancelHandle, Collection, ContextOptions, ExecOptions, Executor, ItemPath, RunOptions, RunPlan,
    SelectError, VarContext, run,
};
use common::write_tree;
use std::sync::Arc;
use wiremock::matchers::{header, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

fn ip(rel: &str) -> ItemPath {
    ItemPath::new(rel).expect("a valid item path")
}

fn req(name: &str, seq: u32, url: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"{url}\"\n"
    )
}

fn names(plan: &RunPlan) -> Vec<String> {
    plan.requests().map(|r| r.name().to_owned()).collect()
}

fn tree(base: &str) -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "f/folder.yml",
            "info:\n  name: f\n  type: folder\n  seq: 1\nrequest:\n  headers:\n    - name: X-Folder\n      value: \"from-f\"\n",
        ),
        ("f/x.yml", &req("x", 1, &format!("{base}/x"))),
        ("f/y.yml", &req("y", 2, &format!("{base}/y"))),
        ("a.yml", &req("a", 1, &format!("{base}/a"))),
        ("b.yml", &req("b", 2, &format!("{base}/b"))),
    ])
}

#[test]
fn chosen_requests_run_in_plan_order_whatever_the_order_given() {
    let dir = tree("http://127.0.0.1:1");
    let c = Collection::open(dir.path()).expect("opens");
    let plan = c.plan(dir.path()).expect("plan");
    assert_eq!(names(&plan), ["x", "y", "a", "b"]);
    let chosen = plan
        .choose(&[ip("b.yml"), ip("f/x.yml"), ip("b.yml")])
        .expect("every id is planned");
    assert_eq!(names(&chosen), ["x", "b"]);
}

#[test]
fn an_empty_choice_leaves_an_empty_plan() {
    let dir = tree("http://127.0.0.1:1");
    let c = Collection::open(dir.path()).expect("opens");
    let plan = c.plan(dir.path()).expect("plan").choose(&[]).expect("ok");
    assert_eq!(plan.items().count(), 0);
}

#[test]
fn an_id_the_plan_does_not_hold_is_an_error() {
    let dir = tree("http://127.0.0.1:1");
    let c = Collection::open(dir.path()).expect("opens");
    // A folder is not a request, and a request outside the planned folder is not planned.
    for bad in ["gone.yml", "f", "a.yml"] {
        let plan = c.plan(&dir.path().join("f")).expect("plan");
        match plan.choose(&[ip("f/x.yml"), ip(bad)]) {
            Err(e @ SelectError::NotPlanned { .. }) => {
                assert_eq!(e.to_string(), format!("{bad} is not a request in this run"));
            }
            other => panic!("{bad}: expected NotPlanned, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn a_chosen_request_keeps_its_folder_headers() {
    let server = MockServer::start().await;
    Mock::given(path("/y"))
        .and(header("X-Folder", "from-f"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/a"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let dir = tree(&server.uri());
    let c = Collection::open(dir.path()).expect("opens");
    let plan = (c.plan(dir.path()).expect("plan"))
        .choose(&[ip("a.yml"), ip("f/y.yml")])
        .expect("planned");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let exec = Arc::new(Executor::new(ExecOptions::new()).expect("client"));
    let report = run(
        plan,
        ctx,
        exec,
        RunOptions::new(),
        CancelHandle::new(),
        Box::new(|_| {}),
    )
    .await;
    let ran: Vec<&str> = report.results.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(ran, ["y", "a"]);
    // The mocks' `.expect` counts fail on drop if x or b was sent, or y lost its folder header.
    server.verify().await;
}
