//! Listing and removing orphaned secrets through the actor and the global queue.

use crate::support;

use apinata_app::command::Command;
use apinata_app::model::CollectionModel;
use apinata_app::workspace::GlobalOp;
use apinata_core::{Collection, GlobalEnvironments, MemoryStore, SecretStore};
use gpui_kit::{Entity, TestAppContext};
use std::sync::Arc;
use support::*;

const DEV: &str = "name: dev\nvariables:\n  - name: live\n    secret: true\n";

fn tree() -> tempfile::TempDir {
    write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", DEV)])
}

fn list(
    cx: &mut TestAppContext,
    model: &Entity<CollectionModel>,
    env: &str,
) -> Result<Vec<String>, String> {
    let (reply, answer) = async_channel::bounded(1);
    send(
        cx,
        model,
        Command::OrphanedSecrets {
            env: env.into(),
            reply,
            fileless: None,
        },
    );
    cx.run_until_parked();
    answer.try_recv().expect("the actor replied")
}

fn remove(
    cx: &mut TestAppContext,
    model: &Entity<CollectionModel>,
    names: &[&str],
) -> Result<Vec<String>, String> {
    let (reply, answer) = async_channel::bounded(1);
    let names = names.iter().map(|n| n.to_string()).collect();
    send(
        cx,
        model,
        Command::RemoveOrphanedSecrets {
            env: "dev".into(),
            names,
            reply,
        },
    );
    cx.run_until_parked();
    answer.try_recv().expect("the actor replied")
}

fn seeded(
    dir: &tempfile::TempDir,
) -> (
    Arc<MemoryStore>,
    apinata_core::SecretKey,
    apinata_core::SecretKey,
) {
    let c = Collection::open(dir.path()).unwrap();
    let store = Arc::new(MemoryStore::new());
    let (live, old) = (c.secret_key("dev", "live"), c.secret_key("dev", "old"));
    store.set(&live, "v").unwrap();
    store.set(&old, "v").unwrap();
    (store, live, old)
}

#[gpui_kit::test]
fn the_actor_lists_and_removes_a_named_orphan(cx: &mut TestAppContext) {
    let dir = tree();
    let (store, live, old) = seeded(&dir);
    let model = open_with(cx, &dir, store.clone());
    assert_eq!(list(cx, &model, "dev").unwrap(), ["old"]);
    assert_eq!(remove(cx, &model, &["old"]).unwrap(), Vec::<String>::new());
    assert_eq!(store.get(&old).unwrap(), None);
    assert!(store.get(&live).unwrap().is_some());
    assert!(list(cx, &model, "dev").unwrap().is_empty());
}

#[gpui_kit::test]
fn a_declared_name_comes_back_as_an_error(cx: &mut TestAppContext) {
    let dir = tree();
    let (store, live, _old) = seeded(&dir);
    let model = open_with(cx, &dir, store.clone());
    assert!(remove(cx, &model, &["live"]).is_err());
    assert!(store.get(&live).unwrap().is_some());
}

#[gpui_kit::test]
fn opening_and_reloading_a_collection_lists_nothing_and_deletes_nothing(cx: &mut TestAppContext) {
    let dir = tree();
    let (store, _live, old) = seeded(&dir);
    let model = open_with(cx, &dir, store.clone());
    send(cx, &model, Command::ReloadAll);
    cx.run_until_parked();
    assert!(store.get(&old).unwrap().is_some());
}

#[gpui_kit::test]
async fn the_global_queue_removes_and_the_workspace_lists(cx: &mut TestAppContext) {
    let col = collection();
    let globals = write_tree(&[("g.yml", "name: g\n")]);
    let store = Arc::new(MemoryStore::new());
    let (_state, ws, _root) = open_dir_with(cx, &col, store.clone());
    let dir = globals.path().to_path_buf();
    ws.update(cx, |w, cx| w.set_global_dir(dir.clone(), cx));
    cx.run_until_parked();
    let key = GlobalEnvironments::open(&dir)
        .unwrap()
        .secret_key("g", "old");
    store.set(&key, "v").unwrap();
    let task = ws.read_with(cx, |w, cx| w.orphaned_global_secrets("g", cx));
    assert_eq!(task.await.unwrap(), ["old"]);
    let op = GlobalOp::RemoveOrphans {
        name: "g".into(),
        names: vec!["old".into()],
    };
    ws.update(cx, |w, cx| w.edit_globals(op, cx));
    cx.run_until_parked();
    assert_eq!(store.get(&key).unwrap(), None);
    assert_eq!(
        ws.read_with(cx, |w, _| w.global_error().map(str::to_owned)),
        None
    );
}

#[gpui_kit::test]
fn a_global_removal_of_a_live_name_lands_in_the_error(cx: &mut TestAppContext) {
    let col = collection();
    let globals = write_tree(&[(
        "g.yml",
        "name: g\nvariables:\n  - name: live\n    secret: true\n",
    )]);
    let store = Arc::new(MemoryStore::new());
    let (_state, ws, _root) = open_dir_with(cx, &col, store.clone());
    let dir = globals.path().to_path_buf();
    ws.update(cx, |w, cx| w.set_global_dir(dir.clone(), cx));
    cx.run_until_parked();
    let key = GlobalEnvironments::open(&dir)
        .unwrap()
        .secret_key("g", "live");
    store.set(&key, "v").unwrap();
    let op = GlobalOp::RemoveOrphans {
        name: "g".into(),
        names: vec!["live".into()],
    };
    ws.update(cx, |w, cx| w.edit_globals(op, cx));
    cx.run_until_parked();
    assert!(store.get(&key).unwrap().is_some());
    assert!(ws.read_with(cx, |w, _| w.global_error().is_some()));
}

#[gpui_kit::test]
async fn a_workspace_with_nothing_loaded_answers_an_error(cx: &mut TestAppContext) {
    let col = collection();
    let (_state, ws, _root) = open_dir_with(cx, &col, Arc::new(MemoryStore::new()));
    let task = ws.read_with(cx, |w, cx| w.orphaned_global_secrets("g", cx));
    assert!(task.await.is_err());
}
