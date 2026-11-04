use crate::support;

use apinata_app::command::{Command, Edit, PlanError, Target};
use apinata_app::model::CollectionModel;
use apinata_core::{Collection, SecretStore, VarScope, VarValue, Variable, VariableSet};
use gpui_kit::{Entity, TestAppContext};
use std::sync::Arc;
use std::time::Duration;
use support::*;

const DEV: &str =
    "name: dev\nvariables:\n  - name: host\n    value: x\n  - secret: true\n    name: key\n";

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://{{host}}/0")),
        ("environments/dev.yml", DEV),
    ])
}

fn variables(
    cx: &mut TestAppContext,
    model: &Entity<CollectionModel>,
    rel: &str,
) -> Result<VariableSet, PlanError> {
    let (reply, answer) = async_channel::bounded(1);
    send(
        cx,
        model,
        Command::Variables {
            item: ip(rel),
            reply,
        },
    );
    cx.run_until_parked();
    answer.try_recv().expect("the actor replied")
}

#[gpui_kit::test]
fn variables_follow_the_environment_and_the_draft(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    let set = variables(cx, &model, "a.yml").expect("built");
    assert!(set.get("host").is_none(), "no environment selected");
    send(cx, &model, Command::SelectEnvironment(Some("dev".into())));
    let host = variables(cx, &model, "a.yml").unwrap().get("host").unwrap();
    assert_eq!(
        (host.scope(), host.value()),
        (VarScope::Environment, Some("x"))
    );
    let own = Variable::new("own", VarValue::Text("mine".into()));
    send(
        cx,
        &model,
        Command::Edit {
            target: Target::Request(ip("a.yml")),
            edit: Edit::Vars(vec![own]),
        },
    );
    let own = variables(cx, &model, "a.yml").unwrap().get("own").unwrap();
    assert_eq!(
        (own.scope(), own.value()),
        (VarScope::Request, Some("mine"))
    );
}

#[gpui_kit::test]
fn a_missing_request_is_not_found(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    assert!(matches!(
        variables(cx, &model, "nope.yml"),
        Err(PlanError::NotFound(_))
    ));
}

#[gpui_kit::test]
fn variables_share_the_send_context_and_hide_secrets(cx: &mut TestAppContext) {
    let store = Arc::new(CountingStore::new(Duration::ZERO));
    let dir = tree();
    let c = Collection::open(dir.path()).unwrap();
    store
        .inner
        .set(&c.secret_key("dev", "key"), "s3cret")
        .unwrap();
    let model = open_with(cx, &dir, store.clone() as Arc<dyn SecretStore>);
    send(cx, &model, Command::SelectEnvironment(Some("dev".into())));
    let set = variables(cx, &model, "a.yml").unwrap();
    let key = set.get("key").expect("a stored secret is defined");
    assert_eq!(
        (key.scope(), key.value(), key.is_secret()),
        (VarScope::Environment, None, true)
    );
    let (reply, answer) = async_channel::bounded(1);
    send(
        cx,
        &model,
        Command::Plan {
            item: ip("a.yml"),
            reply,
        },
    );
    cx.run_until_parked();
    assert!(answer.try_recv().unwrap().is_ok());
    assert_eq!(store.gets(), 1, "one keychain read serves both");
}
