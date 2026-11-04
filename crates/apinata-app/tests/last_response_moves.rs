use crate::support;

use std::time::{Duration, UNIX_EPOCH};

use apinata_app::command::{Command, TreeOp};
use apinata_app::last_response::{Limits, ResponseStore, SendExtra};
use apinata_core::{ItemPath, Outcome, RequestResult, Response};
use gpui_kit::TestAppContext;
use support::*;

fn kept(item: &str) -> RequestResult {
    RequestResult::from_parts(
        ip(item),
        "x",
        Outcome::Passed,
        Some(Response::new(200).with_text_body(item)),
        Vec::new(),
    )
}

fn store_with(dir: &std::path::Path, root: &std::path::Path, items: &[&str]) -> ResponseStore {
    let store = ResponseStore::new(dir.join("responses"));
    for item in items {
        store
            .save(
                root,
                &ip(item),
                &kept(item),
                UNIX_EPOCH + Duration::from_secs(1),
            )
            .unwrap();
    }
    store
}

fn has(store: &ResponseStore, root: &std::path::Path, item: &str) -> bool {
    store.load(root, &ItemPath::new(item).unwrap()).is_some()
}

#[test]
fn a_folder_rename_moves_entries_under_it_by_whole_segments() {
    let data = tempfile::tempdir().unwrap();
    let root = std::path::Path::new("/c");
    let store = store_with(
        data.path(),
        root,
        &["users/u.yml", "users/deep/v.yml", "users2/x.yml", "a.yml"],
    );
    store.rename(root, &ip("users"), &ip("people")).unwrap();
    assert!(has(&store, root, "people/u.yml"));
    assert!(has(&store, root, "people/deep/v.yml"));
    assert!(!has(&store, root, "users/u.yml"));
    assert!(has(&store, root, "users2/x.yml"));
    assert!(has(&store, root, "a.yml"));
    // The moved entry keeps its body.
    let r = store.load(root, &ip("people/u.yml")).unwrap().result;
    assert_eq!(r.response.unwrap().text(), "users/u.yml");
    assert_eq!(r.path, ip("people/u.yml"));
}

#[test]
fn a_request_rename_moves_one_entry_and_other_roots_are_untouched() {
    let data = tempfile::tempdir().unwrap();
    let (c, d) = (std::path::Path::new("/c"), std::path::Path::new("/d"));
    let store = store_with(data.path(), c, &["a.yml"]);
    store
        .save(d, &ip("a.yml"), &kept("a.yml"), UNIX_EPOCH)
        .unwrap();
    store.rename(c, &ip("a.yml"), &ip("b.yml")).unwrap();
    assert!(has(&store, c, "b.yml"));
    assert!(!has(&store, c, "a.yml"));
    assert!(has(&store, d, "a.yml"));
}

#[test]
fn remove_drops_an_item_or_a_folder_with_its_files() {
    let data = tempfile::tempdir().unwrap();
    let root = std::path::Path::new("/c");
    let store = store_with(data.path(), root, &["users/u.yml", "users2/x.yml", "a.yml"]);
    let files = || {
        std::fs::read_dir(data.path().join("responses"))
            .unwrap()
            .count()
    };
    let before = files();
    store.remove(root, &ip("users")).unwrap();
    assert!(!has(&store, root, "users/u.yml"));
    assert!(has(&store, root, "users2/x.yml"));
    store.remove(root, &ip("a.yml")).unwrap();
    assert!(!has(&store, root, "a.yml"));
    assert_eq!(files(), before - 2, "both entry files are deleted");
}

#[test]
fn clear_removes_everything_including_orphans() {
    let data = tempfile::tempdir().unwrap();
    let root = std::path::Path::new("/c");
    let store = store_with(data.path(), root, &["a.yml"]);
    std::fs::write(
        data.path().join("responses/0000000000000000.json"),
        "orphan",
    )
    .unwrap();
    store.clear().unwrap();
    assert!(!data.path().join("responses").exists());
    assert!(!has(&store, root, "a.yml"));
    store.clear().unwrap();
}

#[gpui_kit::test]
fn queued_ops_run_in_arrival_order(cx: &mut TestAppContext) {
    use apinata_app::last_response::{StoreOp, enqueue, install};
    use std::sync::Arc;
    let data = tempfile::tempdir().unwrap();
    let root = std::path::PathBuf::from("/c");
    let store = store_with(data.path(), &root, &["a.yml"]);
    cx.update(|cx| install(store.clone(), cx));
    cx.update(|cx| {
        enqueue(
            StoreOp::Rename {
                root: root.clone(),
                from: ip("a.yml"),
                to: ip("b.yml"),
            },
            cx,
        );
        enqueue(
            StoreOp::Remove {
                root: root.clone(),
                item: ip("b.yml"),
            },
            cx,
        );
        let c = Arc::new(kept("c.yml"));
        enqueue(
            StoreOp::Save {
                root: root.clone(),
                item: ip("c.yml"),
                result: c,
                secrets: Vec::new(),
                saved_at: UNIX_EPOCH,
                limits: Limits::default(),
                extra: SendExtra::default(),
                kept: None,
            },
            cx,
        );
        enqueue(StoreOp::Clear, cx);
        let d = Arc::new(kept("d.yml"));
        enqueue(
            StoreOp::Save {
                root: root.clone(),
                item: ip("d.yml"),
                result: d,
                secrets: Vec::new(),
                saved_at: UNIX_EPOCH,
                limits: Limits::default(),
                extra: SendExtra::default(),
                kept: None,
            },
            cx,
        );
    });
    cx.run_until_parked();
    assert!(!has(&store, &root, "a.yml"));
    assert!(!has(&store, &root, "b.yml"));
    assert!(!has(&store, &root, "c.yml"));
    assert!(has(&store, &root, "d.yml"));
}

#[gpui_kit::test]
fn in_app_renames_and_deletes_reach_the_store(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://127.0.0.1:9/a")),
        ("users/folder.yml", &folder("Users")),
        ("users/u.yml", &request("u", "http://127.0.0.1:9/u")),
    ]);
    let (state, ws, root) = open_dir(cx, &col);
    let store = store_with(state.path(), &root, &["a.yml", "users/u.yml"]);
    cx.update(|cx| apinata_app::last_response::install(store.clone(), cx));
    let rename = TreeOp::Rename {
        item: ip("users"),
        name: "people".into(),
        rename_file: true,
    };
    ws.update(cx, |w, cx| w.send(&root, Command::Tree(rename), cx));
    cx.run_until_parked();
    assert!(has(&store, &root, "people/u.yml"));
    assert!(!has(&store, &root, "users/u.yml"));
    ws.update(cx, |w, cx| {
        w.send(
            &root,
            Command::Tree(TreeOp::Delete { item: ip("a.yml") }),
            cx,
        )
    });
    cx.run_until_parked();
    assert!(!has(&store, &root, "a.yml"));
    assert!(has(&store, &root, "people/u.yml"));
}
