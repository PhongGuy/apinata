//! OAuth 2.0 auth edits: a typed client secret or password goes to `.env`, never into a file.

use crate::support;

use apinata_app::command::{Command, Edit, NoticeKind, Planned, Target};
use apinata_app::model::CollectionModel;
use apinata_core::{AuthSpec, OAuth2Flow, OAuth2Spec};
use gpui_kit::{Entity, TestAppContext};
use std::path::Path;
use std::process::Command as Process;
use support::*;
use tempfile::TempDir;

const TRACKED: &str = ".env is tracked by git, so nothing was saved; untrack .env first";

// A request with a client credentials auth; `credentials` lines go under `credentials:`.
fn oauth_request(flow: &str, credentials: &str, extra: &str) -> String {
    format!(
        "info:\n  name: r\n  type: http\n  seq: 1\n\nhttp:\n  method: GET\n  url: http://x\n  auth:\n    type: oauth2\n    flow: {flow}\n    accessTokenUrl: http://id/token\n    credentials:\n      clientId: svc\n{credentials}{extra}"
    )
}

fn cc_request(credentials: &str) -> String {
    oauth_request("client_credentials", credentials, "")
}

fn request_auth(cx: &mut TestAppContext, model: &Entity<CollectionModel>) -> AuthSpec {
    snap(cx, model).content.requests[&ip("r.yml")].auth.clone()
}

fn oauth(auth: AuthSpec) -> OAuth2Spec {
    match auth {
        AuthSpec::OAuth2(o) => o,
        other => panic!("not oauth2: {other:?}"),
    }
}

fn edit(cx: &mut TestAppContext, model: &Entity<CollectionModel>, target: Target, auth: AuthSpec) {
    send(
        cx,
        model,
        Command::Edit {
            target,
            edit: Edit::Auth(auth),
        },
    );
    cx.run_until_parked();
}

fn save(cx: &mut TestAppContext, model: &Entity<CollectionModel>, target: Target) {
    send(cx, model, Command::Save(target));
    cx.run_until_parked();
}

fn with_secret(secret: &str) -> AuthSpec {
    let mut o = OAuth2Spec::new(OAuth2Flow::ClientCredentials);
    o.access_token_url = "http://id/token".into();
    o.client_id = "svc".into();
    o.client_secret = secret.into();
    AuthSpec::OAuth2(o)
}

fn read(dir: &TempDir, rel: &str) -> String {
    std::fs::read_to_string(dir.path().join(rel)).unwrap_or_default()
}

fn git(dir: &Path, args: &[&str]) {
    let out = Process::new("git")
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}");
}

#[gpui_kit::test]
fn a_typed_client_secret_moves_to_dotenv(cx: &mut TestAppContext) {
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &cc_request(""))]);
    let model = open(cx, &dir);
    let mut o = oauth(request_auth(cx, &model));
    o.client_secret = "CL1ENTs3cret".into();
    edit(
        cx,
        &model,
        Target::Request(ip("r.yml")),
        AuthSpec::OAuth2(o),
    );
    assert_eq!(
        oauth(request_auth(cx, &model)).client_secret,
        "{{process.env.APINATA_OAUTH2_CLIENT_SECRET}}"
    );
    // Nothing reaches the disk before the save.
    assert!(!dir.path().join(".env").exists() && !dir.path().join(".gitignore").exists());
    assert!(notices(cx, &model).is_empty());
    save(cx, &model, Target::Request(ip("r.yml")));
    assert_eq!(
        read(&dir, ".env"),
        "APINATA_OAUTH2_CLIENT_SECRET='CL1ENTs3cret'\n"
    );
    assert_eq!(read(&dir, ".gitignore"), ".env\n");
    assert_eq!(
        notices(cx, &model),
        [NoticeKind::Warnings(vec![
            "Added .env to .gitignore".into()
        ])]
    );
    let saved = read(&dir, "r.yml");
    assert!(
        saved.contains("clientSecret: \"{{process.env.APINATA_OAUTH2_CLIENT_SECRET}}\"")
            && !saved.contains("CL1ENTs3cret"),
        "{saved}"
    );
}

