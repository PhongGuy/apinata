use crate::support;

use apinata_app::command::{Command, NoticeKind, TreeOp};
use apinata_core::{ExampleBody, ExampleBodyKind, ExampleResponse};
use gpui_kit::TestAppContext;
use support::*;

fn add(name: &str) -> Command {
    let mut response = ExampleResponse::new(200);
    response.body = Some(ExampleBody::new(ExampleBodyKind::Text, "ok"));
    Command::Tree(TreeOp::AddExample {
        item: ip("a.yml"),
        name: name.into(),
        description: String::new(),
        response,
    })
}

fn names(
    cx: &mut TestAppContext,
    model: &gpui_kit::Entity<apinata_app::model::CollectionModel>,
) -> Vec<String> {
    snap(cx, model).content.requests[&ip("a.yml")]
        .examples
        .iter()
        .map(|e| e.name.clone())
        .collect()
}

#[gpui_kit::test]
fn examples_are_added_renamed_and_deleted_through_the_actor(cx: &mut TestAppContext) {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/{{id}}")),
    ]);
    let model = open(cx, &dir);
    send(cx, &model, edit_url("a.yml", "http://x/{{other}}"));
    send(cx, &model, add("first"));
    send(cx, &model, add("second"));
    cx.run_until_parked();
    assert_eq!(names(cx, &model), ["first", "second"]);
    let s = snap(cx, &model);
    let view = &s.content.requests[&ip("a.yml")];
    assert_eq!(
        view.examples[0].request.as_ref().unwrap().url,
        "http://x/{{other}}"
    );
    assert!(view.dirty, "the unsaved URL edit stays unsaved");
    let disk = std::fs::read_to_string(dir.path().join("a.yml")).unwrap();
    assert_eq!(disk.matches("{{other}}").count(), 2, "{disk}");
    assert_eq!(disk.matches("{{id}}").count(), 1, "{disk}");
    assert_eq!(
        notices(cx, &model),
        [
            NoticeKind::ExampleAdded {
                item: ip("a.yml"),
                name: "first".into(),
            },
            NoticeKind::ExampleAdded {
                item: ip("a.yml"),
                name: "second".into(),
            },
        ]
    );

    send(
        cx,
        &model,
        Command::Tree(TreeOp::RenameExample {
            item: ip("a.yml"),
            index: 0,
            name: "renamed".into(),
        }),
    );
    send(
        cx,
        &model,
        Command::Tree(TreeOp::DeleteExample {
            item: ip("a.yml"),
            index: 1,
        }),
    );
    cx.run_until_parked();
    assert_eq!(names(cx, &model), ["renamed"]);
    assert_eq!(
        notices(cx, &model),
        [NoticeKind::ExampleDeleted {
            item: ip("a.yml"),
            index: 1
        }]
    );
    let disk = std::fs::read_to_string(dir.path().join("a.yml")).unwrap();
    assert!(
        disk.contains("- name: renamed\n") && !disk.contains("second"),
        "{disk}"
    );

    send(
        cx,
        &model,
        Command::Tree(TreeOp::DeleteExample {
            item: ip("a.yml"),
            index: 5,
        }),
    );
    cx.run_until_parked();
    let failed = notices(cx, &model);
    assert!(
        matches!(failed.as_slice(), [NoticeKind::Failed { .. }]),
        "{failed:?}"
    );
}

#[gpui_kit::test]
fn an_added_example_is_reported_by_name(cx: &mut TestAppContext) {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/1")),
    ]);
    let model = open(cx, &dir);
    send(cx, &model, add("first"));
    cx.run_until_parked();
    assert_eq!(
        notices(cx, &model),
        [NoticeKind::ExampleAdded {
            item: ip("a.yml"),
            name: "first".into(),
        }]
    );
}

// Adding to a request that is not there fails with one error notice and confirms nothing.
#[gpui_kit::test]
fn adding_an_example_to_a_missing_request_only_fails(cx: &mut TestAppContext) {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/1")),
    ]);
    let model = open(cx, &dir);
    let mut response = ExampleResponse::new(200);
    response.body = Some(ExampleBody::new(ExampleBodyKind::Text, "ok"));
    send(
        cx,
        &model,
        Command::Tree(TreeOp::AddExample {
            item: ip("gone.yml"),
            name: "first".into(),
            description: String::new(),
            response,
        }),
    );
    cx.run_until_parked();
    let got = notices(cx, &model);
    assert!(
        matches!(got.as_slice(), [NoticeKind::Failed { .. }]),
        "{got:?}"
    );
    assert!(!dir.path().join("gone.yml").exists());
}
