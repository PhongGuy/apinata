use crate::support;

use apinata_app::command::{Command, NoticeKind, Target, TreeOp};
use apinata_app::snapshot::DiskState;
use apinata_core::{DropTarget, ItemPath};
use gpui_kit::TestAppContext;
use support::*;

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
        ("users/u.yml", &request("u", "http://x/0")),
        ("admin/folder.yml", &folder("admin")),
    ])
}

fn tree_op(op: TreeOp) -> Command {
    Command::Tree(op)
}

#[gpui_kit::test]
fn an_edit_sent_before_a_rename_follows_the_item(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    send(
        cx,
        &model,
        tree_op(TreeOp::Rename {
            item: ip("a.yml"),
            name: "b".into(),
            rename_file: true,
        }),
    );
    send(cx, &model, edit_url("a.yml", "http://x/mine"));
    cx.run_until_parked();
    let moved = notices(cx, &model);
    let [NoticeKind::Moved { from, to }] = moved.as_slice() else {
        panic!("{moved:?}")
    };
    assert_eq!(from, &ip("a.yml"));
    let s = snap(cx, &model);
    assert_eq!(s.content.requests[to].url, "http://x/mine");
    assert!(!s.content.requests.contains_key(&ip("a.yml")));
    // Based on the snapshot that shows the rename, the old path is simply gone.
    send(cx, &model, edit_url("a.yml", "http://x/late"));
    cx.run_until_parked();
    assert_eq!(
        notices(cx, &model),
        [NoticeKind::NotFound(Target::Request(ip("a.yml")))]
    );
}

#[gpui_kit::test]
fn edits_follow_a_moved_folder(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    send(
        cx,
        &model,
        tree_op(TreeOp::Move {
            item: ip("users"),
            to: DropTarget::Inside(ip("admin")),
        }),
    );
    send(cx, &model, edit_url("users/u.yml", "http://x/mine"));
    cx.run_until_parked();
    assert_eq!(url(&snap(cx, &model), "admin/users/u.yml"), "http://x/mine");
}

#[gpui_kit::test]
fn creates_and_clones_report_new_paths_and_errors_are_notices(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    send(
        cx,
        &model,
        tree_op(TreeOp::CreateRequest {
            parent: ItemPath::root(),
            name: "New".into(),
        }),
    );
    send(
        cx,
        &model,
        tree_op(TreeOp::CreateFolder {
            parent: ip("admin"),
            name: "inner".into(),
        }),
    );
    send(
        cx,
        &model,
        tree_op(TreeOp::Clone {
            item: ip("a.yml"),
            name: "a copy".into(),
        }),
    );
    send(
        cx,
        &model,
        tree_op(TreeOp::Rename {
            item: ip("users/u.yml"),
            name: String::new(),
            rename_file: true,
        }),
    );
    cx.run_until_parked();
    let found = notices(cx, &model);
    let [
        NoticeKind::Created(request),
        NoticeKind::Created(folder),
        NoticeKind::Created(clone),
        NoticeKind::Failed { .. },
    ] = found.as_slice()
    else {
        panic!("{found:?}")
    };
    let s = snap(cx, &model);
    for path in [request, folder, clone] {
        assert!(s.content.rows.iter().any(|r| &r.path == path), "{path}");
    }
    assert_eq!(url(&s, clone.as_str()), "http://x/0");
}

#[gpui_kit::test]
fn a_cloned_folder_shows_once_with_its_children_after_a_reload(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    send(
        cx,
        &model,
        tree_op(TreeOp::Clone {
            item: ip("users"),
            name: "users copy".into(),
        }),
    );
    cx.run_until_parked();
    let found = notices(cx, &model);
    let [NoticeKind::Created(made)] = found.as_slice() else {
        panic!("{found:?}")
    };
    assert_eq!(made, &ip("users copy"));
    // The watcher reports the new directory after the actor already loaded it.
    send(
        cx,
        &model,
        Command::Reload(vec![dir.path().join("users copy")]),
    );
    cx.run_until_parked();
    let s = snap(cx, &model);
    for path in ["users copy", "users copy/u.yml"] {
        let rows = s.content.rows.iter().filter(|r| r.path == ip(path)).count();
        assert_eq!(rows, 1, "{path}");
    }
    assert_eq!(url(&s, "users copy/u.yml"), "http://x/0");
}