#[gpui_kit::test]
fn a_password_grant_password_moves_too(cx: &mut TestAppContext) {
    let file = oauth_request(
        "resource_owner_password_credentials",
        "",
        "    resourceOwner:\n      username: ann\n",
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &file)]);
    let model = open(cx, &dir);
    let mut o = oauth(request_auth(cx, &model));
    assert_eq!(o.flow, OAuth2Flow::Password);
    o.password = "pw-1234".into();
    edit(
        cx,
        &model,
        Target::Request(ip("r.yml")),
        AuthSpec::OAuth2(o),
    );
    assert_eq!(
        oauth(request_auth(cx, &model)).password,
        "{{process.env.APINATA_OAUTH2_PASSWORD}}"
    );
    save(cx, &model, Target::Request(ip("r.yml")));
    assert!(read(&dir, ".env").contains("APINATA_OAUTH2_PASSWORD='pw-1234'"));
}

#[gpui_kit::test]
fn folder_and_collection_auth_move_too(cx: &mut TestAppContext) {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("Users")),
    ]);
    let model = open(cx, &dir);
    edit(
        cx,
        &model,
        Target::Folder(ip("users")),
        with_secret("s1-folder"),
    );
    edit(cx, &model, Target::Collection, with_secret("s2-collection"));
    save(cx, &model, Target::Folder(ip("users")));
    save(cx, &model, Target::Collection);
    let folder = read(&dir, "users/folder.yml");
    assert!(
        folder.contains("{{process.env.APINATA_OAUTH2_CLIENT_SECRET}}")
            && !folder.contains("s1-folder"),
        "{folder}"
    );
    let root = read(&dir, "opencollection.yml");
    assert!(
        root.contains("{{process.env.APINATA_OAUTH2_CLIENT_SECRET_2}}")
            && !root.contains("s2-collection"),
        "{root}"
    );
}

#[gpui_kit::test]
fn an_existing_literal_stays(cx: &mut TestAppContext) {
    let file = cc_request("      clientSecret: old-plain\n");
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &file)]);
    let model = open(cx, &dir);
    let mut o = oauth(request_auth(cx, &model));
    assert_eq!(o.client_secret, "old-plain");
    o.client_id = "svc2".into();
    edit(
        cx,
        &model,
        Target::Request(ip("r.yml")),
        AuthSpec::OAuth2(o),
    );
    save(cx, &model, Target::Request(ip("r.yml")));
    assert!(read(&dir, "r.yml").contains("old-plain"));
    assert!(!dir.path().join(".env").exists());
    assert!(notices(cx, &model).is_empty());
}

#[gpui_kit::test]
fn a_reference_keeps_its_name(cx: &mut TestAppContext) {
    let file = cc_request("      clientSecret: \"{{process.env.MY_CS}}\"\n");
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &file),
        (".gitignore", ".env\n"),
    ]);
    let model = open(cx, &dir);
    let mut o = oauth(request_auth(cx, &model));
    o.client_secret = "new-secret".into();
    edit(
        cx,
        &model,
        Target::Request(ip("r.yml")),
        AuthSpec::OAuth2(o),
    );
    assert_eq!(
        oauth(request_auth(cx, &model)).client_secret,
        "{{process.env.MY_CS}}"
    );
    save(cx, &model, Target::Request(ip("r.yml")));
    assert_eq!(read(&dir, ".env"), "MY_CS='new-secret'\n");
}

#[gpui_kit::test]
fn a_template_is_written_as_is(cx: &mut TestAppContext) {
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &cc_request(""))]);
    let model = open(cx, &dir);
    let mut o = oauth(request_auth(cx, &model));
    o.client_secret = "{{cs}}".into();
    edit(
        cx,
        &model,
        Target::Request(ip("r.yml")),
        AuthSpec::OAuth2(o),
    );
    assert_eq!(oauth(request_auth(cx, &model)).client_secret, "{{cs}}");
    assert!(!dir.path().join(".env").exists());
    assert!(notices(cx, &model).is_empty());
}

#[gpui_kit::test]
fn a_tracked_dotenv_refuses_and_writes_nothing(cx: &mut TestAppContext) {
    let file = cc_request("");
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &file),
        (".env", "X=1\n"),
    ]);
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["add", ".env"]);
    let model = open(cx, &dir);
    let before = request_auth(cx, &model);
    let mut o = oauth(before.clone());
    o.client_secret = "CL1ENTs3cret".into();
    edit(
        cx,
        &model,
        Target::Request(ip("r.yml")),
        AuthSpec::OAuth2(o),
    );
    assert_eq!(
        notices(cx, &model),
        [NoticeKind::Failed {
            target: Some(Target::Request(ip("r.yml"))),
            message: TRACKED.into(),
            row: None
        }]
    );
    assert_eq!(request_auth(cx, &model), before);
    assert_eq!(read(&dir, ".env"), "X=1\n");
    assert!(!dir.path().join(".gitignore").exists());
    assert_eq!(read(&dir, "r.yml"), file);
}

