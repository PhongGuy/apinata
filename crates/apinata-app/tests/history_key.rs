//! The history key: one rule names where a tab's sends are kept and read.

use crate::support;

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use apinata_app::history_key::{history_key, new_history_key};
use apinata_app::last_response::{ResponseStore, install};
use apinata_app::request_editor::SendRequest;
use apinata_app::tabs::{TabId, TabKind, Tabs};
use apinata_app::workspace::Workspace;
use apinata_core::{ItemPath, Outcome, RequestResult, Response};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use support::*;
use wiremock::ResponseTemplate;

fn draft_tab(cx: &mut TestAppContext, ws: &Entity<Workspace>, root: &Path) -> (TabId, String) {
    ws.update(cx, |w, cx| w.new_draft(root, ItemPath::root(), cx))
        .expect("open");
    cx.run_until_parked();
    ws.read_with(cx, |w, _| {
        let e = w.tabs().active().expect("the draft tab");
        (e.id, e.item().expect("a request").as_str().to_owned())
    })
}

#[test]
fn a_history_key_is_a_root_level_draft_path_stamped_with_the_send_time() {
    let at = UNIX_EPOCH + Duration::from_millis(1_760_000_000_123);
    let key = new_history_key(at);
    assert!(key.is_draft(), "{key:?}");
    assert_eq!(key.parent(), Some(ItemPath::root()));
    let digits = key
        .as_str()
        .strip_prefix("untitled-")
        .and_then(|s| s.strip_suffix(".draft"))
        .expect("draft-shaped");
    assert_eq!(digits.len(), 19, "{digits}");
    assert!(digits.starts_with("1760000000123"), "{digits}");
}

#[test]
fn a_tab_keeps_its_history_key_until_it_closes() {
    let mut tabs = Tabs::new();
    let root = Path::new("/c");
    let key = ip("untitled-17600000001230000042.draft");
    let id = tabs.open(root, TabKind::Request(ip("untitled-1.draft")), false);
    assert_eq!(tabs.history_key(id), None);
    tabs.set_history_key(id, key.clone());
    assert_eq!(tabs.history_key(id), Some(&key));
    tabs.close(id);
    assert_eq!(tabs.history_key(id), None);
    let other = tabs.open(root, TabKind::Request(ip("untitled-2.draft")), false);
    tabs.set_history_key(other, key.clone());
    tabs.retain(|_| false);
    assert_eq!(tabs.history_key(other), None);
}

#[gpui_kit::test]
fn the_rule_names_a_saved_file_or_a_keyed_draft_and_nothing_else(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let saved = open_tab(cx, &ws, &root, "a.yml");
    let rule = |cx: &mut TestAppContext, tab: TabId| ws.read_with(cx, |w, _| history_key(w, tab));
    assert_eq!(rule(cx, saved), Some((root.clone(), ip("a.yml"))));
    let (draft, item) = draft_tab(cx, &ws, &root);
    let created = rule(cx, draft).expect("a new draft is keyed when it is created");
    assert_eq!(created.0, root);
    assert!(
        created.1.is_draft() && created.1.parent() == Some(ItemPath::root()),
        "{created:?}"
    );
    assert_ne!(created.1.as_str(), item, "a live draft path is never a key");
    let key = new_history_key(SystemTime::now());
    ws.update(cx, |w, cx| {
        w.update_tabs(cx, |t| t.set_history_key(draft, key.clone()))
    });
    assert_eq!(rule(cx, draft), Some((root.clone(), key)));
    std::fs::remove_file(col.path().join("a.yml")).unwrap();
    assert_eq!(
        rule(cx, saved),
        None,
        "a saved request keeps only while its file exists"
    );
}

