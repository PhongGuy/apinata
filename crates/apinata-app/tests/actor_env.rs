use crate::support;

use apinata_app::command::{Command, Edit, EnvOp, NoticeKind, Target};
use apinata_app::snapshot::DiskState;
use apinata_core::{Collection, SecretStore};
use gpui_kit::TestAppContext;
use std::sync::Arc;
use std::time::Duration;
use support::*;

const DEV: &str =
    "name: dev\nvariables:\n  - name: host\n    value: x\n  - secret: true\n    name: token\n";

fn tree() -> tempfile::TempDir {
    write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", DEV)])
}

fn env_op(op: EnvOp) -> Command {
    Command::Env(op)
}

#[gpui_kit::test]
fn an_environment_rename_carries_the_selection_and_older_edits(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    send(cx, &model, Command::SelectEnvironment(Some("dev".into())));
    cx.run_until_parked();
    send(
        cx,
        &model,
        env_op(EnvOp::Rename {
            from: "dev".into(),
            to: "prod".into(),
        }),
    );
    let color = Edit::Color(Some("#ff0000".into()));
    send(
        cx,
        &model,
        Command::Edit {
            target: Target::Environment("dev".into()),
            edit: color,
        },
    );
    cx.run_until_parked();
    let s = snap(cx, &model);
    assert_eq!(s.selected_environment.as_deref(), Some("prod"));
    let prod = s
        .content
        .environments
        .iter()
        .find(|e| e.name == "prod")
        .unwrap();
    assert_eq!(prod.color.as_deref(), Some("#ff0000"));
    send(
        cx,
        &model,
        env_op(EnvOp::Delete {
            name: "prod".into(),
        }),
    );
    cx.run_until_parked();
    assert_eq!(snap(cx, &model).selected_environment, None);
}

#[gpui_kit::test]
fn create_reports_the_name_used(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    send(cx, &model, env_op(EnvOp::Create { name: "dev".into() }));
    cx.run_until_parked();
    let found = notices(cx, &model);
    let [NoticeKind::EnvironmentCreated(name)] = found.as_slice() else {
        panic!("{found:?}")
    };
    assert_ne!(name, "dev", "a taken name is suffixed");
    assert!(
        snap(cx, &model)
            .content
            .environments
            .iter()
            .any(|e| &e.name == name)
    );
}

#[gpui_kit::test]
fn set_secret_moves_the_value_into_the_store(cx: &mut TestAppContext) {
    let dir = tree();
    let store = Arc::new(CountingStore::new(Duration::ZERO));
    let model = open_with(cx, &dir, store.clone() as Arc<dyn SecretStore>);
    send(
        cx,
        &model,
        env_op(EnvOp::SetSecret {
            env: "dev".into(),
            var: "host".into(),
            secret: true,
        }),
    );
    cx.run_until_parked();
    let s = snap(cx, &model);
    let host = s.content.environments[0]
        .variables
        .iter()
        .find(|v| v.name == "host")
        .unwrap()
        .clone();
    assert!(host.secret);
    let key = Collection::open(dir.path())
        .unwrap()
        .secret_key("dev", "host");
    assert_eq!(store.inner.get(&key).unwrap().as_deref(), Some("x"));
}

#[gpui_kit::test]
fn store_failures_after_the_change_are_warnings(cx: &mut TestAppContext) {
    let dir = tree();
    let store = Arc::new(CountingStore {
        fail_deletes: true,
        ..CountingStore::new(Duration::ZERO)
    });
    let key = Collection::open(dir.path())
        .unwrap()
        .secret_key("dev", "token");
    store.inner.set(&key, "s3cret").unwrap();
    let model = open_with(cx, &dir, store as Arc<dyn SecretStore>);
    send(
        cx,
        &model,
        env_op(EnvOp::SetSecret {
            env: "dev".into(),
            var: "token".into(),
            secret: false,
        }),
    );
    cx.run_until_parked();
    let found = notices(cx, &model);
    assert!(
        matches!(found.as_slice(), [NoticeKind::Warnings(w)] if w.len() == 1),
        "{found:?}"
    );
    let on_disk = std::fs::read_to_string(dir.path().join("environments/dev.yml")).unwrap();
    assert!(on_disk.contains("s3cret"));
}

#[gpui_kit::test]
fn a_recreated_environment_inherits_no_flag_or_held_edits(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    let dev = Target::Environment("dev".into());
    let color = |c: &str| Command::Edit {
        target: Target::Environment("dev".into()),
        edit: Edit::Color(Some(c.into())),
    };
    // A dirty draft whose file changes on disk is flagged, and an edit typed before the
    // reload is held.
    send(cx, &model, color("#ff0000"));
    cx.run_until_parked();
    std::fs::write(
        dir.path().join("environments/dev.yml"),
        DEV.replace("value: x", "value: y"),
    )
    .unwrap();
    send(
        cx,
        &model,
        Command::Reload(vec![dir.path().join("environments/dev.yml")]),
    );
    send(cx, &model, color("#00ff00"));
    cx.run_until_parked();
    let s = snap(cx, &model);
    assert_eq!(s.content.environments[0].disk, DiskState::Changed);
    assert!(s.stale.contains_key(&dev));
    notices(cx, &model);
    send(cx, &model, env_op(EnvOp::Delete { name: "dev".into() }));
    cx.run_until_parked();
    send(cx, &model, env_op(EnvOp::Create { name: "dev".into() }));
    cx.run_until_parked();
    assert_eq!(
        notices(cx, &model),
        [NoticeKind::EnvironmentCreated("dev".into())]
    );
    let s = snap(cx, &model);
    let env = s
        .content
        .environments
        .iter()
        .find(|e| e.name == "dev")
        .unwrap();
    assert_ne!(env.disk, DiskState::Changed);
    assert!(!s.stale.contains_key(&dev));
}

#[gpui_kit::test]
fn deleting_an_extended_environment_fails_and_keeps_the_file(cx: &mut TestAppContext) {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("environments/qa.yml", "name: qa\nextends: dev\n"),
    ]);
    let model = open(cx, &dir);
    send(cx, &model, env_op(EnvOp::Delete { name: "dev".into() }));
    cx.run_until_parked();
    let found = notices(cx, &model);
    assert!(
        matches!(found.as_slice(), [NoticeKind::Failed { message, .. }] if message.contains("`qa`")),
        "{found:?}"
    );
    assert!(dir.path().join("environments/dev.yml").is_file());
}
