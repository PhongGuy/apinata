use crate::support;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use apinata_app::command::Edit;
use apinata_app::draft_file;
use apinata_app::history_key::history_key;
use apinata_app::persist::{Paths, save_json};
use apinata_app::session::{CollectionSession, Session};
use apinata_core::{DraftSnapshot, ItemPath, MemoryStore};
use gpui_kit::{AppContext as _, Focusable, TestAppContext, VisualTestContext};
use support::*;

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://127.0.0.1:9/a")),
    ])
}

// Reopens draft `item` as a second tab under its history key `hk`, as a history entry does.
fn reopen_same_key(cx: &mut TestAppContext, s: &Started, item: &str, hk: &ItemPath) -> String {
    let snap = ws_snap(cx, &s.ws, &s.root).content.requests[&ip(item)]
        .snapshot
        .clone()
        .expect("a draft snapshot");
    s.ws.update(cx, |w, cx| {
        w.reopen_draft(&s.root, hk.clone(), 0, (*snap).clone(), cx)
    });
    cx.run_until_parked();
    s.ws.read_with(cx, |w, _| {
        let e = w.tabs().active().expect("the reopened tab");
        e.item().expect("a request").as_str().to_owned()
    })
}

// Two dirty tabs on one history key, each with its own URL.
fn two_dirty_tabs(cx: &mut TestAppContext, s: &Started) -> (String, String, ItemPath) {
    let (a, hk) = new_draft_at(cx, s, ItemPath::root());
    edit_item(cx, s, &a, Edit::Url("http://127.0.0.1:9/tab-a".into()));
    wait_save(cx);
    let b = reopen_same_key(cx, s, &a, &hk);
    assert_ne!(a, b);
    edit_item(cx, s, &b, Edit::Url("http://127.0.0.1:9/tab-b".into()));
    wait_save(cx);
    (a, b, hk)
}

fn kept_urls(s: &Started) -> Vec<String> {
    let loaded = draft_file::load_dir(&s.files.path().join("drafts"));
    assert!(loaded.skipped.is_empty(), "{:?}", loaded.skipped);
    let mut urls: Vec<String> = (loaded.drafts.iter())
        .map(|d| d.request.clone())
        .flat_map(|r| {
            ["tab-a", "tab-b"]
                .into_iter()
                .filter(move |u| r.contains(*u))
                .map(String::from)
        })
        .collect();
    urls.sort();
    urls
}

#[gpui_kit::test]
fn two_dirty_tabs_sharing_a_key_each_keep_a_file(cx: &mut TestAppContext) {
    let col = tree();
    let s = started(cx, &col);
    two_dirty_tabs(cx, &s);
    assert_eq!(kept_urls(&s), ["tab-a", "tab-b"]);
}

fn close_active_tab(cx: &mut TestAppContext, s: &Started) {
    let handle = s.shell.read_with(cx, |v, cx| v.focus_handle(cx));
    VisualTestContext::from_window(s.win, cx).update(|w, cx| handle.focus(w, cx));
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, &key("w"));
    frame(cx, s.win);
}

#[gpui_kit::test]
fn discarding_one_of_two_tabs_keeps_the_others_file(cx: &mut TestAppContext) {
    let col = tree();
    let s = started(cx, &col);
    two_dirty_tabs(cx, &s);
    close_active_tab(cx, &s);
    answer_confirm(cx, "confirm-discard");
    wait_save(cx);
    assert_eq!(kept_urls(&s), ["tab-a"]);
}

fn file_of(root: &Path, key: &ItemPath, slot: u32, url: &str) -> draft_file::DraftFile {
    let json = format!(
        r#"{{"info":{{"name":"Kept","type":"http"}},"http":{{"method":"POST","url":"{url}"}}}}"#
    );
    let snap = DraftSnapshot::from_json(&json).unwrap();
    draft_file::build_slot(key, slot, root, &ItemPath::root(), &snap, false, &[]).unwrap()
}