#[gpui_kit::test]
fn a_drafts_sends_are_kept_under_its_tab_key_and_listed_in_its_pane(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200).set_body_string("d"));
    let col = collection_for(&server.uri(), "");
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    cx.update(|cx| install(store.clone(), cx));
    let (tab, draft) = draft_tab(cx, &ws, &root);
    let url = format!("{}/d", server.uri());
    ws.update(cx, |w, cx| w.send(&root, edit_url(&draft, &url), cx));
    cx.run_until_parked();
    let (win, editor) = editor_for(cx, &ws, &root, &draft, false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    for _ in 0..2 {
        cx.update_window(win, |_, w, cx| {
            editor.update(cx, |e, cx| e.send(&SendRequest, w, cx))
        })
        .unwrap();
        finished(cx, &model);
    }
    wait_real(cx, |cx| pane.read_with(cx, |p, _| p.history().len() == 2));
    let key = ws
        .read_with(cx, |w, _| w.tabs().history_key(tab).cloned())
        .expect("the first send keyed the tab");
    assert!(
        key.is_draft() && key.parent() == Some(ItemPath::root()),
        "{key:?}"
    );
    assert_ne!(key.as_str(), draft, "a live draft path is never a key");
    assert_eq!(store.history(&root, &key).len(), 2);
    assert!(store.history(&root, &ip(&draft)).is_empty());
    pane.update(cx, |p, cx| p.show_history(0, cx));
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.viewing()) == Some(0));
}

#[gpui_kit::test]
fn a_saved_requests_send_from_its_editor_is_kept_under_its_path(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200).set_body_string("s"));
    let col = collection_for(&server.uri(), "");
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    cx.update(|cx| install(store.clone(), cx));
    let (win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    cx.update_window(win, |_, w, cx| {
        editor.update(cx, |e, cx| e.send(&SendRequest, w, cx))
    })
    .unwrap();
    finished(cx, &model);
    wait_real(cx, |_| store.load(&root, &ip("a.yml")).is_some());
    let tab = ws.read_with(cx, |w, _| w.tabs().active().expect("a tab").id);
    assert_eq!(
        ws.read_with(cx, |w, _| w.tabs().history_key(tab).cloned()),
        None
    );
}

#[gpui_kit::test]
fn a_send_without_a_key_is_not_kept(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200).set_body_string("x"));
    let col = collection_for(&server.uri(), "");
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    cx.update(|cx| install(store.clone(), cx));
    let model = sender(cx, &ws);
    let r = root.clone();
    assert!(model.update(cx, |m, cx| m.send_keyed(&r, ip("a.yml"), None, cx)));
    finished(cx, &model);
    cx.run_until_parked();
    assert!(store.load(&root, &ip("a.yml")).is_none());
}

#[gpui_kit::test]
fn the_per_request_list_is_previous_runs(cx: &mut TestAppContext) {
    let col = two_requests();
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    let response = Response::new(201).with_text_body("k");
    let result = RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(response),
        Vec::new(),
    );
    store
        .save(&root, &ip("a.yml"), &result, SystemTime::now())
        .unwrap();
    cx.update(|cx| install(store, cx));
    let (win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    wait_real(cx, |cx| pane.read_with(cx, |p, _| p.history().len() == 1));
    let label = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find("response-history").label().map(str::to_owned)
        })
        .unwrap();
    assert_eq!(label.as_deref(), Some("Previous runs"));
}

#[gpui_kit::test]
fn a_draft_keeps_its_creation_key_through_its_sends(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200).set_body_string("k"));
    let col = collection_for(&server.uri(), "");
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    cx.update(|cx| install(store.clone(), cx));
    let (tab, draft) = draft_tab(cx, &ws, &root);
    let created = ws
        .read_with(cx, |w, _| w.tabs().history_key(tab).cloned())
        .expect("keyed at creation");
    let url = format!("{}/k", server.uri());
    ws.update(cx, |w, cx| w.send(&root, edit_url(&draft, &url), cx));
    cx.run_until_parked();
    let (win, editor) = editor_for(cx, &ws, &root, &draft, false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    cx.update_window(win, |_, w, cx| {
        editor.update(cx, |e, cx| e.send(&SendRequest, w, cx))
    })
    .unwrap();
    finished(cx, &model);
    wait_real(cx, |_| store.history(&root, &created).len() == 1);
    assert_eq!(
        ws.read_with(cx, |w, _| w.tabs().history_key(tab).cloned()),
        Some(created)
    );
}

#[gpui_kit::test]
fn draft_key_finds_the_key_of_a_drafts_tab(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (tab, draft) = draft_tab(cx, &ws, &root);
    let key = ws.read_with(cx, |w, _| w.tabs().history_key(tab).cloned());
    assert!(key.is_some());
    assert_eq!(
        ws.read_with(cx, |w, _| w.draft_key(&root, &ip(&draft))),
        key
    );
    assert_eq!(
        ws.read_with(cx, |w, _| w.draft_key(&root, &ip("a.yml"))),
        None
    );
}
