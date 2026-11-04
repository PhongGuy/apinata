use crate::support;

use apinata_app::command::{Command, Edit, PlanError, Planned, Target};
use apinata_app::model::CollectionModel;
use apinata_core::{Collection, SecretStore};
use gpui_kit::{Entity, TestAppContext};
use std::sync::Arc;
use std::time::{Duration, Instant};
use support::*;

/// `dev` holds a plain `host` and `n` secrets, `s0` to `s{n-1}`, each stored in `store`.
fn with_secrets(n: usize, store: &CountingStore) -> tempfile::TempDir {
    let vars: String = (0..n)
        .map(|i| format!("  - secret: true\n    name: s{i}\n"))
        .collect();
    let env = format!("name: dev\nvariables:\n  - name: host\n    value: x\n{vars}");
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
        ("environments/dev.yml", &env),
    ]);
    let c = Collection::open(dir.path()).unwrap();
    for i in 0..n {
        store
            .inner
            .set(&c.secret_key("dev", &format!("s{i}")), "v")
            .unwrap();
    }
    dir
}

fn plan(
    cx: &mut TestAppContext,
    model: &Entity<CollectionModel>,
    rel: &str,
) -> Result<Planned, PlanError> {
    let (reply, answer) = async_channel::bounded(1);
    send(
        cx,
        model,
        Command::Plan {
            item: ip(rel),
            reply,
        },
    );
    cx.run_until_parked();
    answer.try_recv().expect("the actor replied")
}

#[gpui_kit::test]
fn a_send_plans_the_current_draft(cx: &mut TestAppContext) {
    let store = Arc::new(CountingStore::new(Duration::ZERO));
    let dir = with_secrets(0, &store);
    let model = open_with(cx, &dir, store);
    send(cx, &model, edit_url("a.yml", "http://x/draft"));
    let planned = plan(cx, &model, "a.yml").expect("planned");
    assert_eq!(
        planned.plan.requests().next().unwrap().url(),
        "http://x/draft"
    );
}

#[gpui_kit::test]
fn the_context_is_built_lazily_and_reused(cx: &mut TestAppContext) {
    let store = Arc::new(CountingStore::new(Duration::ZERO));
    let dir = with_secrets(2, &store);
    let model = open_with(cx, &dir, store.clone() as Arc<dyn SecretStore>);
    send(cx, &model, Command::SelectEnvironment(Some("dev".into())));
    let first = plan(cx, &model, "a.yml").expect("planned");
    assert_eq!(first.context.environment(), Some("dev"));
    plan(cx, &model, "a.yml").expect("planned");
    assert_eq!(store.gets(), 2, "one build for two sends");
    send(cx, &model, edit_url("a.yml", "http://x/1"));
    send(cx, &model, Command::Save(Target::Request(ip("a.yml"))));
    plan(cx, &model, "a.yml").expect("planned");
    assert_eq!(
        store.gets(),
        2,
        "request edits and saves do not read the keychain"
    );
    let color = Edit::Color(Some("#00ff00".into()));
    send(
        cx,
        &model,
        Command::Edit {
            target: Target::Environment("dev".into()),
            edit: color,
        },
    );
    plan(cx, &model, "a.yml").expect("planned");
    assert_eq!(store.gets(), 4, "an environment edit rebuilds the context");
}

#[gpui_kit::test]
fn plan_failures_are_replies(cx: &mut TestAppContext) {
    let store = Arc::new(CountingStore::new(Duration::ZERO));
    let dir = with_secrets(0, &store);
    let model = open_with(cx, &dir, store);
    assert!(matches!(
        plan(cx, &model, "nope.yml"),
        Err(PlanError::NotFound(_))
    ));
    send(
        cx,
        &model,
        Command::SelectEnvironment(Some("missing".into())),
    );
    assert!(matches!(
        plan(cx, &model, "a.yml"),
        Err(PlanError::Context(_))
    ));
}

#[gpui_kit::test]
fn a_slow_keychain_stalls_only_the_queue_behind_it(cx: &mut TestAppContext) {
    let store = Arc::new(CountingStore::new(Duration::from_millis(50)));
    let dir = with_secrets(4, &store);
    let model = open_with(cx, &dir, store.clone() as Arc<dyn SecretStore>);
    send(cx, &model, Command::SelectEnvironment(Some("dev".into())));
    let (reply, answer) = async_channel::bounded(1);
    send(
        cx,
        &model,
        Command::Plan {
            item: ip("a.yml"),
            reply,
        },
    );
    send(cx, &model, edit_url("a.yml", "http://x/1"));
    let last = send(cx, &model, edit_url("a.yml", "http://x/2"));
    assert_eq!(
        model.read_with(cx, |m, _| m.pending()),
        3,
        "Plan is not pending"
    );
    let started = Instant::now();
    cx.run_until_parked();
    let stalled = started.elapsed();
    println!("keychain stall: {stalled:?} for 4 secrets at 50 ms each");
    assert!(answer.try_recv().unwrap().is_ok());
    assert_eq!(store.gets(), 4);
    assert!(stalled >= Duration::from_millis(200));
    let s = snap(cx, &model);
    assert_eq!((s.acked, url(&s, "a.yml").as_str()), (last, "http://x/2"));
}

// A run plans from the saved files without touching the actor's state: it is not pending
// work, and the context cached for single sends survives it.
#[gpui_kit::test]
fn a_run_plan_is_not_pending_and_keeps_the_cached_context(cx: &mut TestAppContext) {
    let store = Arc::new(CountingStore::new(Duration::ZERO));
    let dir = with_secrets(2, &store);
    let model = open_with(cx, &dir, store.clone() as Arc<dyn SecretStore>);
    send(cx, &model, Command::SelectEnvironment(Some("dev".into())));
    plan(cx, &model, "a.yml").expect("planned");
    let (reply, answer) = async_channel::bounded(1);
    let item = ip("a.yml");
    send(cx, &model, Command::PlanSaved { item, reply });
    assert_eq!(
        model.read_with(cx, |m, _| m.pending()),
        0,
        "PlanSaved is not pending"
    );
    cx.run_until_parked();
    let planned = answer
        .try_recv()
        .expect("the actor replied")
        .expect("planned");
    assert_eq!(planned.context.environment(), Some("dev"));
    let after_run = store.gets();
    plan(cx, &model, "a.yml").expect("planned");
    assert_eq!(
        store.gets(),
        after_run,
        "a run leaves the cached context in place"
    );
}
