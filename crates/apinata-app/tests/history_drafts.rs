//! A draft's kept send holds its request, blanked and scrubbed; saving the draft moves its history.

use crate::support;

use std::path::Path;
use std::sync::Arc;

use apinata_app::command::{Command, Edit, Target};
use apinata_app::last_response::{ResponseStore, install};
use apinata_app::request_editor::{RequestEditor, SendRequest};
use apinata_app::save_draft::{self, SaveAs};
use apinata_app::tabs::TabId;
use apinata_app::workspace::Workspace;
use apinata_core::{AuthSpec, ItemPath, MemoryStore, SecretStore as _};
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use support::*;
use wiremock::ResponseTemplate;

const TOKEN: &str = "Lit-T0ken-1";
const SECRET: &str = "Draft-S3cret-7";

fn draft_tab(cx: &mut TestAppContext, ws: &Entity<Workspace>, root: &Path) -> (TabId, String) {
    ws.update(cx, |w, cx| w.new_draft(root, ItemPath::root(), cx))
        .expect("open");
    cx.run_until_parked();
    ws.read_with(cx, |w, _| {
        let e = w.tabs().active().expect("the draft tab");
        (e.id, e.item().expect("a request").as_str().to_owned())
    })
}

fn send_from(cx: &mut TestAppContext, win: AnyWindowHandle, editor: &Entity<RequestEditor>) {
    cx.update_window(win, |_, w, cx| {
        editor.update(cx, |e, cx| e.send(&SendRequest, w, cx))
    })
    .unwrap();
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    finished(cx, &model);
}

fn key_of(cx: &mut TestAppContext, ws: &Entity<Workspace>, tab: TabId) -> ItemPath {
    ws.read_with(cx, |w, _| w.tabs().history_key(tab).cloned())
        .expect("a history key")
}

// Store files that contain `secret`.
fn leaks(dir: &Path, secret: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| {
            let path = e.ok()?.path();
            let text = std::fs::read_to_string(&path).ok()?;
            text.contains(secret).then(|| path.display().to_string())
        })
        .collect()
}

#[gpui_kit::test]
fn a_draft_send_keeps_its_request_with_literal_credentials_blanked(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    cx.update(|cx| install(store.clone(), cx));
    let (tab, draft) = draft_tab(cx, &ws, &root);
    let url = format!("{}/d", server.uri());
    ws.update(cx, |w, cx| w.send(&root, edit_url(&draft, &url), cx));
    let auth = Command::Edit {
        target: Target::Request(ip(&draft)),
        edit: Edit::Auth(AuthSpec::Bearer {
            token: TOKEN.into(),
        }),
    };
    ws.update(cx, |w, cx| w.send(&root, auth, cx));
    cx.run_until_parked();
    let (win, editor) = editor_for(cx, &ws, &root, &draft, false);
    send_from(cx, win, &editor);
    let key = key_of(cx, &ws, tab);
    wait_real(cx, |_| store.request_json(&root, &key, 0).is_some());
    let json: serde_json::Value =
        serde_json::from_str(&store.request_json(&root, &key, 0).unwrap()).unwrap();
    assert_eq!(json["http"]["url"], url.as_str());
    assert_eq!(json["http"]["auth"]["type"], "bearer");
    assert_eq!(json["http"]["auth"]["token"], "");
    assert!(
        leaks(store.dir(), TOKEN).is_empty(),
        "{:?}",
        leaks(store.dir(), TOKEN)
    );
}

