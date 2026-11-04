//! Trashing a tree item and undoing it, through the actor with a fake trash.

use crate::support;

use apinata_app::actor::Faults;
use apinata_app::command::{Command, NoticeKind, Target, TreeOp};
use apinata_app::model::CollectionModel;
use apinata_core::MemoryStore;
use gpui_kit::{Entity, TestAppContext};
use std::sync::Arc;
use support::fake_trash::FakeTrash;
use support::*;

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
        ("users/u.yml", &request("u", "http://x/0")),
    ])
}

fn open_with_trash(
    cx: &mut TestAppContext,
    dir: &tempfile::TempDir,
    bin: Arc<FakeTrash>,
) -> Entity<CollectionModel> {
    let root = dir.path().to_path_buf();
    let faults = Faults {
        trash: Some(bin),
        ..Faults::default()
    };
    let model = cx.update(|cx| {
        CollectionModel::open_with_faults(root, Arc::new(MemoryStore::new()), faults, cx)
    });
    cx.run_until_parked();
    model
}

fn trash_item(cx: &mut TestAppContext, model: &Entity<CollectionModel>, rel: &str) -> u64 {
    let id = send(cx, model, Command::Tree(TreeOp::Trash { item: ip(rel) }));
    cx.run_until_parked();
    id
}

#[gpui_kit::test]
fn trashing_an_item_removes_it_and_reports_a_ticket(cx: &mut TestAppContext) {
    let dir = tree();
    let bin = Arc::new(FakeTrash::new());
    let model = open_with_trash(cx, &dir, bin.clone());
    let id = trash_item(cx, &model, "a.yml");
    assert_eq!(
        notices(cx, &model),
        [NoticeKind::Trashed {
            item: ip("a.yml"),
            ticket: id
        }]
    );
    assert!(!dir.path().join("a.yml").exists());
    assert!(!snap(cx, &model).content.requests.contains_key(&ip("a.yml")));
    assert_eq!(bin.len(), 1);
}

#[gpui_kit::test]
fn undo_puts_the_files_back_and_the_item_returns_to_the_tree(cx: &mut TestAppContext) {
    let dir = tree();
    let bin = Arc::new(FakeTrash::new());
    let model = open_with_trash(cx, &dir, bin.clone());
    let ticket = trash_item(cx, &model, "users");
    notices(cx, &model);
    send(cx, &model, Command::Tree(TreeOp::Restore { ticket }));
    cx.run_until_parked();
    assert_eq!(notices(cx, &model), [NoticeKind::Restored(ip("users"))]);
    assert!(dir.path().join("users/u.yml").exists());
    assert!(
        snap(cx, &model)
            .content
            .requests
            .contains_key(&ip("users/u.yml"))
    );
    assert_eq!(bin.len(), 0);
}

#[gpui_kit::test]
fn undo_after_the_toast_expired_does_nothing(cx: &mut TestAppContext) {
    let dir = tree();
    let bin = Arc::new(FakeTrash::new());
    let model = open_with_trash(cx, &dir, bin.clone());
    let ticket = trash_item(cx, &model, "a.yml");
    notices(cx, &model);
    // The toast closing is what forgets the ticket.
    send(cx, &model, Command::Tree(TreeOp::ForgetTrash { ticket }));
    send(cx, &model, Command::Tree(TreeOp::Restore { ticket }));
    cx.run_until_parked();
    assert_eq!(notices(cx, &model), []);
    assert!(!dir.path().join("a.yml").exists());
    assert_eq!(bin.len(), 1, "the item stays in the platform trash");
}

#[gpui_kit::test]
fn undo_onto_a_taken_name_fails_and_never_overwrites(cx: &mut TestAppContext) {
    let dir = tree();
    let bin = Arc::new(FakeTrash::new());
    let model = open_with_trash(cx, &dir, bin.clone());
    let ticket = trash_item(cx, &model, "a.yml");
    notices(cx, &model);
    let newer = request("newer", "http://x/9");
    std::fs::write(dir.path().join("a.yml"), &newer).unwrap();
    send(cx, &model, Command::Tree(TreeOp::Restore { ticket }));
    cx.run_until_parked();
    let got = notices(cx, &model);
    let [NoticeKind::Failed { message, .. }] = got.as_slice() else {
        panic!("{got:?}")
    };
    assert!(
        message.contains("already exists") && message.contains("left in the trash"),
        "{message}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.yml")).unwrap(),
        newer
    );
    assert_eq!(bin.len(), 1);
}

#[gpui_kit::test]
fn a_broken_trash_fails_the_delete_and_keeps_the_file(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open_with_trash(cx, &dir, Arc::new(FakeTrash::broken()));
    trash_item(cx, &model, "a.yml");
    let got = notices(cx, &model);
    assert!(
        matches!(
            got.as_slice(),
            [NoticeKind::Failed {
                target: Some(Target::Request(_)),
                ..
            }]
        ),
        "{got:?}"
    );
    assert!(dir.path().join("a.yml").exists());
}

/// The seqs of `files`, on disk and in the model, which must agree.
fn seqs<const N: usize>(
    cx: &mut TestAppContext,
    model: &Entity<CollectionModel>,
    dir: &tempfile::TempDir,
    files: [&str; N],
) -> [u64; N] {
    let s = snap(cx, model);
    files.map(|file| {
        let text = std::fs::read_to_string(dir.path().join(file)).unwrap();
        let line = text.lines().find(|l| l.trim_start().starts_with("seq:"));
        let on_disk: u64 = line.unwrap().trim()[4..].trim().parse().unwrap();
        assert_eq!(s.content.requests[&ip(file)].seq, Some(on_disk), "{file}");
        on_disk
    })
}

#[gpui_kit::test]
fn undoing_deletes_oldest_first_keeps_the_numbers_dense(cx: &mut TestAppContext) {
    let numbered = |name: &str, seq: &str| {
        request(name, "http://x/0").replace("seq: 1", &format!("seq: {seq}"))
    };
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &numbered("a", "1")),
        ("b.yml", &numbered("b", "2")),
        ("c.yml", &numbered("c", "3")),
    ]);
    let model = open_with_trash(cx, &dir, Arc::new(FakeTrash::new()));
    let a = trash_item(cx, &model, "a.yml");
    let b = trash_item(cx, &model, "b.yml");
    notices(cx, &model);

    send(cx, &model, Command::Tree(TreeOp::Restore { ticket: a }));
    cx.run_until_parked();
    assert_eq!(notices(cx, &model), [NoticeKind::Restored(ip("a.yml"))]);
    assert_eq!(seqs(cx, &model, &dir, ["a.yml", "c.yml"]), [1, 2]);

    send(cx, &model, Command::Tree(TreeOp::Restore { ticket: b }));
    cx.run_until_parked();
    assert_eq!(notices(cx, &model), [NoticeKind::Restored(ip("b.yml"))]);
    assert_eq!(
        seqs(cx, &model, &dir, ["a.yml", "b.yml", "c.yml"]),
        [1, 2, 3]
    );
}