// A data directory as a previous run left it, with `files` stored under their own names.
fn data_dir(root: &Path, files: &[draft_file::DraftFile]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::in_dir(dir.path());
    save_json(&paths.settings, &test_settings()).unwrap();
    let mut session = Session {
        open: vec![root.to_path_buf()],
        ..Session::default()
    };
    session
        .collections
        .insert(root.to_path_buf(), CollectionSession::default());
    save_json(&paths.session, &session).unwrap();
    for f in files {
        let name = draft_file::file_name_slot(&f.item_key().unwrap(), f.slot);
        save_json(&paths.drafts.join(name), f).unwrap();
    }
    dir
}

fn restarted(cx: &mut TestAppContext, files: tempfile::TempDir, root: PathBuf) -> Started {
    use gpui_kit::test::TestWindowExt as _;
    init(cx);
    let paths = Paths::in_dir(files.path());
    let store = Arc::new(MemoryStore::new());
    let (win, shell) = cx
        .update(|cx| apinata_app::app::start(&paths, store, Vec::new(), cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    let ws = shell.read_with(cx, |s, _| s.workspace().clone());
    Started {
        files,
        root,
        win,
        shell,
        ws,
    }
}

fn restored_urls(cx: &mut TestAppContext, s: &Started) -> Vec<(ItemPath, String)> {
    let snap = ws_snap(cx, &s.ws, &s.root);
    let mut out: Vec<_> = s.ws.read_with(cx, |w, _| {
        (w.tabs().entries().iter())
            .filter_map(|e| {
                let item = e.item()?.clone();
                let (_, key) = history_key(w, e.id).filter(|_| item.is_draft())?;
                let url = snap.content.requests[&item].snapshot.as_ref()?.to_json();
                Some((key, url))
            })
            .collect()
    });
    out.sort();
    out
}

#[gpui_kit::test]
fn both_files_of_a_shared_key_restore_and_stay_two_files(cx: &mut TestAppContext) {
    let col = tree();
    let root = col.path().to_path_buf();
    let key = ip("untitled-1760600000001000007.draft");
    let a = file_of(&root, &key, 0, "http://127.0.0.1:9/tab-a");
    let b = file_of(&root, &key, 1, "http://127.0.0.1:9/tab-b");
    let s = restarted(cx, data_dir(&root, &[a, b]), root);
    let tabs = restored_urls(cx, &s);
    assert_eq!(tabs.len(), 2);
    assert!(tabs.iter().all(|(k, _)| *k == key), "one shared key");
    assert!(tabs[0].1.contains("tab-a") && tabs[1].1.contains("tab-b"));
    // Settling rewrites the files in place instead of minting more.
    wait_save(cx);
    let loaded = draft_file::load_dir(&s.files.path().join("drafts"));
    assert_eq!(loaded.drafts.len(), 2, "{:?}", loaded.skipped);
}

#[gpui_kit::test]
fn a_file_written_before_slots_existed_still_loads_and_restores(cx: &mut TestAppContext) {
    let col = tree();
    let root = col.path().to_path_buf();
    let key = ip("untitled-1760600000002000007.draft");
    let old = file_of(&root, &key, 0, "http://127.0.0.1:9/old");
    let files = data_dir(&root, &[]);
    let text = serde_json::to_string_pretty(&old).unwrap();
    assert!(!text.contains("slot"), "slot 0 keeps the old format");
    let dir = files.path().join("drafts");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("untitled-1760600000002000007.json"), text).unwrap();
    let s = restarted(cx, files, root);
    let tabs = restored_urls(cx, &s);
    assert_eq!(tabs.len(), 1);
    assert!(tabs[0].1.contains("/old"));
    // Dropping the draft removes the old-named file.
    close_active_tab(cx, &s);
    answer_confirm(cx, "confirm-discard");
    wait_save(cx);
    assert!(draft_file::load_dir(&dir).drafts.is_empty());
}

// The file name `slot` of `key` is stored under in the data directory of `s`.
fn slot_path(s: &Started, key: &ItemPath, slot: u32) -> PathBuf {
    s.files
        .path()
        .join("drafts")
        .join(draft_file::file_name_slot(key, slot))
}

fn slot_text(s: &Started, key: &ItemPath, slot: u32) -> Option<String> {
    std::fs::read_to_string(slot_path(s, key, slot)).ok()
}

// Item names sort as text, so ten tabs put `untitled-10` before `untitled-2`: a tab that takes
// the next free slot by item order would swap files with the tab restored from slot 1.
#[gpui_kit::test]
fn each_restored_tab_rewrites_only_the_file_it_came_from(cx: &mut TestAppContext) {
    let col = tree();
    let root = col.path().to_path_buf();
    let key = ip("untitled-1760600000003000007.draft");
    let files: Vec<_> = (0..10)
        .map(|n| file_of(&root, &key, n, &format!("http://127.0.0.1:9/tab-{n}")))
        .collect();
    let s = restarted(cx, data_dir(&root, &files), root);
    assert_eq!(restored_urls(cx, &s).len(), 10);
    wait_save(cx);
    for slot in 0..10 {
        let text = slot_text(&s, &key, slot).expect("the file stays");
        assert!(
            text.contains(&format!("/tab-{slot}\\\"")),
            "slot {slot} still holds its own request: {text}"
        );
    }
}

// A restore whose tab cannot be made leaves its file unread by anyone; a later tab on the same
// key must not be handed that file's slot and overwrite what the user never saw.
#[gpui_kit::test]
fn a_failed_restore_keeps_its_file_from_a_later_tab_of_the_same_key(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
    let col = tree();
    let root = col.path().to_path_buf();
    let key = ip("untitled-1760600000004000007.draft");
    let kept = file_of(&root, &key, 0, "http://127.0.0.1:9/never-seen");
    let files = data_dir(&root, &[kept]);
    let refuse = Arc::new(AtomicBool::new(true));
    init(cx);
    let paths = Paths::in_dir(files.path());
    let store = Arc::new(MemoryStore::new());
    let (win, shell) = cx
        .update(|cx| apinata_app::app::start(&paths, store, Vec::new(), cx))
        .unwrap();
    let ws = shell.read_with(cx, |s, _| s.workspace().clone());
    // Reopen the collection under an actor that refuses to make the restored tab.
    let faults = apinata_app::actor::Faults {
        refuse_draft_from: Some(refuse.clone()),
        ..Default::default()
    };
    ws.update(cx, |w, cx| {
        w.close(&root, cx);
        w.set_open_faults(faults);
        w.open(root.clone(), cx);
    });
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    let s = Started {
        files,
        root,
        win,
        shell,
        ws,
    };
    assert!(restored_urls(cx, &s).is_empty(), "the restore failed");
    let before = slot_text(&s, &key, 0).expect("the failed restore's file stays");
    // A later tab on the same history key, edited until it is written.
    refuse.store(false, SeqCst);
    let snap = DraftSnapshot::from_json(
        r#"{"info":{"name":"New","type":"http"},"http":{"method":"GET","url":"http://127.0.0.1:9/new"}}"#,
    )
    .unwrap();
    s.ws.update(cx, |w, cx| {
        w.reopen_draft(&s.root, key.clone(), 0, snap, cx)
    });
    cx.run_until_parked();
    let item = s.ws.read_with(cx, |w, _| {
        w.tabs()
            .active()
            .and_then(|e| e.item())
            .map(|i| i.as_str().to_owned())
    });
    edit_item(
        cx,
        &s,
        &item.expect("the new tab"),
        Edit::Url("http://127.0.0.1:9/typed".into()),
    );
    wait_save(cx);
    assert_eq!(
        slot_text(&s, &key, 0).as_deref(),
        Some(before.as_str()),
        "the restored file is untouched"
    );
    let loaded = draft_file::load_dir(&s.files.path().join("drafts"));
    assert_eq!(loaded.drafts.len(), 2, "the new tab has a file of its own");
    // Discarding the new tab removes its file by key, never the one nobody saw.
    close_active_tab(cx, &s);
    answer_confirm(cx, "confirm-discard");
    wait_save(cx);
    assert_eq!(slot_text(&s, &key, 0).as_deref(), Some(before.as_str()));
}