#[gpui_kit::test]
fn a_held_auth_edit_writes_no_dotenv(cx: &mut TestAppContext) {
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &cc_request(""))]);
    let model = open(cx, &dir);
    let mut o = oauth(request_auth(cx, &model));
    std::fs::write(
        dir.path().join("r.yml"),
        cc_request("").replace("name: r", "name: renamed"),
    )
    .unwrap();
    send(cx, &model, Command::Reload(vec![dir.path().join("r.yml")]));
    o.client_secret = "CL1ENTs3cret".into();
    edit(
        cx,
        &model,
        Target::Request(ip("r.yml")),
        AuthSpec::OAuth2(o),
    );
    assert_eq!(
        notices(cx, &model),
        [NoticeKind::Stale(Target::Request(ip("r.yml")))]
    );
    assert!(!dir.path().join(".env").exists() && !dir.path().join(".gitignore").exists());
}

#[gpui_kit::test]
fn other_auth_modes_are_untouched(cx: &mut TestAppContext) {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request("r", "http://x")),
    ]);
    let model = open(cx, &dir);
    let basic = AuthSpec::Basic {
        username: "u".into(),
        password: "p4ss".into(),
    };
    edit(cx, &model, Target::Request(ip("r.yml")), basic.clone());
    assert_eq!(request_auth(cx, &model), basic);
    save(cx, &model, Target::Request(ip("r.yml")));
    assert!(read(&dir, "r.yml").contains("p4ss"));
    assert!(!dir.path().join(".env").exists());
    assert!(notices(cx, &model).is_empty());
}

fn move_secrets(cx: &mut TestAppContext, model: &Entity<CollectionModel>, target: Target) {
    send(
        cx,
        model,
        Command::Edit {
            target,
            edit: Edit::MoveAuthSecrets,
        },
    );
    cx.run_until_parked();
}

#[gpui_kit::test]
fn move_to_dotenv_moves_existing_literals(cx: &mut TestAppContext) {
    let file = oauth_request(
        "resource_owner_password_credentials",
        "      clientSecret: old-plain\n",
        "    resourceOwner:\n      username: ann\n      password: old-pw\n",
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &file)]);
    let model = open(cx, &dir);
    let before = oauth(request_auth(cx, &model));
    assert_eq!(
        (before.client_secret.as_str(), before.password.as_str()),
        ("old-plain", "old-pw")
    );
    move_secrets(cx, &model, Target::Request(ip("r.yml")));
    let o = oauth(request_auth(cx, &model));
    assert_eq!(
        o.client_secret,
        "{{process.env.APINATA_OAUTH2_CLIENT_SECRET}}"
    );
    assert_eq!(o.password, "{{process.env.APINATA_OAUTH2_PASSWORD}}");
    assert!(!dir.path().join(".env").exists());
    save(cx, &model, Target::Request(ip("r.yml")));
    let env = read(&dir, ".env");
    assert!(
        env.contains("APINATA_OAUTH2_CLIENT_SECRET='old-plain'")
            && env.contains("APINATA_OAUTH2_PASSWORD='old-pw'"),
        "{env}"
    );
    let saved = read(&dir, "r.yml");
    assert!(
        !saved.contains("old-plain") && !saved.contains("old-pw"),
        "{saved}"
    );
}

#[gpui_kit::test]
fn moving_nothing_writes_nothing(cx: &mut TestAppContext) {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &cc_request("      clientSecret: \"{{secret}}\"\n")),
    ]);
    let model = open(cx, &dir);
    let before = request_auth(cx, &model);
    move_secrets(cx, &model, Target::Request(ip("r.yml")));
    assert_eq!(request_auth(cx, &model), before);
    assert!(!snap(cx, &model).content.requests[&ip("r.yml")].dirty);
    assert!(notices(cx, &model).is_empty());
    assert!(!dir.path().join(".env").exists() && !dir.path().join(".gitignore").exists());
}

#[gpui_kit::test]
fn moving_on_an_environment_is_unsupported(cx: &mut TestAppContext) {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", "name: dev\nvariables: []\n"),
    ]);
    let model = open(cx, &dir);
    move_secrets(cx, &model, Target::Environment("dev".into()));
    assert!(matches!(
        notices(cx, &model).as_slice(),
        [NoticeKind::Unsupported { .. }]
    ));
}

