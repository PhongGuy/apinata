use crate::support;

use apinata_app::actor::Faults;
use apinata_app::command::{Command, Field, NoticeKind, Target};
use apinata_app::model::{CollectionModel, NOT_OPEN_MESSAGE, OPEN_PANIC_MESSAGE, STOPPED_MESSAGE};
use apinata_core::{MemoryStore, SecretError, SecretKey, SecretStore};
use gpui_kit::{Entity, TestAppContext};
use std::sync::Arc;
use support::*;

/// Stands in for a keychain library that panics on a read.
struct PanickingStore;

impl SecretStore for PanickingStore {
    fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
        panic!("keychain exploded")
    }
    fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
        Ok(())
    }
    fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
        Ok(())
    }
}

type Answer =
    async_channel::Receiver<Result<apinata_app::command::Planned, apinata_app::command::PlanError>>;

/// Selecting `dev` and planning reads its secret, which panics on the actor.
fn panicked(cx: &mut TestAppContext) -> (tempfile::TempDir, Entity<CollectionModel>, Answer) {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - secret: true\n    name: token\n",
        ),
    ]);
    let model = open_with(cx, &dir, Arc::new(PanickingStore));
    let (reply, answer) = async_channel::bounded(1);
    send(cx, &model, Command::SelectEnvironment(Some("dev".into())));
    send(
        cx,
        &model,
        Command::Plan {
            item: ip("a.yml"),
            reply,
        },
    );
    send(cx, &model, edit_url("a.yml", "http://x/1"));
    assert_eq!(model.read_with(cx, |m, _| m.pending()), 2);
    cx.run_until_parked();
    (dir, model, answer)
}

fn faulty(
    cx: &mut TestAppContext,
    dir: &tempfile::TempDir,
    faults: Faults,
) -> Entity<CollectionModel> {
    let root = dir.path().to_path_buf();
    let model = cx.update(|cx| {
        CollectionModel::open_with_faults(root, Arc::new(MemoryStore::new()), faults, cx)
    });
    cx.run_until_parked();
    model
}

fn stopped_notice(found: &[NoticeKind]) -> bool {
    matches!(found, [NoticeKind::Failed { target: None, message, .. }] if message == STOPPED_MESSAGE)
}

#[gpui_kit::test]
fn a_panic_in_a_command_stops_the_actor_and_clears_pending(cx: &mut TestAppContext) {
    let (_dir, model, answer) = panicked(cx);
    assert!(answer.try_recv().is_err(), "a panicked plan sends no reply");
    assert!(model.read_with(cx, |m, _| m.stopped()));
    assert_eq!(model.read_with(cx, |m, _| m.pending()), 0);
    let found = notices(cx, &model);
    assert!(stopped_notice(&found), "{found:?}");
    assert_eq!(
        url(&snap(cx, &model), "a.yml"),
        "http://x/0",
        "the edit behind the panic never applied"
    );
}

#[gpui_kit::test]
fn a_send_to_a_stopped_actor_fails_fast(cx: &mut TestAppContext) {
    let (_dir, model, _answer) = panicked(cx);
    notices(cx, &model);
    send(cx, &model, edit_url("a.yml", "http://x/2"));
    assert_eq!(model.read_with(cx, |m, _| m.pending()), 0);
    assert!(!model.read_with(cx, |m, _| {
        m.in_flight(&Target::Request(ip("a.yml")), Field::UrlParams)
    }));
    let found = notices(cx, &model);
    assert!(stopped_notice(&found), "{found:?}");
}

#[gpui_kit::test]
fn a_panic_while_opening_is_an_open_failure(cx: &mut TestAppContext) {
    let dir = two_requests();
    let model = faulty(
        cx,
        &dir,
        Faults {
            open: Some(|| panic!("open exploded")),
            ..Faults::default()
        },
    );
    assert_eq!(
        model.read_with(cx, |m, _| m.open_error().map(str::to_owned)),
        Some(OPEN_PANIC_MESSAGE.to_owned())
    );
    assert!(!model.read_with(cx, |m, _| m.stopped()));
    assert!(model.read_with(cx, |m, _| m.snapshot().is_none()));
}

#[gpui_kit::test]
fn a_panic_while_publishing_stops_the_actor(cx: &mut TestAppContext) {
    let dir = two_requests();
    // The opening snapshot is seq 1; the one after the first command panics.
    let faults = Faults {
        publish: Some(|seq| assert!(seq < 2, "publish exploded")),
        ..Faults::default()
    };
    let model = faulty(cx, &dir, faults);
    assert!(model.read_with(cx, |m, _| m.snapshot().is_some()));
    send(cx, &model, edit_url("a.yml", "http://x/1"));
    cx.run_until_parked();
    assert!(model.read_with(cx, |m, _| m.stopped()));
    assert_eq!(model.read_with(cx, |m, _| m.pending()), 0);
    assert!(stopped_notice(&notices(cx, &model)));
    assert_eq!(snap(cx, &model).seq, 1, "no snapshot after the panic");
}

#[gpui_kit::test]
fn a_send_after_a_failed_open_fails_fast(cx: &mut TestAppContext) {
    let model = cx.update(|cx| {
        CollectionModel::open(
            "/nonexistent/apinata-root".into(),
            Arc::new(MemoryStore::new()),
            cx,
        )
    });
    send(cx, &model, edit_url("a.yml", "http://x/1"));
    assert_eq!(
        model.read_with(cx, |m, _| m.pending()),
        1,
        "queued before the actor ran"
    );
    cx.run_until_parked();
    assert!(model.read_with(cx, |m, _| m.open_error().is_some() && !m.stopped()));
    assert_eq!(model.read_with(cx, |m, _| m.pending()), 0);
    send(cx, &model, edit_url("a.yml", "http://x/2"));
    assert_eq!(model.read_with(cx, |m, _| m.pending()), 0);
    let found = notices(cx, &model);
    assert!(
        matches!(found.as_slice(), [NoticeKind::Failed { target: None, message, .. }] if message == NOT_OPEN_MESSAGE),
        "{found:?}"
    );
}
