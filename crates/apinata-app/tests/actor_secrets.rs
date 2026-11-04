use crate::support;

use apinata_app::command::{Command, EnvOp, NoticeKind, Target};
use apinata_app::model::CollectionModel;
use apinata_core::{Collection, MemoryStore, SecretStore, VarValue};
use gpui_kit::{Entity, TestAppContext};
use std::sync::Arc;
use support::*;

const DEV: &str =
    "name: dev\nvariables:\n  - name: host\n    value: x\n  - secret: true\n    name: token\n";

fn tree() -> tempfile::TempDir {
    write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", DEV)])
}

fn set_value(env: &str, var: &str, value: &str) -> Command {
    Command::Env(EnvOp::SetSecretValue {
        env: env.into(),
        var: var.into(),
        value: value.into(),
    })
}

fn reveal(
    cx: &mut TestAppContext,
    model: &Entity<CollectionModel>,
    env: &str,
    var: &str,
) -> Result<Option<String>, String> {
    let (reply, answer) = async_channel::bounded(1);
    send(
        cx,
        model,
        Command::RevealSecret {
            env: env.into(),
            var: var.into(),
            reply,
        },
    );
    cx.run_until_parked();
    answer.try_recv().expect("the actor replied")
}

fn stored(dir: &tempfile::TempDir, store: &MemoryStore, env: &str, var: &str) -> Option<String> {
    let key = Collection::open(dir.path()).unwrap().secret_key(env, var);
    store.get(&key).unwrap()
}

#[gpui_kit::test]
fn a_secret_value_reaches_the_store_and_nothing_else(cx: &mut TestAppContext) {
    let dir = tree();
    let store = Arc::new(MemoryStore::new());
    let model = open_with(cx, &dir, store.clone());
    send(cx, &model, set_value("dev", "token", "s3cret"));
    cx.run_until_parked();
    assert_eq!(
        stored(&dir, &store, "dev", "token").as_deref(),
        Some("s3cret")
    );
    let file = std::fs::read_to_string(dir.path().join("environments/dev.yml")).unwrap();
    assert!(!file.contains("s3cret"), "{file}");
    let s = snap(cx, &model);
    let dev = s
        .content
        .environments
        .iter()
        .find(|e| e.name == "dev")
        .unwrap();
    assert!(!dev.dirty);
    for v in &dev.variables {
        let shown = match &v.value {
            VarValue::Text(t) => t.clone(),
            VarValue::Typed { data, .. } => data.clone(),
            _ => String::new(),
        };
        assert!(!shown.contains("s3cret"));
    }
    assert!(notices(cx, &model).is_empty());
}

#[gpui_kit::test]
fn a_reveal_reads_the_store_after_earlier_commands(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open_with(cx, &dir, Arc::new(MemoryStore::new()));
    assert_eq!(reveal(cx, &model, "dev", "token"), Ok(None));
    send(cx, &model, set_value("dev", "token", "v1"));
    let (reply, answer) = async_channel::bounded(1);
    send(
        cx,
        &model,
        Command::RevealSecret {
            env: "dev".into(),
            var: "token".into(),
            reply,
        },
    );
    assert_eq!(
        model.read_with(cx, |m, _| m.pending()),
        1,
        "a reveal is not pending"
    );
    cx.run_until_parked();
    assert_eq!(answer.try_recv().unwrap(), Ok(Some("v1".into())));
    assert!(
        reveal(cx, &model, "dev", "host").is_err(),
        "a plain variable has no stored value"
    );
    assert!(reveal(cx, &model, "nope", "token").is_err());
    assert!(
        notices(cx, &model).is_empty(),
        "a reveal answers on its reply only"
    );
}

#[gpui_kit::test]
fn a_secret_value_for_a_plain_or_missing_variable_is_refused(cx: &mut TestAppContext) {
    let dir = tree();
    let store = Arc::new(MemoryStore::new());
    let model = open_with(cx, &dir, store.clone());
    send(cx, &model, set_value("dev", "host", "v"));
    send(cx, &model, set_value("nope", "token", "v"));
    cx.run_until_parked();
    let n = notices(cx, &model);
    assert!(
        matches!(&n[0], NoticeKind::Failed { target: Some(Target::Environment(e)), .. } if e == "dev"),
        "{n:?}"
    );
    assert_eq!(
        n[1],
        NoticeKind::NotFound(Target::Environment("nope".into()))
    );
    assert_eq!(stored(&dir, &store, "dev", "host"), None);
}

#[gpui_kit::test]
fn env_commands_follow_an_unseen_rename(cx: &mut TestAppContext) {
    let dir = tree();
    let store = Arc::new(MemoryStore::new());
    let model = open_with(cx, &dir, store.clone());
    send(
        cx,
        &model,
        Command::Env(EnvOp::Rename {
            from: "dev".into(),
            to: "prod".into(),
        }),
    );
    // Sent before any snapshot showed the rename, so each still names `dev`.
    send(cx, &model, set_value("dev", "token", "v2"));
    send(
        cx,
        &model,
        Command::Env(EnvOp::SetSecret {
            env: "dev".into(),
            var: "host".into(),
            secret: true,
        }),
    );
    send(cx, &model, Command::SelectEnvironment(Some("dev".into())));
    cx.run_until_parked();
    assert_eq!(stored(&dir, &store, "prod", "token").as_deref(), Some("v2"));
    assert_eq!(stored(&dir, &store, "prod", "host").as_deref(), Some("x"));
    assert_eq!(
        snap(cx, &model).selected_environment.as_deref(),
        Some("prod")
    );
    assert!(notices(cx, &model).is_empty());
}
