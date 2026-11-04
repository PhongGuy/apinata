use crate::support;

use apinata_app::command::{Command, Edit, EnvOp, NoticeKind, Target};
use apinata_app::model::CollectionModel;
use apinata_core::{Collection, MemoryStore, SecretStore};
use gpui_kit::{Entity, TestAppContext};
use std::sync::Arc;
use support::*;

const DEV: &str =
    "name: dev\nvariables:\n  - name: host\n    value: x\n  - secret: true\n    name: token\n";

fn tree() -> tempfile::TempDir {
    write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", DEV)])
}

fn stored(dir: &tempfile::TempDir, store: &dyn SecretStore, env: &str) -> Option<String> {
    let key = Collection::open(dir.path())
        .unwrap()
        .secret_key(env, "token");
    store.get(&key).unwrap()
}

fn set_token(cx: &mut TestAppContext, model: &Entity<CollectionModel>, env: &str) {
    send(
        cx,
        model,
        Command::Env(EnvOp::SetSecretValue {
            env: env.into(),
            var: "token".into(),
            value: "v".into(),
        }),
    );
    cx.run_until_parked();
}

/// Send `env`'s current draft without its secret variables.
fn drop_secrets(cx: &mut TestAppContext, model: &Entity<CollectionModel>, env: &str) {
    let s = snap(cx, model);
    let view = s
        .content
        .environments
        .iter()
        .find(|e| e.name == env)
        .expect("an environment");
    let plain = view
        .variables
        .iter()
        .filter(|v| !v.secret)
        .cloned()
        .collect();
    send(
        cx,
        model,
        Command::Edit {
            target: Target::Environment(env.into()),
            edit: Edit::Variables(plain),
        },
    );
    cx.run_until_parked();
}

fn save(cx: &mut TestAppContext, model: &Entity<CollectionModel>, env: &str) {
    send(cx, model, Command::Save(Target::Environment(env.into())));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_saved_draft_without_a_secret_deletes_its_value_after_other_operations(
    cx: &mut TestAppContext,
) {
    let dir = tree();
    let store = Arc::new(MemoryStore::new());
    let model = open_with(cx, &dir, store.clone());
    set_token(cx, &model, "dev");
    drop_secrets(cx, &model, "dev");
    send(
        cx,
        &model,
        Command::Env(EnvOp::Create { name: "qa".into() }),
    );
    cx.run_until_parked();
    assert_eq!(
        stored(&dir, &*store, "dev").as_deref(),
        Some("v"),
        "a draft deletes nothing"
    );
    send(
        cx,
        &model,
        Command::Revert(Target::Environment("dev".into())),
    );
    cx.run_until_parked();
    assert_eq!(
        stored(&dir, &*store, "dev").as_deref(),
        Some("v"),
        "a revert loses nothing"
    );
    drop_secrets(cx, &model, "dev");
    save(cx, &model, "dev");
    assert_eq!(stored(&dir, &*store, "dev"), None);
    let n = notices(cx, &model);
    assert!(
        n.iter()
            .all(|k| !matches!(k, NoticeKind::Warnings(_) | NoticeKind::Failed { .. })),
        "{n:?}"
    );
}

#[gpui_kit::test]
fn an_environment_added_by_a_reload_is_covered(cx: &mut TestAppContext) {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    let store = Arc::new(MemoryStore::new());
    let model = open_with(cx, &dir, store.clone());
    std::fs::create_dir_all(dir.path().join("environments")).unwrap();
    std::fs::write(
        dir.path().join("environments/stage.yml"),
        DEV.replace("dev", "stage"),
    )
    .unwrap();
    send(cx, &model, Command::ReloadAll);
    cx.run_until_parked();
    set_token(cx, &model, "stage");
    drop_secrets(cx, &model, "stage");
    save(cx, &model, "stage");
    assert_eq!(stored(&dir, &*store, "stage"), None);
}

#[gpui_kit::test]
fn a_failed_save_deletes_nothing(cx: &mut TestAppContext) {
    let dir = tree();
    let store = Arc::new(MemoryStore::new());
    let model = open_with(cx, &dir, store.clone());
    set_token(cx, &model, "dev");
    drop_secrets(cx, &model, "dev");
    // A disk change the actor has not reloaded makes the save a conflict.
    std::fs::write(
        dir.path().join("environments/dev.yml"),
        format!("{DEV}color: \"#ff0000\"\n"),
    )
    .unwrap();
    save(cx, &model, "dev");
    assert_eq!(stored(&dir, &*store, "dev").as_deref(), Some("v"));
    assert!(
        std::fs::read_to_string(dir.path().join("environments/dev.yml"))
            .unwrap()
            .contains("token")
    );
}

#[gpui_kit::test]
fn a_failed_secret_delete_is_a_warning(cx: &mut TestAppContext) {
    let dir = tree();
    let mut store = CountingStore::new(std::time::Duration::ZERO);
    store.fail_deletes = true;
    let model = open_with(cx, &dir, Arc::new(store));
    set_token(cx, &model, "dev");
    drop_secrets(cx, &model, "dev");
    save(cx, &model, "dev");
    let n = notices(cx, &model);
    assert!(
        matches!(&n[..], [NoticeKind::Warnings(w)] if w.len() == 1 && w[0].contains("token")),
        "{n:?}"
    );
    assert!(
        !std::fs::read_to_string(dir.path().join("environments/dev.yml"))
            .unwrap()
            .contains("token"),
        "the save stands"
    );
}

#[gpui_kit::test]
fn a_save_that_writes_nothing_deletes_nothing(cx: &mut TestAppContext) {
    // The actor holds `token` as plain; a checkout makes it secret on disk with a stored value,
    // and Save runs on the clean draft before any reload.
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "environments/dev.yml",
            &DEV.replace("  - secret: true\n", "  - "),
        ),
    ]);
    let store = Arc::new(MemoryStore::new());
    let model = open_with(cx, &dir, store.clone());
    std::fs::write(dir.path().join("environments/dev.yml"), DEV).unwrap();
    let key = Collection::open(dir.path())
        .unwrap()
        .secret_key("dev", "token");
    store.set(&key, "v").unwrap();
    save(cx, &model, "dev");
    assert_eq!(stored(&dir, &*store, "dev").as_deref(), Some("v"));
}
