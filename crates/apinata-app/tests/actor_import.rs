//! The actor applies a parsed request to a draft and writes imported environments.

use crate::support;

use apinata_app::command::{Command, Field, NoticeKind, Target};
use apinata_app::model::CollectionModel;
use apinata_core::import::{
    ImportReport, ImportedRequest, NoteKind, RequestFormat, parse_environment, parse_request,
};
use apinata_core::{Collection, KeyValue, MemoryStore, SecretError, SecretKey, SecretStore};
use gpui_kit::{Entity, TestAppContext};
use std::fs;
use std::sync::Arc;
use support::*;

const ENV: &str = r#"{"name":"dev","values":[{"key":"host","value":"x.test","enabled":true},{"key":"token","value":"s3","type":"secret","enabled":true}],"_postman_variable_scope":"environment"}"#;

fn curl(text: &str) -> ImportedRequest {
    parse_request(Some(RequestFormat::Curl), text)
        .expect("parsed")
        .request
}

fn import_env(
    cx: &mut TestAppContext,
    model: &Entity<CollectionModel>,
) -> Result<ImportReport, String> {
    let (reply, answer) = async_channel::bounded(1);
    let env = parse_environment(None, ENV.as_bytes()).expect("parsed");
    send(cx, model, Command::ImportEnvironment { env, reply });
    cx.run_until_parked();
    answer.try_recv().expect("the actor replied")
}

/// A store whose writes all fail, as a locked keychain does.
struct Locked;

impl SecretStore for Locked {
    fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
        Ok(None)
    }
    fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
        Err(SecretError::Unavailable {
            message: "locked".into(),
        })
    }
    fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
        Ok(())
    }
}

#[gpui_kit::test]
fn an_applied_import_replaces_the_draft_but_not_its_name(cx: &mut TestAppContext) {
    let dir = collection();
    let model = open(cx, &dir);
    let imported = curl("curl -X PUT 'https://x.test/u?q=1' -H 'X-A: 1'");
    send(
        cx,
        &model,
        Command::ApplyImport {
            item: ip("a.yml"),
            request: imported,
        },
    );
    cx.run_until_parked();
    let s = snap(cx, &model);
    let view = &s.content.requests[&ip("a.yml")];
    assert_eq!((view.name.as_str(), view.method.as_str()), ("a", "PUT"));
    assert_eq!(view.url, "https://x.test/u?q=1");
    let headers: Vec<_> = view
        .headers
        .iter()
        .map(|h| (h.name.as_str(), h.value.as_str()))
        .collect();
    assert_eq!(headers, [("X-A", "1")]);
    assert!(view.dirty, "the draft stays unsaved");
    assert_eq!(
        fs::read_to_string(dir.path().join("a.yml")).unwrap(),
        request("a", "http://x/0")
    );
    assert!(notices(cx, &model).is_empty());
}

#[gpui_kit::test]
fn an_applied_import_holds_its_fields_until_acknowledged(cx: &mut TestAppContext) {
    let dir = collection();
    let model = open(cx, &dir);
    send(
        cx,
        &model,
        Command::ApplyImport {
            item: ip("a.yml"),
            request: curl("curl https://x.test/b"),
        },
    );
    let target = Target::Request(ip("a.yml"));
    let fields = [
        Field::Method,
        Field::UrlParams,
        Field::Headers,
        Field::Body,
        Field::Auth,
        Field::Docs,
        Field::Settings,
    ];
    assert!(model.read_with(cx, |m, _| fields.iter().all(|f| m.in_flight(&target, *f))));
    cx.run_until_parked();
    assert!(model.read_with(cx, |m, _| fields.iter().all(|f| !m.in_flight(&target, *f))));
}