#[gpui_kit::test]
fn a_command_queued_behind_a_delete_is_a_notice(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    send(cx, &model, tree_op(TreeOp::Delete { item: ip("users") }));
    send(cx, &model, edit_url("users/u.yml", "http://x/late"));
    cx.run_until_parked();
    assert_eq!(
        notices(cx, &model),
        [
            NoticeKind::Deleted(ip("users")),
            NoticeKind::NotFound(Target::Request(ip("users/u.yml")))
        ]
    );
    assert!(!dir.path().join("users").exists());
    assert!(
        !snap(cx, &model)
            .content
            .rows
            .iter()
            .any(|r| r.path == ip("users"))
    );
}

#[gpui_kit::test]
fn deleting_a_folder_kept_for_drafts_drops_it(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    send(cx, &model, edit_url("users/u.yml", "http://x/mine"));
    cx.run_until_parked();
    std::fs::remove_dir_all(dir.path().join("users")).unwrap();
    send(cx, &model, Command::Reload(vec![dir.path().join("users")]));
    cx.run_until_parked();
    assert!(
        snap(cx, &model)
            .content
            .rows
            .iter()
            .any(|r| r.path == ip("users"))
    );
    send(cx, &model, tree_op(TreeOp::Delete { item: ip("users") }));
    cx.run_until_parked();
    assert_eq!(notices(cx, &model), [NoticeKind::Deleted(ip("users"))]);
    assert!(
        !snap(cx, &model)
            .content
            .rows
            .iter()
            .any(|r| r.path == ip("users"))
    );
}

#[gpui_kit::test]
fn a_changed_on_disk_flag_follows_a_moved_folder(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    send(cx, &model, edit_url("users/u.yml", "http://x/draft"));
    cx.run_until_parked();
    std::fs::write(
        dir.path().join("users/u.yml"),
        request("u", "http://x/disk"),
    )
    .unwrap();
    send(
        cx,
        &model,
        Command::Reload(vec![dir.path().join("users/u.yml")]),
    );
    cx.run_until_parked();
    send(
        cx,
        &model,
        tree_op(TreeOp::Move {
            item: ip("users"),
            to: DropTarget::Inside(ip("admin")),
        }),
    );
    cx.run_until_parked();
    assert_eq!(notices(cx, &model).len(), 1);
    let s = snap(cx, &model);
    assert_eq!(
        s.content.requests[&ip("admin/users/u.yml")].disk,
        DiskState::Changed
    );
}

#[gpui_kit::test]
fn a_recreated_item_inherits_no_flag_or_held_edits(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    // A dirty draft whose file changes on disk is flagged, and an edit typed before the
    // reload is held.
    send(cx, &model, edit_url("users/u.yml", "http://x/draft"));
    cx.run_until_parked();
    std::fs::write(
        dir.path().join("users/u.yml"),
        request("u", "http://x/disk"),
    )
    .unwrap();
    send(
        cx,
        &model,
        Command::Reload(vec![dir.path().join("users/u.yml")]),
    );
    send(cx, &model, edit_url("users/u.yml", "http://x/held"));
    cx.run_until_parked();
    let u = Target::Request(ip("users/u.yml"));
    let s = snap(cx, &model);
    assert_eq!(
        s.content.requests[&ip("users/u.yml")].disk,
        DiskState::Changed
    );
    assert!(s.stale.contains_key(&u));
    notices(cx, &model);
    send(cx, &model, tree_op(TreeOp::Delete { item: ip("users") }));
    cx.run_until_parked();
    send(
        cx,
        &model,
        tree_op(TreeOp::CreateFolder {
            parent: ItemPath::root(),
            name: "users".into(),
        }),
    );
    cx.run_until_parked();
    send(
        cx,
        &model,
        tree_op(TreeOp::CreateRequest {
            parent: ip("users"),
            name: "u".into(),
        }),
    );
    cx.run_until_parked();
    let found = notices(cx, &model);
    assert_eq!(
        found,
        [
            NoticeKind::Deleted(ip("users")),
            NoticeKind::Created(ip("users")),
            NoticeKind::Created(ip("users/u.yml"))
        ]
    );
    let s = snap(cx, &model);
    assert_ne!(
        s.content.requests[&ip("users/u.yml")].disk,
        DiskState::Changed
    );
    assert!(s.stale.is_empty(), "{:?}", s.stale.keys());
}