#[gpui_kit::test]
fn a_draft_snapshot_never_keeps_an_environment_secret_value(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let dev = "name: dev\nvariables:\n  - name: token\n    secret: true\n";
    let col = write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", dev)]);
    let c = apinata_core::Collection::open(col.path()).unwrap();
    let secrets = MemoryStore::new();
    secrets.set(&c.secret_key("dev", "token"), SECRET).unwrap();
    let s = started_with_store(cx, &col, Arc::new(secrets));
    s.ws.update(cx, |w, cx| {
        w.select_environment(&s.root, Some("dev".into()), cx)
    });
    let (tab, draft) = draft_tab(cx, &s.ws, &s.root);
    let url = format!("{}/ok?t={{{{token}}}}&copy={SECRET}", server.uri());
    s.ws.update(cx, |w, cx| w.send(&s.root, edit_url(&draft, &url), cx));
    cx.run_until_parked();
    frame(cx, s.win);
    let editor = s
        .shell
        .read_with(cx, |sh, cx| sh.tab_area().read(cx).editor(tab).cloned())
        .expect("the draft's editor");
    send_from(cx, s.win, &editor);
    let key = key_of(cx, &s.ws, tab);
    let store = ResponseStore::new(s.files.path().join("responses"));
    wait_real(cx, |_| store.request_json(&s.root, &key, 0).is_some());
    let json = store.request_json(&s.root, &key, 0).unwrap();
    assert!(json.contains("{{token}}"), "{json}");
    assert!(!json.contains(SECRET), "{json}");
    assert!(
        leaks(store.dir(), SECRET).is_empty(),
        "{:?}",
        leaks(store.dir(), SECRET)
    );
}

// A password typed into a draft's url is no secret variable, but core reports it as sent, so
// the kept snapshot drops it too.
#[gpui_kit::test]
fn a_draft_snapshot_never_keeps_a_url_password(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    cx.update(|cx| install(store.clone(), cx));
    let (tab, draft) = draft_tab(cx, &ws, &root);
    let url = server.uri().replace("http://", "http://joe:hunter2pw@") + "/d";
    ws.update(cx, |w, cx| w.send(&root, edit_url(&draft, &url), cx));
    cx.run_until_parked();
    let (win, editor) = editor_for(cx, &ws, &root, &draft, false);
    send_from(cx, win, &editor);
    let key = key_of(cx, &ws, tab);
    wait_real(cx, |_| store.request_json(&root, &key, 0).is_some());
    assert!(
        leaks(store.dir(), "hunter2pw").is_empty(),
        "{:?}",
        leaks(store.dir(), "hunter2pw")
    );
}

// A password after an encoded colon in the user is sent by its place, so the kept send drops it.
#[gpui_kit::test]
fn a_draft_snapshot_never_keeps_a_password_after_an_encoded_colon(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    cx.update(|cx| install(store.clone(), cx));
    let (tab, draft) = draft_tab(cx, &ws, &root);
    let url = server.uri().replace("http://", "http://joe%3Ahunter2pw@") + "/d";
    ws.update(cx, |w, cx| w.send(&root, edit_url(&draft, &url), cx));
    cx.run_until_parked();
    let (win, editor) = editor_for(cx, &ws, &root, &draft, false);
    send_from(cx, win, &editor);
    let key = key_of(cx, &ws, tab);
    wait_real(cx, |_| store.request_json(&root, &key, 0).is_some());
    assert!(
        leaks(store.dir(), "hunter2pw").is_empty(),
        "{:?}",
        leaks(store.dir(), "hunter2pw")
    );
}

// A password typed in an unusual percent-encoding matches no wire form of the decoded value
// core derives, so the kept snapshot drops it by its place in the url instead.
#[gpui_kit::test]
fn a_draft_snapshot_never_keeps_an_oddly_encoded_url_password(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    cx.update(|cx| install(store.clone(), cx));
    let (tab, draft) = draft_tab(cx, &ws, &root);
    let url = server.uri().replace("http://", "http://joe:p%4a%4Bss@") + "/d";
    ws.update(cx, |w, cx| w.send(&root, edit_url(&draft, &url), cx));
    cx.run_until_parked();
    let (win, editor) = editor_for(cx, &ws, &root, &draft, false);
    send_from(cx, win, &editor);
    let key = key_of(cx, &ws, tab);
    wait_real(cx, |_| store.request_json(&root, &key, 0).is_some());
    let kept = store.request_json(&root, &key, 0).unwrap();
    assert!(kept.contains("joe@"), "{kept}");
    // Any case of the typed or the decoded password counts as a leak.
    for entry in std::fs::read_dir(store.dir()).unwrap() {
        let path = entry.unwrap().path();
        let text = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).to_lowercase();
        for form in ["p%4a%4bss", "pjkss"] {
            assert!(!text.contains(form), "{form} in {}", path.display());
        }
    }
}

