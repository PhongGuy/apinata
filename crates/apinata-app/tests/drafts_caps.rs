use crate::support;

use std::cell::RefCell;
use std::rc::Rc;

use apinata_app::command::Edit;
use apinata_app::draft_file::{self, MAX_DRAFT_BYTES};
use apinata_app::draft_store::{DraftStoreEvent, Drafts, NotKeptReason};
use apinata_app::persist::save_json;
use apinata_core::{DraftSnapshot, ItemPath};
use gpui_kit::component::WindowExt;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

fn events(cx: &mut TestAppContext) -> Rc<RefCell<Vec<DraftStoreEvent>>> {
    let store = cx.update(|cx| cx.global::<Drafts>().0.clone());
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    cx.update(|cx| {
        cx.subscribe(&store, move |_, e: &DraftStoreEvent, _| {
            sink.borrow_mut().push(e.clone())
        })
        .detach()
    });
    seen
}

#[gpui_kit::test]
fn a_draft_over_one_mib_is_dropped_with_one_toast_and_settles(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let store = cx.update(|cx| cx.global::<Drafts>().0.clone());
    let seen = events(cx);
    let toasts = |cx: &mut TestAppContext| {
        cx.update_window(s.win, |_, w, cx| w.notifications(cx).len())
            .unwrap()
    };
    let (item, hk) = new_draft_at(cx, &s, ItemPath::root());
    edit_item(cx, &s, &item, Edit::Url("http://127.0.0.1:9/small".into()));
    wait_save(cx);
    assert!(draft_path(&s, &hk).exists());
    let before = toasts(cx);
    edit_item(cx, &s, &item, Edit::Docs("x".repeat(MAX_DRAFT_BYTES)));
    wait_save(cx);
    edit_item(cx, &s, &item, Edit::Docs("y".repeat(MAX_DRAFT_BYTES)));
    wait_save(cx);
    assert!(!draft_path(&s, &hk).exists(), "the old file goes too");
    let not_kept: Vec<_> = seen
        .borrow()
        .iter()
        .filter(|e| matches!(e, DraftStoreEvent::NotKept { .. }))
        .cloned()
        .collect();
    assert_eq!(
        not_kept,
        [DraftStoreEvent::NotKept {
            name: "Untitled request".into(),
            reason: NotKeptReason::TooLarge
        }]
    );
    assert_eq!(toasts(cx) - before, 1);
    let fetches = store.read_with(cx, |d, _| d.fetches());
    edit_item(
        cx,
        &s,
        "a.yml",
        Edit::Url("http://127.0.0.1:9/saved".into()),
    );
    wait_save(cx);
    assert_eq!(
        store.read_with(cx, |d, _| d.fetches()),
        fetches,
        "an over-size draft is not refetched"
    );
    edit_item(cx, &s, &item, Edit::Docs("small again".into()));
    wait_save(cx);
    assert!(
        draft_path(&s, &hk).exists(),
        "a draft that fits again is written again"
    );
}