fn plan(cx: &mut TestAppContext, model: &Entity<CollectionModel>, rel: &str) -> Planned {
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
    answer
        .try_recv()
        .expect("the actor replied")
        .expect("planned")
}

// The `.env` value of `MY_CS` a send of request `rel` would use, read from its secrets.
fn sent_secret(cx: &mut TestAppContext, model: &Entity<CollectionModel>, rel: &str) -> String {
    let secrets = plan(cx, model, rel).context.secret_values();
    let found: Vec<&String> = secrets.iter().filter(|v| v.ends_with("-secret")).collect();
    assert_eq!(found.len(), 1, "{secrets:?}");
    found[0].clone()
}

#[gpui_kit::test]
fn dont_save_leaves_dotenv_untouched(cx: &mut TestAppContext) {
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &cc_request(""))]);
    let model = open(cx, &dir);
    edit(
        cx,
        &model,
        Target::Request(ip("r.yml")),
        with_secret("CL1ENTs3cret"),
    );
    send(cx, &model, Command::Revert(Target::Request(ip("r.yml"))));
    cx.run_until_parked();
    assert_eq!(oauth(request_auth(cx, &model)).client_secret, "");
    // A later save of the reverted request writes nothing for the discarded secret.
    save(cx, &model, Target::Request(ip("r.yml")));
    assert!(!dir.path().join(".env").exists() && !dir.path().join(".gitignore").exists());
}

#[gpui_kit::test]
fn an_unsaved_secret_is_what_a_send_uses(cx: &mut TestAppContext) {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &cc_request("      clientSecret: \"{{process.env.MY_CS}}\"\n"),
        ),
        (".env", "MY_CS=old-secret\n"),
        (".gitignore", ".env\n"),
    ]);
    let model = open(cx, &dir);
    assert_eq!(sent_secret(cx, &model, "r.yml"), "old-secret");
    let mut o = oauth(request_auth(cx, &model));
    o.client_secret = "new-secret".into();
    edit(
        cx,
        &model,
        Target::Request(ip("r.yml")),
        AuthSpec::OAuth2(o),
    );
    assert_eq!(sent_secret(cx, &model, "r.yml"), "new-secret");
    assert_eq!(read(&dir, ".env"), "MY_CS=old-secret\n");
    // Discarding the edit sends the stored value again.
    send(cx, &model, Command::Revert(Target::Request(ip("r.yml"))));
    cx.run_until_parked();
    assert_eq!(sent_secret(cx, &model, "r.yml"), "old-secret");
}

#[gpui_kit::test]
fn a_reference_that_is_no_env_key_is_never_reused(cx: &mut TestAppContext) {
    let file = cc_request(
        "      clientSecret: \"{{process.env.APINATA_OAmy-clientUTH2_CLIENT_SECRET}}\"\n",
    );
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &file),
        (".gitignore", ".env\n"),
    ]);
    let model = open(cx, &dir);
    let mut o = oauth(request_auth(cx, &model));
    o.client_secret = "topsecret".into();
    edit(
        cx,
        &model,
        Target::Request(ip("r.yml")),
        AuthSpec::OAuth2(o),
    );
    assert_eq!(
        oauth(request_auth(cx, &model)).client_secret,
        "{{process.env.APINATA_OAUTH2_CLIENT_SECRET}}"
    );
    save(cx, &model, Target::Request(ip("r.yml")));
    assert_eq!(
        read(&dir, ".env"),
        "APINATA_OAUTH2_CLIENT_SECRET='topsecret'\n"
    );
}

#[gpui_kit::test]
fn a_tracked_dotenv_at_save_refuses_the_save(cx: &mut TestAppContext) {
    let file = cc_request("");
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &file),
        (".env", "X=1\n"),
    ]);
    git(dir.path(), &["init", "-q"]);
    let model = open(cx, &dir);
    edit(
        cx,
        &model,
        Target::Request(ip("r.yml")),
        with_secret("CL1ENTs3cret"),
    );
    git(dir.path(), &["add", ".env"]);
    save(cx, &model, Target::Request(ip("r.yml")));
    assert_eq!(
        notices(cx, &model),
        [NoticeKind::Failed {
            target: Some(Target::Request(ip("r.yml"))),
            message: TRACKED.into(),
            row: None
        }]
    );
    assert_eq!(read(&dir, ".env"), "X=1\n");
    assert_eq!(read(&dir, "r.yml"), file);
    // The refusal keeps the held secret, so a save after untracking writes it.
    git(dir.path(), &["rm", "-q", "--cached", ".env"]);
    save(cx, &model, Target::Request(ip("r.yml")));
    assert_eq!(
        read(&dir, ".env"),
        "X=1\nAPINATA_OAUTH2_CLIENT_SECRET='CL1ENTs3cret'\n"
    );
    assert!(read(&dir, "r.yml").contains("{{process.env.APINATA_OAUTH2_CLIENT_SECRET}}"));
}

