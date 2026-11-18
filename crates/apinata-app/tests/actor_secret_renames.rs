//! The actor finds pulled secret renames, moves them in order, and finishes re-entered values.

use crate::git_support::Sandbox;
use crate::support::*;
use apinata_app::command::Command;
use apinata_core::{
    Collection, MemoryStore, PullRange, SecretError, SecretKey, SecretName, SecretRenames,
    SecretStore,
};
use gpui_kit::TestAppContext;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const DEV: &str =
    "name: dev\nvariables:\n  - name: a\n    secret: true\n  - name: b\n    secret: true\n";
const STAGING: &str =
    "name: staging\nvariables:\n  - name: a\n    secret: true\n  - name: b\n    secret: true\n";

/// A store that answers `Unavailable` to every `set` once `locked` is on.
#[derive(Default)]
struct Lockable {
    inner: MemoryStore,
    locked: AtomicBool,
}

impl SecretStore for Lockable {
    fn get(&self, key: &SecretKey) -> Result<Option<String>, SecretError> {
        self.inner.get(key)
    }
    fn set(&self, key: &SecretKey, value: &str) -> Result<(), SecretError> {
        if self.locked.load(Ordering::SeqCst) {
            return Err(SecretError::Unavailable {
                message: "locked".into(),
            });
        }
        self.inner.set(key, value)
    }
    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        self.inner.delete(key)
    }
    fn list(&self, root: &std::path::Path) -> Result<Vec<SecretKey>, SecretError> {
        self.inner.list(root)
    }
}

/// A committed collection whose second commit renames `dev` to `staging`; the range between.
fn renamed(sb: &Sandbox) -> (tempfile::TempDir, PullRange) {
    let dir = write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", DEV)]);
    sb.git(dir.path(), &["init", "-q", "-b", "main"]);
    sb.commit_all(dir.path(), "base");
    let from = sb.git(dir.path(), &["rev-parse", "HEAD"]).trim().to_owned();
    std::fs::remove_file(dir.path().join("environments/dev.yml")).unwrap();
    std::fs::write(dir.path().join("environments/staging.yml"), STAGING).unwrap();
    sb.commit_all(dir.path(), "rename");
    let to = sb.git(dir.path(), &["rev-parse", "HEAD"]).trim().to_owned();
    (dir, PullRange::new(from, to))
}

fn recv<T>(cx: &mut TestAppContext, rx: &async_channel::Receiver<T>) -> T {
    wait_for(cx, "the actor's reply", |_| !rx.is_empty());
    rx.try_recv().unwrap()
}

fn n(env: &str, var: &str) -> SecretName {
    SecretName::new(env, var)
}

#[gpui_kit::test]
fn the_actor_finds_and_moves_pulled_renames(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (dir, range) = renamed(&sb);
    let c = Collection::open(dir.path()).unwrap();
    let store = Arc::new(Lockable::default());
    store.inner.set(&c.secret_key("dev", "a"), "va").unwrap();
    store.inner.set(&c.secret_key("dev", "b"), "vb").unwrap();
    let model = open_with(cx, &dir, store.clone());
    let (reply, rx) = async_channel::bounded(1);
    send(
        cx,
        &model,
        Command::FindSecretRenames {
            range,
            copies: None,
            git: sb.options(),
            reply,
        },
    );
    let found: SecretRenames = recv(cx, &rx).unwrap();
    assert_eq!(found.renames.len(), 2);
    let moves = vec![
        (n("dev", "a"), n("staging", "a")),
        (n("dev", "b"), n("staging", "b")),
    ];
    let (reply, rx) = async_channel::bounded(1);
    send(cx, &model, Command::MoveSecretRenames { moves, reply });
    assert_eq!(recv(cx, &rx), vec![Ok(()), Ok(())]);
    assert_eq!(
        store.get(&c.secret_key("staging", "b")).unwrap().as_deref(),
        Some("vb")
    );
    assert_eq!(store.get(&c.secret_key("dev", "a")).unwrap(), None);
}

#[gpui_kit::test]
fn move_all_stops_at_an_unavailable_store(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (dir, _) = renamed(&sb);
    let c = Collection::open(dir.path()).unwrap();
    let store = Arc::new(Lockable::default());
    // A value no error text could contain by chance, as "va" sits inside "unavailable".
    store
        .inner
        .set(&c.secret_key("dev", "a"), "s3cr3t-a")
        .unwrap();
    store.inner.set(&c.secret_key("dev", "b"), "vb").unwrap();
    store.locked.store(true, Ordering::SeqCst);
    let model = open_with(cx, &dir, store.clone());
    let moves = vec![
        (n("dev", "a"), n("staging", "a")),
        (n("dev", "b"), n("staging", "b")),
    ];
    let (reply, rx) = async_channel::bounded(1);
    send(cx, &model, Command::MoveSecretRenames { moves, reply });
    let results = recv(cx, &rx);
    assert!(results[0].is_err());
    assert!(
        results[1].as_ref().unwrap_err().starts_with("not tried"),
        "{results:?}"
    );
    assert!(
        !format!("{results:?}").contains("s3cr3t-a"),
        "no value in an error"
    );
    assert_eq!(
        store.get(&c.secret_key("dev", "b")).unwrap().as_deref(),
        Some("vb")
    );
}

#[gpui_kit::test]
fn the_actor_finishes_a_reentered_value(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (dir, _) = renamed(&sb);
    let c = Collection::open(dir.path()).unwrap();
    let store = Arc::new(Lockable::default());
    store.inner.set(&c.secret_key("dev", "a"), "va").unwrap();
    store
        .inner
        .set(&c.secret_key("staging", "a"), "typed")
        .unwrap();
    let model = open_with(cx, &dir, store.clone());
    let (reply, rx) = async_channel::bounded(1);
    send(
        cx,
        &model,
        Command::FinishReentered {
            from: n("dev", "a"),
            to: n("staging", "a"),
            reply,
        },
    );
    assert_eq!(recv(cx, &rx), Ok(true));
    assert_eq!(store.get(&c.secret_key("dev", "a")).unwrap(), None);
}
