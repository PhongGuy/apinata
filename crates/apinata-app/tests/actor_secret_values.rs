use crate::support;

use apinata_app::command::{Command, Edit, NoticeKind, Target, TreeOp};
use apinata_app::model::CollectionModel;
use apinata_core::{AuthSpec, Collection, ItemPath, MemoryStore, RequestKind, SecretStore};
use gpui_kit::{Entity, TestAppContext};
use std::sync::Arc;
use support::*;

const DEV: &str =
    "name: dev\nvariables:\n  - secret: true\n    name: token\n  - secret: true\n    name: pin\n";

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://127.0.0.1:9/{{token}}")),
        ("environments/dev.yml", DEV),
        (".env", "DOTENV_KEY=dotenv-value-77\n"),
    ])
}

fn secret_values(
    cx: &mut TestAppContext,
    model: &Entity<CollectionModel>,
) -> Result<Vec<String>, String> {
    let (reply, answer) = async_channel::bounded(1);
    send(cx, model, Command::SecretValues { reply });
    cx.run_until_parked();
    answer.try_recv().expect("the actor replied")
}

#[gpui_kit::test]
fn the_actor_answers_the_collections_secret_values(cx: &mut TestAppContext) {
    let dir = tree();
    let store = Arc::new(MemoryStore::new());
    let c = Collection::open(dir.path()).unwrap();
    store
        .set(&c.secret_key("dev", "token"), "env-token-value-1")
        .unwrap();
    store.set(&c.secret_key("dev", "pin"), "123").unwrap();
    let model = open_with(cx, &dir, store);
    send(cx, &model, Command::SelectEnvironment(Some("dev".into())));
    cx.run_until_parked();
    let values = secret_values(cx, &model).expect("built");
    assert!(
        values.contains(&"env-token-value-1".to_owned()),
        "{values:?}"
    );
    assert!(values.contains(&"dotenv-value-77".to_owned()), "{values:?}");
    assert!(!values.iter().any(|v| v == "123"), "under the floor");
    assert_eq!(
        model.read_with(cx, |m, _| m.pending()),
        0,
        "never counted as pending"
    );
}

#[gpui_kit::test]
fn a_fetch_keeps_the_cached_context(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    let before = snap(cx, &model).context_generation;
    secret_values(cx, &model).expect("built");
    secret_values(cx, &model).expect("built");
    assert_eq!(snap(cx, &model).context_generation, before);
}

#[gpui_kit::test]
fn a_context_that_cannot_be_built_answers_its_error(cx: &mut TestAppContext) {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://127.0.0.1:9/")),
    ]);
    std::fs::create_dir(dir.path().join(".env")).unwrap();
    let model = open(cx, &dir);
    assert!(secret_values(cx, &model).is_err());
}

#[gpui_kit::test]
fn a_drafts_view_carries_its_blanked_snapshot(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    let create = TreeOp::CreateDraft {
        parent: ItemPath::root(),
        kind: RequestKind::Http,
    };
    send(cx, &model, Command::Tree(create));
    cx.run_until_parked();
    let draft = notices(cx, &model)
        .into_iter()
        .find_map(|n| match n {
            NoticeKind::Created(item) => Some(item),
            _ => None,
        })
        .expect("created");
    let auth = |token: &str| Command::Edit {
        target: Target::Request(draft.clone()),
        edit: Edit::Auth(AuthSpec::Bearer {
            token: token.into(),
        }),
    };
    send(cx, &model, auth("literal-token-1"));
    cx.run_until_parked();
    let s = snap(cx, &model);
    let view = &s.content.requests[&draft];
    let json = view
        .snapshot
        .as_ref()
        .expect("a draft carries one")
        .to_json();
    assert!(!json.contains("literal-token-1"), "{json}");
    assert!(view.blanks_credentials);
    assert!(
        s.content.requests[&ip("a.yml")].snapshot.is_none(),
        "saved requests carry none"
    );
    send(cx, &model, auth("{{token}}"));
    cx.run_until_parked();
    assert!(!snap(cx, &model).content.requests[&draft].blanks_credentials);
}