// Edits request `rel`'s client secret to `secret`.
fn type_secret(cx: &mut TestAppContext, model: &Entity<CollectionModel>, rel: &str, secret: &str) {
    let mut o = oauth(snap(cx, model).content.requests[&ip(rel)].auth.clone());
    o.client_secret = secret.into();
    edit(cx, model, Target::Request(ip(rel)), AuthSpec::OAuth2(o));
}

#[gpui_kit::test]
fn drafts_sharing_a_reference_keep_their_own_values(cx: &mut TestAppContext) {
    let shared = cc_request("      clientSecret: \"{{process.env.MY_CS}}\"\n");
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &shared),
        ("b.yml", &shared),
        ("c.yml", &shared),
        (".env", "MY_CS=old-secret\n"),
        (".gitignore", ".env\n"),
    ]);
    let model = open(cx, &dir);
    type_secret(cx, &model, "a.yml", "a-secret");
    type_secret(cx, &model, "b.yml", "b-secret");
    // Each draft's send sees its own value; an untouched request sees the file's.
    assert_eq!(sent_secret(cx, &model, "a.yml"), "a-secret");
    assert_eq!(sent_secret(cx, &model, "b.yml"), "b-secret");
    assert_eq!(sent_secret(cx, &model, "c.yml"), "old-secret");
    save(cx, &model, Target::Request(ip("a.yml")));
    assert_eq!(read(&dir, ".env"), "MY_CS='a-secret'\n");
    assert_eq!(sent_secret(cx, &model, "b.yml"), "b-secret");
    save(cx, &model, Target::Request(ip("b.yml")));
    assert_eq!(read(&dir, ".env"), "MY_CS='b-secret'\n");
}

#[gpui_kit::test]
fn a_folder_run_never_takes_a_request_drafts_secret(cx: &mut TestAppContext) {
    let shared = cc_request("      clientSecret: \"{{process.env.MY_CS}}\"\n");
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("f/a.yml", &shared),
        ("f/b.yml", &shared),
        ("f/c.yml", &shared),
        (".env", "MY_CS=old-secret\n"),
        (".gitignore", ".env\n"),
    ]);
    let model = open(cx, &dir);
    type_secret(cx, &model, "f/a.yml", "a-secret");
    type_secret(cx, &model, "f/b.yml", "b-secret");
    // Each request sends its own draft's value, and the untouched one the file's.
    assert_eq!(sent_secret(cx, &model, "f/a.yml"), "a-secret");
    assert_eq!(sent_secret(cx, &model, "f/b.yml"), "b-secret");
    assert_eq!(sent_secret(cx, &model, "f/c.yml"), "old-secret");
    // The folder's own context holds no request's draft, so it sends the file's value too.
    assert_eq!(sent_secret(cx, &model, "f"), "old-secret");
    // A run plans from the saved files, so every request in it sends the file's value.
    let (reply, answer) = async_channel::bounded(1);
    send(
        cx,
        &model,
        Command::PlanSaved {
            item: ip("f"),
            reply,
        },
    );
    cx.run_until_parked();
    let run = answer
        .try_recv()
        .expect("the actor replied")
        .expect("planned");
    assert_eq!(run.plan.requests().count(), 3);
    let secrets = run.context.secret_values();
    let found: Vec<&String> = secrets.iter().filter(|v| v.ends_with("-secret")).collect();
    assert_eq!(found, ["old-secret"], "{secrets:?}");
}

