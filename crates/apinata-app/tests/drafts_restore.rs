use crate::support;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use apinata_app::draft_file::{self, DraftFile};
use apinata_app::history_key::history_key;
use apinata_app::last_response::ResponseStore;
use apinata_app::persist::{Paths, save_json};
use apinata_app::session::{CollectionSession, SavedTab, Session};
use apinata_core::{DraftSnapshot, ItemPath, MemoryStore, Outcome, RequestResult, Response};
use gpui_kit::component::WindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://127.0.0.1:9/a")),
        ("f/folder.yml", &folder("f")),
    ])
}

fn draft(root: &Path, ms: u64, parent: &str, url: &str, blanked: bool) -> DraftFile {
    let json = format!(
        r#"{{"info":{{"name":"Kept {ms}","type":"http"}},"http":{{"method":"POST","url":"{url}"}}}}"#
    );
    let snap = DraftSnapshot::from_json(&json).unwrap();
    let key = ip(&format!("untitled-{ms}000007.draft"));
    draft_file::build(
        &key,
        root,
        &ItemPath::new(parent).unwrap(),
        &snap,
        blanked,
        &[],
    )
    .unwrap()
}

// A data directory as a previous run left it: settings, a session with `root` open, and drafts.
fn data_dir(root: &Path, collection: CollectionSession, drafts: &[DraftFile]) -> tempfile::TempDir {
    let files = tempfile::tempdir().unwrap();
    let paths = Paths::in_dir(files.path());
    save_json(&paths.settings, &test_settings()).unwrap();
    let mut session = Session {
        open: vec![root.to_path_buf()],
        ..Session::default()
    };
    session.collections.insert(root.to_path_buf(), collection);
    save_json(&paths.session, &session).unwrap();
    for d in drafts {
        save_json(
            &paths
                .drafts
                .join(draft_file::file_name(&d.item_key().unwrap())),
            d,
        )
        .unwrap();
    }
    files
}

// `app::start` over an existing data directory, as a relaunch with kept data.
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

// Each open draft tab of `root` as (item, history key), in tab order.
fn draft_tabs(cx: &mut TestAppContext, s: &Started) -> Vec<(ItemPath, ItemPath)> {
    s.ws.read_with(cx, |w, _| {
        w.tabs()
            .entries()
            .iter()
            .filter_map(|e| {
                let item = e.item()?.clone();
                let (_, key) = history_key(w, e.id).filter(|_| item.is_draft())?;
                Some((item, key))
            })
            .collect()
    })
}