/// A read-only sibling cannot be renumbered, but the moves themselves went through.
#[gpui_kit::test]
fn a_sibling_that_cannot_be_renumbered_does_not_fail_the_moves(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r1.yml", &request("r1", "http://x/0")),
        ("r2.yml", &request("r2", "http://x/0")),
        ("RO/folder.yml", &folder("RO")),
        ("Target/folder.yml", &folder("Target")),
    ]);
    let ro = dir.path().join("RO");
    let _g = crate::support::ModeGuard::set(&ro, 0o555);
    // Permissions are not enforced for root, so there is nothing to observe.
    let enforced = std::fs::write(ro.join("probe"), "").is_err();
    let model = open(cx, &dir);
    for name in ["r1.yml", "r2.yml"] {
        send(
            cx,
            &model,
            tree_op(TreeOp::Move {
                item: ip(name),
                to: DropTarget::Inside(ip("Target")),
            }),
        );
    }
    cx.run_until_parked();
    let got = notices(cx, &model);
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
    if !enforced {
        return;
    }
    let moved = got
        .iter()
        .filter(|n| matches!(n, NoticeKind::Moved { .. }))
        .count();
    assert_eq!(moved, 2, "{got:?}");
    assert!(
        !got.iter().any(|n| matches!(n, NoticeKind::Failed { .. })),
        "{got:?}"
    );
    let warnings: Vec<&NoticeKind> = got
        .iter()
        .filter(|n| matches!(n, NoticeKind::Warnings(_)))
        .collect();
    assert!(!warnings.is_empty(), "{got:?}");
    // The text names the sibling relative to the collection, not by its absolute path.
    let NoticeKind::Warnings(w) = warnings[0] else {
        unreachable!()
    };
    let root = dir.path().display().to_string();
    assert!(w[0].contains("RO/folder.yml"), "{w:?}");
    assert!(!w[0].contains(&root), "{w:?}");
    assert!(dir.path().join("Target/r1.yml").exists());
}

/// A failed move names the file relative to the collection, as the other toasts do.
#[gpui_kit::test]
fn a_failed_move_names_paths_relative_to_the_collection(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r1.yml", &request("r1", "http://x/0")),
        ("Target/folder.yml", &folder("Target")),
    ]);
    let target = dir.path().join("Target");
    let _g = crate::support::ModeGuard::set(&target, 0o555);
    let enforced = std::fs::write(target.join("probe"), "").is_err();
    let model = open(cx, &dir);
    send(
        cx,
        &model,
        tree_op(TreeOp::Move {
            item: ip("r1.yml"),
            to: DropTarget::Inside(ip("Target")),
        }),
    );
    cx.run_until_parked();
    let got = notices(cx, &model);
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
    if !enforced {
        return;
    }
    let [NoticeKind::Failed { message, .. }] = got.as_slice() else {
        panic!("{got:?}")
    };
    assert!(
        !message.contains(&dir.path().display().to_string()),
        "{message}"
    );
}

/// A read-only sibling folder cannot be renumbered, but the delete itself went through.
#[gpui_kit::test]
fn a_delete_whose_siblings_cannot_be_renumbered_still_deletes(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r1.yml", &request("r1", "http://x/0")),
        // Seq 2 after the deleted request's 1, so the delete has to rewrite it to 1.
        (
            "RO/folder.yml",
            "info:\n  name: RO\n  type: folder\n  seq: 2\n",
        ),
    ]);
    let ro = dir.path().join("RO");
    let _g = crate::support::ModeGuard::set(&ro, 0o555);
    // Permissions are not enforced for root, so there is nothing to observe.
    let enforced = std::fs::write(ro.join("probe"), "").is_err();
    let model = open(cx, &dir);
    send(cx, &model, tree_op(TreeOp::Delete { item: ip("r1.yml") }));
    cx.run_until_parked();
    let got = notices(cx, &model);
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
    if !enforced {
        return;
    }
    assert!(!dir.path().join("r1.yml").exists());
    assert!(got.contains(&NoticeKind::Deleted(ip("r1.yml"))), "{got:?}");
    assert!(
        !got.iter().any(|n| matches!(n, NoticeKind::Failed { .. })),
        "{got:?}"
    );
    let warnings: Vec<&NoticeKind> = got
        .iter()
        .filter(|n| matches!(n, NoticeKind::Warnings(_)))
        .collect();
    assert_eq!(warnings.len(), 1, "{got:?}");
    let NoticeKind::Warnings(w) = warnings[0] else {
        unreachable!()
    };
    assert!(
        w[0].starts_with("Deleted, but could not renumber the siblings: "),
        "{w:?}"
    );
    assert!(w[0].contains("RO/folder.yml"), "{w:?}");
    assert!(!w[0].contains(&dir.path().display().to_string()), "{w:?}");
}