#[gpui_kit::test]
fn a_conflicted_save_leaves_dotenv_as_it_was_until_overwrite(cx: &mut TestAppContext) {
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &cc_request(""))]);
    let model = open(cx, &dir);
    type_secret(cx, &model, "r.yml", "CL1ENTs3cret");
    let outside = cc_request("").replace("name: r", "name: outside");
    std::fs::write(dir.path().join("r.yml"), &outside).unwrap();
    save(cx, &model, Target::Request(ip("r.yml")));
    assert_eq!(read(&dir, "r.yml"), outside);
    assert!(!dir.path().join(".env").exists() && !dir.path().join(".gitignore").exists());
    send(cx, &model, Command::Overwrite(Target::Request(ip("r.yml"))));
    cx.run_until_parked();
    assert_eq!(
        read(&dir, ".env"),
        "APINATA_OAUTH2_CLIENT_SECRET='CL1ENTs3cret'\n"
    );
    assert!(read(&dir, "r.yml").contains("{{process.env.APINATA_OAUTH2_CLIENT_SECRET}}"));
}

#[gpui_kit::test]
fn a_conflicted_save_then_revert_leaves_nothing_behind(cx: &mut TestAppContext) {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &cc_request("")),
        (".env", "X=1\n"),
    ]);
    let model = open(cx, &dir);
    type_secret(cx, &model, "r.yml", "CL1ENTs3cret");
    let outside = cc_request("").replace("name: r", "name: outside");
    std::fs::write(dir.path().join("r.yml"), &outside).unwrap();
    save(cx, &model, Target::Request(ip("r.yml")));
    send(cx, &model, Command::Revert(Target::Request(ip("r.yml"))));
    cx.run_until_parked();
    assert_eq!(read(&dir, ".env"), "X=1\n");
    assert!(!dir.path().join(".gitignore").exists());
    // The discarded secret is gone: a later save of the request writes nothing to `.env`.
    save(cx, &model, Target::Request(ip("r.yml")));
    assert_eq!(read(&dir, ".env"), "X=1\n");
}

#[gpui_kit::test]
fn a_held_secret_moves_with_its_renamed_folder(cx: &mut TestAppContext) {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("Users")),
        ("users/u.yml", &cc_request("")),
        (".gitignore", ".env\n"),
    ]);
    let model = open(cx, &dir);
    type_secret(cx, &model, "users/u.yml", "CL1ENTs3cret");
    let rename = apinata_app::command::TreeOp::Rename {
        item: ip("users"),
        name: "people".into(),
        rename_file: true,
    };
    send(cx, &model, Command::Tree(rename));
    cx.run_until_parked();
    assert_eq!(sent_secret_of(cx, &model, "people/u.yml"), "CL1ENTs3cret");
    save(cx, &model, Target::Request(ip("people/u.yml")));
    assert_eq!(
        read(&dir, ".env"),
        "APINATA_OAUTH2_CLIENT_SECRET='CL1ENTs3cret'\n"
    );
}

#[gpui_kit::test]
fn saving_a_new_draft_writes_its_held_secret(cx: &mut TestAppContext) {
    let dir = write_tree(&[("opencollection.yml", ROOT), (".gitignore", ".env\n")]);
    let model = open(cx, &dir);
    let create = apinata_app::command::TreeOp::CreateDraft {
        parent: apinata_core::ItemPath::root(),
        kind: apinata_core::RequestKind::Http,
    };
    send(cx, &model, Command::Tree(create));
    cx.run_until_parked();
    let draft = match notices(cx, &model).as_slice() {
        [NoticeKind::Created(path)] => path.clone(),
        other => panic!("{other:?}"),
    };
    edit(
        cx,
        &model,
        Target::Request(draft.clone()),
        with_secret("CL1ENTs3cret"),
    );
    assert!(!dir.path().join(".env").exists());
    let save_draft = apinata_app::command::TreeOp::SaveDraft {
        draft,
        parent: apinata_core::ItemPath::root(),
        name: "saved".into(),
    };
    send(cx, &model, Command::Tree(save_draft));
    cx.run_until_parked();
    assert_eq!(
        read(&dir, ".env"),
        "APINATA_OAUTH2_CLIENT_SECRET='CL1ENTs3cret'\n"
    );
    assert!(read(&dir, "saved.yml").contains("{{process.env.APINATA_OAUTH2_CLIENT_SECRET}}"));
}

// The secret value a send of `rel` would use, when its context holds exactly one.
fn sent_secret_of(cx: &mut TestAppContext, model: &Entity<CollectionModel>, rel: &str) -> String {
    let secrets = plan(cx, model, rel).context.secret_values();
    assert_eq!(secrets.len(), 1, "{secrets:?}");
    secrets[0].clone()
}