fn toasts(cx: &mut TestAppContext, s: &Started) -> usize {
    cx.update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

#[gpui_kit::test]
fn drafts_restore_dirty_with_their_keys_after_the_saved_tabs(cx: &mut TestAppContext) {
    let col = tree();
    let root = col.path().to_path_buf();
    let older = draft(
        &root,
        1_760_600_000_001,
        "f",
        "http://127.0.0.1:9/one",
        false,
    );
    let newer = draft(
        &root,
        1_760_600_000_002,
        "gone",
        "http://127.0.0.1:9/two",
        false,
    );
    let saved = CollectionSession {
        tabs: vec!["a.yml".into()],
        ..CollectionSession::default()
    };
    let s = restarted(
        cx,
        data_dir(&root, saved, &[newer.clone(), older.clone()]),
        root.clone(),
    );
    assert_eq!(tab_names(cx, &s.ws)[0], "a.yml");
    let tabs = draft_tabs(cx, &s);
    let keys: Vec<_> = tabs.iter().map(|(_, k)| k.as_str().to_owned()).collect();
    assert_eq!(
        keys,
        [older.key.clone(), newer.key.clone()],
        "oldest first, same keys"
    );
    assert_eq!(tabs[0].0.parent(), Some(ip("f")));
    assert_eq!(
        tabs[1].0.parent(),
        Some(ItemPath::root()),
        "a vanished folder falls back to the root"
    );
    let snap = ws_snap(cx, &s.ws, &s.root);
    let view = &snap.content.requests[&tabs[1].0];
    assert_eq!(view.url, "http://127.0.0.1:9/two");
    assert!(view.dirty, "a restored draft is unsaved");
    assert_eq!(toasts(cx, &s), 1, "one toast says the drafts were reopened");
}

#[test]
fn the_reopened_toast_names_one_draft_and_counts_more() {
    use apinata_app::draft_store::reopened_message;
    assert_eq!(
        reopened_message(&["Untitled request".to_owned()]),
        "Reopened \"Untitled request\"."
    );
    assert_eq!(
        reopened_message(&["a".to_owned(), "b".to_owned()]),
        "Reopened 2 unsaved drafts."
    );
}

#[test]
fn the_reopened_toast_says_when_credentials_were_cleared() {
    use apinata_app::draft_store::reopened_blanked_message;
    assert_eq!(
        reopened_blanked_message(&["a".to_owned()]),
        "Reopened \"a\". Its credentials were cleared; enter them again before sending."
    );
    assert_eq!(
        reopened_blanked_message(&["a".to_owned(), "b".to_owned()]),
        "Reopened 2 unsaved drafts. Some credentials were cleared; enter them again before sending."
    );
}

#[gpui_kit::test]
fn restored_drafts_reopen_with_one_toast(cx: &mut TestAppContext) {
    let col = tree();
    let root = col.path().to_path_buf();
    let one = draft(
        &root,
        1_760_600_000_001,
        "",
        "http://127.0.0.1:9/one",
        false,
    );
    let two = draft(
        &root,
        1_760_600_000_002,
        "",
        "http://127.0.0.1:9/two",
        false,
    );
    let s = restarted(
        cx,
        data_dir(&root, CollectionSession::default(), &[one, two]),
        root,
    );
    assert_eq!(draft_tabs(cx, &s).len(), 2);
    cx.run_until_parked();
    assert_eq!(toasts(cx, &s), 1, "one toast for both drafts");
}

#[gpui_kit::test]
fn a_start_with_no_drafts_shows_no_toast(cx: &mut TestAppContext) {
    let col = tree();
    let root = col.path().to_path_buf();
    let s = restarted(cx, data_dir(&root, CollectionSession::default(), &[]), root);
    cx.run_until_parked();
    assert_eq!(toasts(cx, &s), 0);
}

#[gpui_kit::test]
fn the_active_draft_is_active_again(cx: &mut TestAppContext) {
    let col = tree();
    let root = col.path().to_path_buf();
    let one = draft(
        &root,
        1_760_600_000_001,
        "",
        "http://127.0.0.1:9/one",
        false,
    );
    let two = draft(
        &root,
        1_760_600_000_002,
        "",
        "http://127.0.0.1:9/two",
        false,
    );
    let saved = CollectionSession {
        tabs: vec!["a.yml".into()],
        active_draft: Some(one.key.clone()),
        ..CollectionSession::default()
    };
    let s = restarted(cx, data_dir(&root, saved, &[one.clone(), two]), root);
    let active =
        s.ws.read_with(cx, |w, _| history_key(w, w.tabs().active().unwrap().id));
    assert_eq!(active.map(|(_, k)| k.as_str().to_owned()), Some(one.key));
}

#[gpui_kit::test]
fn a_saved_active_tab_stays_active_after_drafts_open(cx: &mut TestAppContext) {
    let col = tree();
    let root = col.path().to_path_buf();
    let one = draft(
        &root,
        1_760_600_000_001,
        "",
        "http://127.0.0.1:9/one",
        false,
    );
    let saved = CollectionSession {
        tabs: vec!["a.yml".into()],
        active: Some(SavedTab::Request("a.yml".into())),
        ..CollectionSession::default()
    };
    let s = restarted(cx, data_dir(&root, saved, &[one]), root);
    let active =
        s.ws.read_with(cx, |w, _| w.tabs().active().and_then(|e| e.item().cloned()));
    assert_eq!(active, Some(ip("a.yml")));
}

#[gpui_kit::test]
fn blanked_credentials_toast_once_and_survive_and_bad_neighbours_do_not_stop_startup(
    cx: &mut TestAppContext,
) {
    let col = tree();
    let root = col.path().to_path_buf();
    let good = draft(&root, 1_760_600_000_001, "", "http://127.0.0.1:9/one", true);
    let files = data_dir(
        &root,
        CollectionSession::default(),
        std::slice::from_ref(&good),
    );
    let drafts = files.path().join("drafts");
    std::fs::write(drafts.join("corrupt.json"), "not json").unwrap();
    let mut v2 = draft(
        &root,
        1_760_600_000_002,
        "",
        "http://127.0.0.1:9/two",
        false,
    );
    v2.version = 2;
    save_json(
        &drafts.join(draft_file::file_name(&v2.item_key().unwrap())),
        &v2,
    )
    .unwrap();
    std::fs::write(
        drafts.join("huge.json"),
        " ".repeat(draft_file::MAX_DRAFT_BYTES + 1),
    )
    .unwrap();
    let s = restarted(cx, files, root);
    assert_eq!(draft_tabs(cx, &s).len(), 1);
    assert_eq!(
        toasts(cx, &s),
        1,
        "one reopened toast, saying the credentials were cleared"
    );
    assert!(drafts.join("corrupt.json").exists() && drafts.join("huge.json").exists());
    wait_save(cx);
    let kept = draft_file::load_dir(&drafts);
    let again = kept
        .drafts
        .iter()
        .find(|d| d.key == good.key)
        .expect("rewritten, still valid");
    assert!(
        !again.credentials_blanked,
        "the toast was shown, so the rewrite clears the flag and the next start stays quiet"
    );
}

#[gpui_kit::test]
fn the_blanked_credentials_toast_shows_once_per_draft_across_restarts(cx: &mut TestAppContext) {
    let col = tree();
    let root = col.path().to_path_buf();
    let kept = draft(&root, 1_760_600_000_001, "", "http://127.0.0.1:9/one", true);
    let files = data_dir(
        &root,
        CollectionSession::default(),
        std::slice::from_ref(&kept),
    );
    let drafts = files.path().join("drafts");
    let s = restarted(cx, files, root.clone());
    assert_eq!(toasts(cx, &s), 1, "the reopened toast informs the user");
    wait_save(cx);
    let after = draft_file::load_dir(&drafts);
    assert_eq!(after.drafts.len(), 1);
    assert!(!after.drafts[0].credentials_blanked, "informed once");
    // A second start over the files the first one left shows nothing.
    let second = data_dir(&root, CollectionSession::default(), &after.drafts);
    let s2 = restarted(cx, second, root);
    assert_eq!(draft_tabs(cx, &s2).len(), 1);
    assert_eq!(
        toasts(cx, &s2),
        1,
        "the second restart shows only the plain reopened toast"
    );
}

#[gpui_kit::test]
fn a_restored_draft_keeps_the_key_its_history_is_stored_under(cx: &mut TestAppContext) {
    let col = tree();
    let root = col.path().to_path_buf();
    let one = draft(
        &root,
        1_760_600_000_001,
        "",
        "http://127.0.0.1:9/one",
        false,
    );
    let key = one.item_key().unwrap();
    let files = data_dir(&root, CollectionSession::default(), &[one]);
    let store = ResponseStore::new(Paths::in_dir(files.path()).responses);
    let response = Response::new(200).with_text_body("kept");
    let result = RequestResult::from_parts(
        key.clone(),
        "Kept",
        Outcome::Passed,
        Some(response),
        Vec::new(),
    );
    store
        .save(&root, &key, &result, std::time::SystemTime::now())
        .unwrap();
    let s = restarted(cx, files, root.clone());
    let tabs = draft_tabs(cx, &s);
    assert_eq!(
        tabs.iter().map(|(_, k)| k.clone()).collect::<Vec<_>>(),
        std::slice::from_ref(&key)
    );
    assert_eq!(
        store.history(&root, &tabs[0].1).len(),
        1,
        "the entry stored under that key is still there"
    );
}

#[gpui_kit::test]
fn a_second_instance_restores_nothing(cx: &mut TestAppContext) {
    let col = tree();
    let root = col.path().to_path_buf();
    let one = draft(
        &root,
        1_760_600_000_001,
        "",
        "http://127.0.0.1:9/one",
        false,
    );
    let files = data_dir(
        &root,
        CollectionSession::default(),
        std::slice::from_ref(&one),
    );
    let _held = draft_file::lock(&files.path().join("drafts")).expect("free");
    let s = restarted(cx, files, root);
    assert!(draft_tabs(cx, &s).is_empty());
    let path = s
        .files
        .path()
        .join("drafts")
        .join(draft_file::file_name(&one.item_key().unwrap()));
    assert!(path.exists());
}

#[gpui_kit::test]
fn a_drafts_collection_restores_it_whenever_it_opens(cx: &mut TestAppContext) {
    let (col, other) = (tree(), tree());
    let (root, later) = (col.path().to_path_buf(), other.path().to_path_buf());
    let waiting = draft(
        &later,
        1_760_600_000_001,
        "",
        "http://127.0.0.1:9/later",
        false,
    );
    let s = restarted(
        cx,
        data_dir(
            &root,
            CollectionSession::default(),
            std::slice::from_ref(&waiting),
        ),
        root,
    );
    assert!(draft_tabs(cx, &s).is_empty());
    s.ws.update(cx, |w, cx| w.open(later.clone(), cx));
    cx.run_until_parked();
    let keys: Vec<_> = draft_tabs(cx, &s)
        .into_iter()
        .map(|(_, k)| k.as_str().to_owned())
        .collect();
    assert_eq!(keys, [waiting.key]);
}

#[gpui_kit::test]
fn a_restored_draft_grown_past_one_mib_loses_its_loaded_file(cx: &mut TestAppContext) {
    let col = tree();
    let root = col.path().to_path_buf();
    let one = draft(
        &root,
        1_760_600_000_001,
        "",
        "http://127.0.0.1:9/one",
        false,
    );
    let key = one.item_key().unwrap();
    let s = restarted(
        cx,
        data_dir(&root, CollectionSession::default(), &[one]),
        root,
    );
    let (item, _) = draft_tabs(cx, &s).remove(0);
    assert!(draft_path(&s, &key).exists());
    let big = apinata_app::command::Edit::Docs("x".repeat(draft_file::MAX_DRAFT_BYTES));
    edit_item(cx, &s, item.as_str(), big);
    wait_save(cx);
    assert!(
        !draft_path(&s, &key).exists(),
        "the loaded file would restore older content"
    );
}

// A new draft of `root` in `s`, edited so it is written; its history key.
fn edited_draft(cx: &mut TestAppContext, s: &Started, root: &Path) -> ItemPath {
    s.ws.update(cx, |w, cx| w.new_draft(root, ItemPath::root(), cx))
        .expect("open");
    cx.run_until_parked();
    let (item, key) = s.ws.read_with(cx, |w, _| {
        let e = w.tabs().active().expect("the draft tab");
        (
            e.item().cloned().expect("a request"),
            w.tabs().history_key(e.id).cloned().expect("keyed"),
        )
    });
    let command = apinata_app::command::Command::Edit {
        target: apinata_app::command::Target::Request(item),
        edit: apinata_app::command::Edit::Url("http://127.0.0.1:9/new".into()),
    };
    s.ws.update(cx, |w, cx| w.send(root, command, cx));
    wait_save(cx);
    key
}

// The restored root has no secret list (its `.env` is a directory), so its drafts keep the
// files they were loaded with; those still count toward the cap, once each.
#[gpui_kit::test]
fn restored_drafts_count_once_toward_the_cap(cx: &mut TestAppContext) {
    let (col, other) = (tree(), tree());
    std::fs::create_dir(col.path().join(".env")).unwrap();
    let (root, later) = (col.path().to_path_buf(), other.path().to_path_buf());
    let kept: Vec<_> = (1..draft_file::MAX_DRAFTS as u64)
        .map(|i| {
            draft(
                &root,
                1_760_600_000_000 + i,
                "",
                "http://127.0.0.1:9/k",
                false,
            )
        })
        .collect();
    let s = restarted(
        cx,
        data_dir(&root, CollectionSession::default(), &kept),
        root,
    );
    assert_eq!(draft_tabs(cx, &s).len(), draft_file::MAX_DRAFTS - 1);
    s.ws.update(cx, |w, cx| w.open(later.clone(), cx));
    cx.run_until_parked();
    let first = edited_draft(cx, &s, &later);
    assert!(
        draft_path(&s, &first).exists(),
        "one below the cap, so the new draft is kept"
    );
    let second = edited_draft(cx, &s, &later);
    assert!(
        !draft_path(&s, &second).exists(),
        "the restored files count toward the cap"
    );
}

#[gpui_kit::test]
fn another_collections_saved_active_tab_stays_active_after_drafts_open(cx: &mut TestAppContext) {
    let (col_b, col_a) = (tree(), tree());
    let (b, a) = (col_b.path().to_path_buf(), col_a.path().to_path_buf());
    let kept = draft(&a, 1_760_600_000_001, "", "http://127.0.0.1:9/one", false);
    // B opens first and owns the window's active tab; A only brings back a draft.
    let files = data_dir(
        &b,
        CollectionSession {
            tabs: vec!["a.yml".into()],
            active: Some(SavedTab::Request("a.yml".into())),
            ..CollectionSession::default()
        },
        &[kept],
    );
    let paths = Paths::in_dir(files.path());
    let mut session: Session =
        serde_json::from_slice(&std::fs::read(&paths.session).unwrap()).unwrap();
    session.open.push(a.clone());
    session
        .collections
        .insert(a.clone(), CollectionSession::default());
    save_json(&paths.session, &session).unwrap();
    let s = restarted(cx, files, b.clone());
    assert_eq!(draft_tabs(cx, &s).len(), 1, "A's draft is restored");
    let active = s.ws.read_with(cx, |w, _| {
        w.tabs()
            .active()
            .map(|e| (e.root.clone(), e.item().cloned()))
    });
    assert_eq!(active, Some((b, Some(ip("a.yml")))));
}

#[gpui_kit::test]
fn a_later_open_leaves_the_users_tab_for_its_restored_draft(cx: &mut TestAppContext) {
    let (col_b, col_a) = (tree(), tree());
    let (b, a) = (col_b.path().to_path_buf(), col_a.path().to_path_buf());
    let kept = draft(&a, 1_760_600_000_001, "", "http://127.0.0.1:9/one", false);
    let files = data_dir(
        &b,
        CollectionSession {
            tabs: vec!["a.yml".into()],
            active: Some(SavedTab::Request("a.yml".into())),
            ..CollectionSession::default()
        },
        &[kept],
    );
    let s = restarted(cx, files, b.clone());
    // The user moves off the startup tab, then opens A, whose kept draft comes back.
    new_draft_at(cx, &s, ItemPath::root());
    s.ws.update(cx, |w, cx| w.open(a.clone(), cx));
    cx.run_until_parked();
    let active = s.ws.read_with(cx, |w, _| {
        w.tabs()
            .active()
            .map(|e| (e.root.clone(), e.item().is_some_and(|i| i.is_draft())))
    });
    assert_eq!(active, Some((a, true)), "A's restored draft stays active");
}

// A loaded file of slot 1 whose root is not open stays untaken; discarding its key removes
// every file of the key, not only slot 0's.
#[gpui_kit::test]
fn discarding_a_key_removes_an_untaken_file_of_a_later_slot(cx: &mut TestAppContext) {
    let (col, unopened) = (tree(), tree());
    let root = col.path().to_path_buf();
    let d = draft(
        unopened.path(),
        1_760_600_000_000,
        "",
        "http://127.0.0.1:9/k",
        false,
    );
    let key = d.item_key().unwrap();
    let d = DraftFile { slot: 1, ..d };
    let files = data_dir(&root, CollectionSession::default(), &[]);
    let path = Paths::in_dir(files.path())
        .drafts
        .join(draft_file::file_name_slot(&key, 1));
    save_json(&path, &d).unwrap();
    let _s = restarted(cx, files, root);
    assert!(path.exists(), "loaded and left on disk");
    cx.update(|cx| apinata_app::draft_store::discard(&key, cx));
    cx.run_until_parked();
    assert!(!path.exists(), "no file of the key remains");
}

#[gpui_kit::test]
fn a_literal_credential_typed_into_a_restored_draft_sets_the_flag_again(cx: &mut TestAppContext) {
    let col = tree();
    let root = col.path().to_path_buf();
    let kept = draft(&root, 1_760_600_000_001, "", "http://127.0.0.1:9/one", true);
    let s = restarted(
        cx,
        data_dir(&root, CollectionSession::default(), &[kept]),
        root,
    );
    wait_save(cx);
    let flag = |s: &Started| {
        draft_file::load_dir(&s.files.path().join("drafts")).drafts[0].credentials_blanked
    };
    assert!(!flag(&s), "the toast was shown");
    let (item, _) = draft_tabs(cx, &s).remove(0);
    let token = apinata_app::command::Edit::Auth(apinata_core::AuthSpec::Bearer {
        token: "literal-bearer-901".into(),
    });
    edit_item(cx, &s, item.as_str(), token);
    cx.run_until_parked();
    cx.update(|cx| apinata_app::draft_store::flush_now(None, cx));
    assert!(flag(&s), "a new literal credential is blanked and flagged");
}

// The moment `take_root` hands a file over, the tab made from it matches that file, so a window
// closed before the first secret list arrives must not prompt for it or delete it.
#[gpui_kit::test]
fn a_taken_draft_is_kept_before_any_secret_list_arrives(cx: &mut TestAppContext) {
    use apinata_app::draft_store::DraftStore;
    let root = PathBuf::from("/some/root");
    let one = draft(
        &root,
        1_760_600_000_001,
        "",
        "http://127.0.0.1:9/one",
        false,
    );
    let dir = tempfile::tempdir().unwrap();
    let key = one.item_key().unwrap();
    save_json(&dir.path().join(draft_file::file_name(&key)), &one).unwrap();
    let store = cx.new(|cx| DraftStore::open(dir.path().to_path_buf(), cx));
    let taken = store.update(cx, |s, cx| s.take_root(&root, cx));
    assert_eq!(taken.len(), 1);
    let snapshot = taken[0].snapshot().unwrap();
    assert!(store.read_with(cx, |s, cx| s.is_kept(&key, &snapshot, false, cx)));
    let edited = DraftSnapshot::from_json(
        r#"{"info":{"name":"Kept","type":"http"},"http":{"method":"POST","url":"http://x/new"}}"#,
    )
    .unwrap();
    assert!(
        !store.read_with(cx, |s, cx| s.is_kept(&key, &edited, false, cx)),
        "an edited tab no longer matches the file"
    );
}

#[gpui_kit::test]
fn closing_right_after_a_restore_asks_nothing_and_keeps_the_file(cx: &mut TestAppContext) {
    use gpui_kit::VisualTestContext;
    let col = tree();
    let root = col.path().to_path_buf();
    let one = draft(
        &root,
        1_760_600_000_001,
        "",
        "http://127.0.0.1:9/one",
        false,
    );
    let files = data_dir(
        &root,
        CollectionSession::default(),
        std::slice::from_ref(&one),
    );
    let s = restarted(cx, files, root.clone());
    assert_eq!(draft_tabs(cx, &s).len(), 1);
    assert!(VisualTestContext::from_window(s.win, cx).simulate_close());
    assert!(!confirm_open(cx));
    let on_disk = Paths::in_dir(s.files.path())
        .drafts
        .join(draft_file::file_name(&one.item_key().unwrap()));
    assert!(on_disk.exists());
}

#[gpui_kit::test]
fn a_url_password_is_kept_out_of_the_draft_file_and_restores_with_the_toast(
    cx: &mut TestAppContext,
) {
    let col = tree();
    let s = started(cx, &col);
    let root = s.root.clone();
    let (item, key) = new_draft_at(cx, &s, ItemPath::root());
    let url = apinata_app::command::Edit::Url("http://u:hunter2pw@127.0.0.1/x".into());
    edit_item(cx, &s, &item, url);
    wait_save(cx);
    let text = std::fs::read_to_string(draft_path(&s, &key)).expect("written");
    let loaded = draft_file::load_dir(&s.files.path().join("drafts"));
    assert_eq!(loaded.drafts.len(), 1, "{:?}", loaded.skipped);
    assert!(!text.contains("hunter2pw"), "leaked: {text}");
    assert!(
        loaded.drafts[0].request.contains("http://u@127.0.0.1/x"),
        "{text}"
    );
    let again = restarted(
        cx,
        data_dir(&root, CollectionSession::default(), &loaded.drafts),
        root,
    );
    assert_eq!(draft_tabs(cx, &again).len(), 1);
    assert_eq!(toasts(cx, &again), 1, "the blanked-credentials toast");
}

#[gpui_kit::test]
fn an_encoded_colon_url_password_is_kept_out_of_the_draft_file_and_restores_with_the_toast(
    cx: &mut TestAppContext,
) {
    let col = tree();
    let s = started(cx, &col);
    let root = s.root.clone();
    let (item, key) = new_draft_at(cx, &s, ItemPath::root());
    let url = apinata_app::command::Edit::Url("http://u%3Ahunter2pw@127.0.0.1/x".into());
    edit_item(cx, &s, &item, url);
    wait_save(cx);
    let text = std::fs::read_to_string(draft_path(&s, &key)).expect("written");
    let loaded = draft_file::load_dir(&s.files.path().join("drafts"));
    assert_eq!(loaded.drafts.len(), 1, "{:?}", loaded.skipped);
    assert!(!text.contains("hunter2pw"), "leaked: {text}");
    assert!(
        loaded.drafts[0].request.contains("http://u@127.0.0.1/x"),
        "{text}"
    );
    let again = restarted(
        cx,
        data_dir(&root, CollectionSession::default(), &loaded.drafts),
        root,
    );
    assert_eq!(draft_tabs(cx, &again).len(), 1);
    assert_eq!(toasts(cx, &again), 1, "the blanked-credentials toast");
}