#[gpui_kit::test]
fn an_import_into_a_missing_request_is_not_found(cx: &mut TestAppContext) {
    let dir = collection();
    let model = open(cx, &dir);
    send(
        cx,
        &model,
        Command::ApplyImport {
            item: ip("gone.yml"),
            request: curl("curl https://x.test/b"),
        },
    );
    cx.run_until_parked();
    assert_eq!(
        notices(cx, &model),
        [NoticeKind::NotFound(Target::Request(ip("gone.yml")))]
    );
}

#[gpui_kit::test]
fn an_imported_environment_is_written_and_its_secret_stored(cx: &mut TestAppContext) {
    let dir = collection();
    let store = Arc::new(MemoryStore::new());
    let model = open_with(cx, &dir, store.clone());
    let report = import_env(cx, &model).expect("imported");
    assert_eq!(report.environments, ["dev"]);
    assert!(report.secrets_to_enter.is_empty());
    let s = snap(cx, &model);
    assert_eq!(
        s.content
            .environments
            .iter()
            .map(|e| e.name.as_str())
            .collect::<Vec<_>>(),
        ["dev"]
    );
    let text = fs::read_to_string(dir.path().join("environments/dev.yml")).unwrap();
    assert!(!text.contains("s3"), "secret values never reach the file");
    let key = Collection::open(dir.path())
        .unwrap()
        .secret_key("dev", "token");
    assert_eq!(store.get(&key).unwrap().as_deref(), Some("s3"));
    assert!(notices(cx, &model).contains(&NoticeKind::EnvironmentCreated("dev".into())));
}

#[gpui_kit::test]
fn a_refused_secret_keeps_the_environment_and_asks_for_the_value(cx: &mut TestAppContext) {
    let dir = collection();
    let model = open_with(cx, &dir, Arc::new(Locked));
    let report = import_env(cx, &model).expect("imported");
    assert_eq!(
        report.secrets_to_enter,
        [("dev".to_owned(), "token".to_owned())]
    );
    assert!(report.notes.iter().any(|n| n.kind == NoteKind::NeedsInput));
    assert!(dir.path().join("environments/dev.yml").is_file());
}

#[gpui_kit::test]
fn an_import_older_than_a_reload_fails_and_is_not_held(cx: &mut TestAppContext) {
    let dir = collection();
    let model = open(cx, &dir);
    fs::write(dir.path().join("a.yml"), request("a", "http://x/disk")).unwrap();
    // Both envelopes carry the pre-reload `seen`, as in actor_reload.rs.
    send(cx, &model, Command::Reload(vec![dir.path().join("a.yml")]));
    send(
        cx,
        &model,
        Command::ApplyImport {
            item: ip("a.yml"),
            request: curl("curl https://x.test/b"),
        },
    );
    cx.run_until_parked();
    let a = Target::Request(ip("a.yml"));
    let s = snap(cx, &model);
    assert_eq!(s.content.requests[&ip("a.yml")].url, "http://x/disk");
    assert!(!s.stale.contains_key(&a), "an import is not held");
    let message = "the request was reloaded from disk; import it again".into();
    assert_eq!(
        notices(cx, &model),
        [NoticeKind::Failed {
            target: Some(a),
            message,
            row: None
        }]
    );
}

#[gpui_kit::test]
fn an_import_core_cannot_write_fails_and_leaves_the_draft(cx: &mut TestAppContext) {
    let dir = collection();
    let model = open(cx, &dir);
    let mut imported = curl("curl -X PUT https://x.test/b");
    imported.headers.push(KeyValue::new("X-A\nB", "1"));
    send(
        cx,
        &model,
        Command::ApplyImport {
            item: ip("a.yml"),
            request: imported,
        },
    );
    cx.run_until_parked();
    let view = &snap(cx, &model).content.requests[&ip("a.yml")];
    assert_eq!(
        (view.method.as_str(), view.url.as_str(), view.dirty),
        ("GET", "http://x/0", false)
    );
    let notices = notices(cx, &model);
    assert!(
        matches!(notices.as_slice(), [NoticeKind::Failed { target: Some(Target::Request(p)), message, .. }]
            if *p == ip("a.yml") && message.contains("headers[0].name")),
        "{notices:?}"
    );
}

