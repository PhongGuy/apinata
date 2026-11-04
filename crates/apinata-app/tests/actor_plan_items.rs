use crate::support;

use apinata_app::command::{Command, PlanError, Planned};
use apinata_app::model::CollectionModel;
use gpui_kit::{Entity, TestAppContext};
use support::*;

fn plan_items(
    cx: &mut TestAppContext,
    model: &Entity<CollectionModel>,
    items: &[&str],
) -> Result<Planned, PlanError> {
    let (reply, answer) = async_channel::bounded(1);
    let items = items.iter().map(|p| ip(p)).collect();
    send(cx, model, Command::PlanSavedItems { items, reply });
    cx.run_until_parked();
    answer.try_recv().expect("the actor replied")
}

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/a")),
        ("b.yml", &request("b", "http://x/b")),
        ("f/folder.yml", &folder("f")),
        ("f/x.yml", &request("x", "http://x/x")),
    ])
}

fn paths(p: &Planned) -> Vec<String> {
    p.plan.items().map(|i| i.as_str().to_owned()).collect()
}

#[gpui_kit::test]
fn items_plan_in_run_order_from_the_saved_files(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    // An unsaved edit never reaches a saved plan.
    send(cx, &model, edit_url("b.yml", "http://x/unsaved"));
    let planned = plan_items(cx, &model, &["b.yml", "f"]).expect("planned");
    assert_eq!(paths(&planned), ["f/x.yml", "b.yml"]);
    let url = planned.plan.requests().last().unwrap().url().to_owned();
    assert_eq!(url, "http://x/b");
}

#[gpui_kit::test]
fn stale_items_are_skipped_not_errors(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    std::fs::remove_file(dir.path().join("a.yml")).unwrap();
    let planned = plan_items(cx, &model, &["a.yml", "b.yml"]).expect("planned");
    assert_eq!(paths(&planned), ["b.yml"]);
    let none = plan_items(cx, &model, &["a.yml", "nope.yml"]).expect("planned");
    assert!(paths(&none).is_empty());
}

#[gpui_kit::test]
fn the_command_is_not_pending_work(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    let (reply, _answer) = async_channel::bounded(1);
    send(
        cx,
        &model,
        Command::PlanSavedItems {
            items: vec![ip("a.yml")],
            reply,
        },
    );
    assert_eq!(model.read_with(cx, |m, _| m.pending()), 0);
}
