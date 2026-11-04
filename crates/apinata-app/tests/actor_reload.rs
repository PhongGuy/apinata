use crate::support;

use apinata_app::command::{Command, Edit, NoticeKind, Target};
use apinata_app::snapshot::DiskState;
use gpui_kit::TestAppContext;
use support::*;

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
        ("b.yml", &request("b", "http://x/0")),
        ("users/folder.yml", &folder("Users")),
        ("users/u.yml", &request("u", "http://x/0")),
        ("users/deep/d.yml", &request("d", "http://x/0")),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: host\n    value: x\n",
        ),
    ])
}

fn reload(dir: &tempfile::TempDir, rel: &str) -> Command {
    Command::Reload(vec![dir.path().join(rel)])
}

#[gpui_kit::test]
fn an_edit_typed_before_a_reload_is_held_not_applied(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    std::fs::write(dir.path().join("a.yml"), request("a", "http://x/disk")).unwrap();
    send(cx, &model, reload(&dir, "a.yml"));
    send(cx, &model, edit_url("a.yml", "http://x/mine"));
    cx.run_until_parked();
    let s = snap(cx, &model);
    let a = Target::Request(ip("a.yml"));
    assert_eq!(url(&s, "a.yml"), "http://x/disk");
    assert!(matches!(s.stale[&a].as_slice(), [Edit::Url(u)] if u == "http://x/mine"));
    assert_eq!(notices(cx, &model), [NoticeKind::Stale(a.clone())]);
    send(cx, &model, edit_url("a.yml", "http://x/mine"));
    cx.run_until_parked();
    assert_eq!(url(&snap(cx, &model), "a.yml"), "http://x/mine");
    send(cx, &model, Command::DiscardStale(a.clone()));
    cx.run_until_parked();
    assert!(!snap(cx, &model).stale.contains_key(&a));
}

#[gpui_kit::test]
fn the_reload_seq_is_the_first_snapshot_that_shows_it(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    send(cx, &model, edit_url("b.yml", "http://x/b"));
    cx.run_until_parked();
    assert_eq!(snap(cx, &model).seq, 2);
    std::fs::write(dir.path().join("a.yml"), request("a", "http://x/disk")).unwrap();
    send(cx, &model, reload(&dir, "a.yml"));
    send(cx, &model, edit_url("a.yml", "http://x/seen-2"));
    cx.run_until_parked();
    let s = snap(cx, &model);
    assert_eq!((s.seq, url(&s, "a.yml").as_str()), (3, "http://x/disk"));
    send(cx, &model, edit_url("a.yml", "http://x/seen-3"));
    cx.run_until_parked();
    assert_eq!(url(&snap(cx, &model), "a.yml"), "http://x/seen-3");
}

#[gpui_kit::test]
fn a_seq_only_change_does_not_hold_edits(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    let text = request("a", "http://x/0").replace("seq: 1", "seq: 7");
    std::fs::write(dir.path().join("a.yml"), text).unwrap();
    send(cx, &model, reload(&dir, "a.yml"));
    send(cx, &model, edit_url("a.yml", "http://x/mine"));
    cx.run_until_parked();
    let s = snap(cx, &model);
    assert_eq!(url(&s, "a.yml"), "http://x/mine");
    assert!(s.stale.is_empty());
}

#[gpui_kit::test]
fn a_removed_directory_holds_edits_under_it(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    send(cx, &model, edit_url("users/deep/d.yml", "http://x/draft"));
    cx.run_until_parked();
    std::fs::remove_dir_all(dir.path().join("users")).unwrap();
    send(cx, &model, reload(&dir, "users"));
    send(cx, &model, edit_url("users/deep/d.yml", "http://x/late"));
    cx.run_until_parked();
    let s = snap(cx, &model);
    assert!(
        s.stale
            .contains_key(&Target::Request(ip("users/deep/d.yml")))
    );
}

#[gpui_kit::test]
fn a_folder_file_change_holds_folder_edits_only(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    let changed = format!("{}docs: changed on disk\n", folder("Users"));
    std::fs::write(dir.path().join("users/folder.yml"), changed).unwrap();
    send(cx, &model, reload(&dir, "users/folder.yml"));
    send(
        cx,
        &model,
        Command::Edit {
            target: Target::Folder(ip("users")),
            edit: Edit::Docs("mine".into()),
        },
    );
    send(cx, &model, edit_url("users/u.yml", "http://x/mine"));
    cx.run_until_parked();
    let s = snap(cx, &model);
    assert!(s.stale.contains_key(&Target::Folder(ip("users"))));
    assert_eq!(url(&s, "users/u.yml"), "http://x/mine");
}

#[gpui_kit::test]
fn a_deleted_file_under_a_draft_shows_as_removed(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    send(cx, &model, edit_url("a.yml", "http://x/draft"));
    cx.run_until_parked();
    std::fs::remove_file(dir.path().join("a.yml")).unwrap();
    send(cx, &model, reload(&dir, "a.yml"));
    cx.run_until_parked();
    let s = snap(cx, &model);
    assert_eq!(url(&s, "a.yml"), "http://x/draft");
    let row = s
        .content
        .rows
        .iter()
        .find(|r| r.path == ip("a.yml"))
        .unwrap();
    assert_eq!(row.disk, DiskState::Removed);
}