#[gpui_kit::test]
fn a_failed_environment_import_is_noticed_and_replied(cx: &mut TestAppContext) {
    let dir = collection();
    let model = open(cx, &dir);
    let mut env = parse_environment(None, ENV.as_bytes()).expect("parsed");
    env.name = "...".into(); // A7: sanitises to empty, so core refuses it
    let (reply, answer) = async_channel::bounded(1);
    send(cx, &model, Command::ImportEnvironment { env, reply });
    cx.run_until_parked();
    let message = answer.try_recv().expect("the actor replied").unwrap_err();
    assert!(
        message.contains("not a valid environment name"),
        "{message}"
    );
    assert_eq!(
        notices(cx, &model),
        [NoticeKind::Failed {
            target: None,
            message,
            row: None
        }]
    );
    assert!(!dir.path().join("environments").exists());
}

#[gpui_kit::test]
fn an_imported_aws_secret_is_stored_in_the_only_environment(cx: &mut TestAppContext) {
    let dir = support::write_tree(&[
        ("opencollection.yml", support::ROOT),
        ("a.yml", &support::request("a", "http://x/0")),
        ("environments/dev.yml", "name: dev\n"),
    ]);
    let store = Arc::new(MemoryStore::new());
    let model = open_with(cx, &dir, store.clone());
    let (reply, answer) = async_channel::bounded(1);
    let aws = curl("curl --aws-sigv4 aws:amz:r:s --user AKID:TOPSECRET https://x.test/b");
    send(
        cx,
        &model,
        Command::ApplyImport {
            item: ip("a.yml"),
            request: aws.clone(),
        },
    );
    send(
        cx,
        &model,
        Command::StoreImportSecrets {
            item: ip("a.yml"),
            request: aws,
            notes: Vec::new(),
            reply,
        },
    );
    cx.run_until_parked();
    let root = fs::canonicalize(dir.path()).unwrap();
    let key = SecretKey::new(root, "dev", "awsSecretAccessKey");
    assert_eq!(store.get(&key).unwrap().as_deref(), Some("TOPSECRET"));
    let env = fs::read_to_string(dir.path().join("environments/dev.yml")).unwrap();
    assert!(
        env.contains("awsSecretAccessKey") && !env.contains("TOPSECRET"),
        "{env}"
    );
    assert!(
        answer.try_recv().is_ok(),
        "the actor answered with the notes"
    );
}

#[gpui_kit::test]
fn a_secret_is_not_stored_when_its_import_did_not_apply(cx: &mut TestAppContext) {
    let dir = support::write_tree(&[
        ("opencollection.yml", support::ROOT),
        ("a.yml", &support::request("a", "http://x/0")),
        ("environments/dev.yml", "name: dev\n"),
    ]);
    let store = Arc::new(MemoryStore::new());
    let model = open_with(cx, &dir, store.clone());
    let aws = curl("curl --aws-sigv4 aws:amz:r:s --user AKID:TOPSECRET https://x.test/b");
    let (reply, answer) = async_channel::bounded(1);
    for command in [
        Command::ApplyImport {
            item: ip("gone.yml"),
            request: aws.clone(),
        },
        Command::StoreImportSecrets {
            item: ip("gone.yml"),
            request: aws,
            notes: Vec::new(),
            reply,
        },
    ] {
        send(cx, &model, command);
    }
    cx.run_until_parked();
    let root = fs::canonicalize(dir.path()).unwrap();
    let key = SecretKey::new(root, "dev", "awsSecretAccessKey");
    assert_eq!(store.get(&key).unwrap(), None);
    let env = fs::read_to_string(dir.path().join("environments/dev.yml")).unwrap();
    assert!(!env.contains("awsSecretAccessKey"), "{env}");
    assert!(answer.try_recv().is_ok(), "the banner still gets an answer");
}