#[gpui_kit::test]
fn the_fifty_first_draft_is_not_written(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started_with(cx, &col, |paths| {
        let snap =
            DraftSnapshot::from_json(r#"{"info":{"name":"Elsewhere","type":"http"},"http":{}}"#)
                .unwrap();
        for i in 0..50u64 {
            let key = ip(&format!("untitled-{}{:06}.draft", 1_760_600_000_000 + i, i));
            let file = draft_file::build(
                &key,
                std::path::Path::new("/elsewhere/c"),
                &ItemPath::root(),
                &snap,
                false,
                &[],
            )
            .unwrap();
            save_json(&paths.drafts.join(draft_file::file_name(&key)), &file).unwrap();
        }
    });
    let seen = events(cx);
    let (item, hk) = new_draft_at(cx, &s, ItemPath::root());
    edit_item(cx, &s, &item, Edit::Url("http://127.0.0.1:9/late".into()));
    wait_save(cx);
    assert!(!draft_path(&s, &hk).exists());
    assert!(seen.borrow().contains(&DraftStoreEvent::NotKept {
        name: "Untitled request".into(),
        reason: NotKeptReason::TooMany
    }));
}

// `n` files for keys made at distinct times, `per_key` slots each, belonging to another collection.
fn elsewhere(paths: &apinata_app::persist::Paths, n: u64, per_key: u32) {
    let snap = DraftSnapshot::from_json(r#"{"info":{"name":"Elsewhere","type":"http"},"http":{}}"#)
        .unwrap();
    for i in 0..n {
        let key = ip(&format!("untitled-{}{:06}.draft", 1_760_600_000_000 + i, i));
        for slot in 0..per_key {
            let file = draft_file::build_slot(
                &key,
                slot,
                std::path::Path::new("/elsewhere/c"),
                &ItemPath::root(),
                &snap,
                false,
                &[],
            )
            .unwrap();
            let name = draft_file::file_name_slot(&key, slot);
            save_json(&paths.drafts.join(name), &file).unwrap();
        }
    }
}

fn too_many(seen: &Rc<RefCell<Vec<DraftStoreEvent>>>) -> usize {
    (seen.borrow().iter())
        .filter(|e| {
            matches!(
                e,
                DraftStoreEvent::NotKept {
                    reason: NotKeptReason::TooMany,
                    ..
                }
            )
        })
        .count()
}

#[gpui_kit::test]
fn every_slot_of_a_shared_key_counts_toward_the_cap(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started_with(cx, &col, |paths| elsewhere(paths, 25, 2));
    let seen = events(cx);
    let (item, hk) = new_draft_at(cx, &s, ItemPath::root());
    edit_item(cx, &s, &item, Edit::Url("http://127.0.0.1:9/late".into()));
    wait_save(cx);
    assert!(!draft_path(&s, &hk).exists(), "25 keys of 2 slots fill 50");
    assert_eq!(too_many(&seen), 1);
}

#[gpui_kit::test]
fn restored_and_new_drafts_share_the_fifty_and_the_fifty_first_is_refused(cx: &mut TestAppContext) {
    let col = two_requests();
    let root = col.path().to_path_buf();
    let s = started_with(cx, &col, |paths| {
        elsewhere(paths, 48, 1);
        let snap = DraftSnapshot::from_json(
            r#"{"info":{"name":"Mine","type":"http"},"http":{"url":"http://127.0.0.1:9/mine"}}"#,
        )
        .unwrap();
        let key = ip("untitled-1760600000900000007.draft");
        let file = draft_file::build(&key, &root, &ItemPath::root(), &snap, false, &[]).unwrap();
        save_json(&paths.drafts.join(draft_file::file_name(&key)), &file).unwrap();
    });
    let seen = events(cx);
    // 48 elsewhere plus the restored one make 49; one new draft fits, the next does not.
    let (first, hk1) = new_draft_at(cx, &s, ItemPath::root());
    edit_item(cx, &s, &first, Edit::Url("http://127.0.0.1:9/one".into()));
    wait_save(cx);
    assert!(draft_path(&s, &hk1).exists(), "the 50th is kept");
    assert_eq!(too_many(&seen), 0);
    let (second, hk2) = new_draft_at(cx, &s, ItemPath::root());
    edit_item(cx, &s, &second, Edit::Url("http://127.0.0.1:9/two".into()));
    wait_save(cx);
    assert!(!draft_path(&s, &hk2).exists(), "the 51st is refused");
    assert_eq!(too_many(&seen), 1);
}

#[gpui_kit::test]
fn a_null_request_file_does_not_count_toward_the_cap(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started_with(cx, &col, |paths| {
        elsewhere(paths, 49, 1);
        let key = ip("untitled-1760600000800000007.draft");
        let snap =
            DraftSnapshot::from_json(r#"{"info":{"name":"N","type":"http"},"http":{}}"#).unwrap();
        let mut file = draft_file::build(
            &key,
            std::path::Path::new("/elsewhere/c"),
            &ItemPath::root(),
            &snap,
            false,
            &[],
        )
        .unwrap();
        file.request = "null".into();
        save_json(&paths.drafts.join(draft_file::file_name(&key)), &file).unwrap();
    });
    let (item, hk) = new_draft_at(cx, &s, ItemPath::root());
    edit_item(cx, &s, &item, Edit::Url("http://127.0.0.1:9/late".into()));
    wait_save(cx);
    assert!(draft_path(&s, &hk).exists(), "49 valid files leave room");
}

#[gpui_kit::test]
fn a_draft_json_cannot_hold_says_so_instead_of_claiming_one_mib(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let seen = events(cx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("w.yml");
    std::fs::write(
        &path,
        "info:\n  name: W\n  type: http\nhttp:\n  method: GET\n  url: http://127.0.0.1:9/w\n? [a, b]\n: c\n",
    )
    .unwrap();
    let snap = apinata_core::RequestFile::load(&path).unwrap().snapshot();
    let hk = ip("untitled-1760600000700000007.draft");
    s.ws.update(cx, |w, cx| w.reopen_draft(&s.root, hk.clone(), 0, snap, cx));
    cx.run_until_parked();
    let item = s.ws.read_with(cx, |w, _| {
        w.tabs()
            .active()
            .unwrap()
            .item()
            .unwrap()
            .as_str()
            .to_owned()
    });
    edit_item(cx, &s, &item, Edit::Url("http://127.0.0.1:9/edited".into()));
    wait_save(cx);
    assert!(
        !draft_path(&s, &hk).exists(),
        "a null request is never written"
    );
    let reasons: Vec<_> = (seen.borrow().iter())
        .filter_map(|e| match e {
            DraftStoreEvent::NotKept { reason, .. } => Some(*reason),
            _ => None,
        })
        .collect();
    assert_eq!(reasons, [NotKeptReason::Unserializable]);
}