#[gpui_kit::test]
fn a_saved_requests_send_keeps_no_snapshot(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    cx.update(|cx| install(store.clone(), cx));
    let (win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    send_from(cx, win, &editor);
    wait_real(cx, |_| store.load(&root, &ip("a.yml")).is_some());
    assert_eq!(store.request_json(&root, &ip("a.yml"), 0), None);
}

#[gpui_kit::test]
fn saving_a_draft_moves_its_kept_sends_to_the_new_file(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    cx.update(|cx| install(store.clone(), cx));
    let (tab, draft) = draft_tab(cx, &ws, &root);
    ws.update(cx, |w, cx| {
        w.send(&root, edit_url(&draft, &format!("{}/d", server.uri())), cx)
    });
    cx.run_until_parked();
    let (win, editor) = editor_for(cx, &ws, &root, &draft, false);
    send_from(cx, win, &editor);
    let key = key_of(cx, &ws, tab);
    wait_real(cx, |_| store.history(&root, &key).len() == 1);
    let save_as = SaveAs {
        name: "Kept".into(),
        parent: ItemPath::root(),
        target: None,
    };
    let (w2, r2) = (ws.clone(), root.clone());
    cx.update(|cx| save_draft::send(&w2, &r2, ip(&draft), save_as, cx))
        .expect("sent");
    wait_real(cx, |_| store.history(&root, &ip("Kept.yml")).len() == 1);
    assert!(store.history(&root, &key).is_empty());
    assert_eq!(
        ws.read_with(cx, |w, _| w.tabs().history_key(tab).cloned()),
        None
    );
    let item = ws.read_with(cx, |w, _| w.tabs().get(tab).and_then(|e| e.item().cloned()));
    assert_eq!(item, Some(ip("Kept.yml")));
    send_from(cx, win, &editor);
    wait_real(cx, |_| store.history(&root, &ip("Kept.yml")).len() == 2);
}

#[gpui_kit::test]
fn a_send_in_flight_when_its_draft_is_saved_lands_under_the_saved_request(cx: &mut TestAppContext) {
    http_ready(cx);
    let slow = ResponseTemplate::new(200).set_delay(std::time::Duration::from_millis(1500));
    let server = mock_server(cx, slow);
    let col = collection_for(&server.uri(), "");
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    cx.update(|cx| install(store.clone(), cx));
    let (tab, draft) = draft_tab(cx, &ws, &root);
    ws.update(cx, |w, cx| {
        w.send(&root, edit_url(&draft, &format!("{}/d", server.uri())), cx)
    });
    cx.run_until_parked();
    let (win, editor) = editor_for(cx, &ws, &root, &draft, false);
    let old = key_of(cx, &ws, tab);
    cx.update_window(win, |_, w, cx| {
        editor.update(cx, |e, cx| e.send(&SendRequest, w, cx))
    })
    .unwrap();
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    assert!(model.read_with(cx, |m, _| m.is_sending()));
    let save_as = SaveAs {
        name: "Kept".into(),
        parent: ItemPath::root(),
        target: None,
    };
    let (w2, r2) = (ws.clone(), root.clone());
    cx.update(|cx| save_draft::send(&w2, &r2, ip(&draft), save_as, cx))
        .expect("sent");
    wait_real(cx, |cx| {
        ws.read_with(cx, |w, _| w.tabs().history_key(tab).is_none())
    });
    assert!(
        model.read_with(cx, |m, _| m.is_sending()),
        "the save must land before the response"
    );
    finished(cx, &model);
    wait_real(cx, |_| store.history(&root, &ip("Kept.yml")).len() == 1);
    // Let any queued store write for the old key land before looking for an orphan.
    wait_real(cx, |_| store.history(&root, &old).is_empty());
    std::thread::sleep(std::time::Duration::from_millis(200));
    cx.run_until_parked();
    assert!(store.history(&root, &old).is_empty(), "orphan entry");
    assert_eq!(store.history(&root, &ip("Kept.yml")).len(), 1);
}