#[gpui_kit::test]
fn a_changed_file_under_a_draft_shows_as_changed_until_overwritten(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    send(cx, &model, edit_url("a.yml", "http://x/draft"));
    cx.run_until_parked();
    std::fs::write(dir.path().join("a.yml"), request("a", "http://x/disk")).unwrap();
    send(cx, &model, reload(&dir, "a.yml"));
    cx.run_until_parked();
    let s = snap(cx, &model);
    assert_eq!(s.content.requests[&ip("a.yml")].disk, DiskState::Changed);
    send(cx, &model, Command::Overwrite(Target::Request(ip("a.yml"))));
    cx.run_until_parked();
    let s = snap(cx, &model);
    assert_eq!(s.content.requests[&ip("a.yml")].disk, DiskState::Present);
    assert!(
        std::fs::read_to_string(dir.path().join("a.yml"))
            .unwrap()
            .contains("http://x/draft")
    );
}

#[gpui_kit::test]
fn config_file_reloads_mark_their_own_targets_and_dotenv_marks_none(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    std::fs::remove_file(dir.path().join("environments/dev.yml")).unwrap();
    std::fs::write(
        dir.path().join("opencollection.yml"),
        format!("{ROOT}docs: on disk\n"),
    )
    .unwrap();
    std::fs::write(dir.path().join(".env"), "HOST=x\n").unwrap();
    send(cx, &model, reload(&dir, "environments/dev.yml"));
    send(cx, &model, reload(&dir, "opencollection.yml"));
    send(cx, &model, reload(&dir, ".env"));
    let color = Edit::Color(Some("#ff0000".into()));
    send(
        cx,
        &model,
        Command::Edit {
            target: Target::Environment("dev".into()),
            edit: color,
        },
    );
    send(
        cx,
        &model,
        Command::Edit {
            target: Target::Collection,
            edit: Edit::Docs("mine".into()),
        },
    );
    send(cx, &model, edit_url("a.yml", "http://x/mine"));
    cx.run_until_parked();
    let s = snap(cx, &model);
    assert!(s.stale.contains_key(&Target::Environment("dev".into())));
    assert!(s.stale.contains_key(&Target::Collection));
    assert_eq!(s.stale.len(), 2);
    assert_eq!(url(&s, "a.yml"), "http://x/mine");
}

const GRAPHQL: &str = "info:\n  name: a\n  type: graphql\n  seq: 1\ngraphql:\n  url: \"http://x/gql\"\n  body:\n    query: \"{ me }\"\n";

#[gpui_kit::test]
fn a_dirty_request_reloaded_as_the_other_kind_and_back_follows_the_disk(cx: &mut TestAppContext) {
    use apinata_core::RequestKind;
    let dir = tree();
    let model = open(cx, &dir);
    let a = Target::Request(ip("a.yml"));
    let kind = |cx: &mut TestAppContext| snap(cx, &model).content.requests[&ip("a.yml")].kind;

    // The file turns into a graphql request under an HTTP draft: the draft stays, as changed.
    send(cx, &model, edit_url("a.yml", "http://x/draft"));
    cx.run_until_parked();
    std::fs::write(dir.path().join("a.yml"), GRAPHQL).unwrap();
    send(cx, &model, reload(&dir, "a.yml"));
    cx.run_until_parked();
    let s = snap(cx, &model);
    let view = &s.content.requests[&ip("a.yml")];
    assert_eq!(
        (view.kind, view.dirty, view.disk),
        (RequestKind::Http, true, DiskState::Changed)
    );
    assert_eq!(view.url, "http://x/draft");

    // Reload takes the disk's kind and drops the draft.
    send(cx, &model, Command::Revert(a.clone()));
    cx.run_until_parked();
    let s = snap(cx, &model);
    let view = &s.content.requests[&ip("a.yml")];
    assert_eq!(
        (view.kind, view.dirty, view.disk),
        (RequestKind::Graphql, false, DiskState::Present)
    );
    assert_eq!(view.url, "http://x/gql");

    // A dirty graphql request whose file turns back into HTTP reloads as HTTP.
    send(cx, &model, edit_url("a.yml", "http://x/gql-draft"));
    cx.run_until_parked();
    assert_eq!(kind(cx), RequestKind::Graphql);
    std::fs::write(dir.path().join("a.yml"), request("a", "http://x/back")).unwrap();
    send(cx, &model, reload(&dir, "a.yml"));
    cx.run_until_parked();
    assert_eq!(
        snap(cx, &model).content.requests[&ip("a.yml")].disk,
        DiskState::Changed
    );
    send(cx, &model, Command::Revert(a));
    cx.run_until_parked();
    let s = snap(cx, &model);
    let view = &s.content.requests[&ip("a.yml")];
    assert_eq!(
        (view.kind, view.dirty, view.disk),
        (RequestKind::Http, false, DiskState::Present)
    );
    assert_eq!(view.url, "http://x/back");
}