#[test]
fn relative_to_root_strips_unix_and_windows_roots() {
    use apinata_app::actor::relative_to_root;
    use std::path::Path;
    let unix = Path::new("/home/me/coll");
    assert_eq!(
        relative_to_root("denied: /home/me/coll/RO/folder.yml", unix),
        "denied: RO/folder.yml"
    );
    let windows = Path::new(r"C:\Users\me\coll");
    assert_eq!(
        relative_to_root(r"denied: C:\Users\me\coll\RO\folder.yml", windows),
        r"denied: RO\folder.yml"
    );
    // A root written with forward slashes still matches an error text that uses either kind.
    let mixed = Path::new("C:/Users/me/coll");
    assert_eq!(
        relative_to_root(r"denied: C:\Users\me\coll\RO\folder.yml", mixed),
        r"denied: RO\folder.yml"
    );
    // A sibling directory that merely shares the prefix is left alone.
    assert_eq!(
        relative_to_root("/home/me/coll2/x.yml", unix),
        "/home/me/coll2/x.yml"
    );
}

#[cfg(unix)]
#[test]
fn relative_to_root_strips_the_real_path_under_a_symlinked_root() {
    use apinata_app::actor::relative_to_root;
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real");
    std::fs::create_dir(&real).unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let canonical = real.canonicalize().unwrap();
    let text = format!("denied: {}/RO/folder.yml", canonical.display());
    assert_eq!(relative_to_root(&text, &link), "denied: RO/folder.yml");
}

/// Through the workspace, a move whose sibling cannot be renumbered reports the warning before
/// the Moved notice, so a batch can fold it into its summary.
#[gpui_kit::test]
fn the_renumber_warning_precedes_the_moved_notice_through_the_shell(cx: &mut TestAppContext) {
    use apinata_app::workspace::WorkspaceEvent;
    use std::os::unix::fs::PermissionsExt;
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r1.yml", &request("r1", "http://x/0")),
        ("r2.yml", &request("r2", "http://x/0")),
        ("RO/folder.yml", &folder("RO")),
        ("Target/folder.yml", &folder("Target")),
    ]);
    let ro = col.path().join("RO");
    let _g = crate::support::ModeGuard::set(&ro, 0o555);
    let enforced = std::fs::write(ro.join("probe"), "").is_err();
    let root = col.path().to_path_buf();
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, apinata_app::session::Session::default());
    let ws = workspace(cx, &session);
    let seen = events(cx, &ws);
    ws.update(cx, |w, cx| w.open(root.clone(), cx));
    cx.run_until_parked();
    ws.update(cx, |w, cx| {
        w.send(
            &root,
            tree_op(TreeOp::Move {
                item: ip("r1.yml"),
                to: DropTarget::Inside(ip("Target")),
            }),
            cx,
        )
    });
    cx.run_until_parked();
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
    if !enforced {
        return;
    }
    let kinds: Vec<NoticeKind> = seen
        .borrow()
        .iter()
        .filter_map(|e| match e {
            WorkspaceEvent::Notices {
                root: r, notices, ..
            } if *r == root => Some(notices.iter().map(|n| n.kind.clone())),
            _ => None,
        })
        .flatten()
        .collect();
    let warning = kinds
        .iter()
        .position(|k| matches!(k, NoticeKind::Warnings(_)))
        .unwrap_or_else(|| panic!("no warning in {kinds:?}"));
    let moved = kinds
        .iter()
        .position(|k| matches!(k, NoticeKind::Moved { .. }))
        .unwrap_or_else(|| panic!("no Moved in {kinds:?}"));
    assert!(warning < moved, "{kinds:?}");
    let NoticeKind::Warnings(text) = &kinds[warning] else {
        unreachable!()
    };
    assert!(text[0].contains("RO/folder.yml"), "{text:?}");
    assert!(!text[0].contains(&root.display().to_string()), "{text:?}");
}
