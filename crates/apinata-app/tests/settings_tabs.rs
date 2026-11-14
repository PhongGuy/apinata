use crate::support;

use apinata_app::command::{Command, Edit, Target, TreeOp};
use apinata_app::tab_area::TabArea;
use apinata_app::tabs::{TabId, TabKind};
use apinata_app::workspace::Workspace;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use std::path::Path;
use support::*;

fn open(
    cx: &mut TestAppContext,
    ws: &Entity<Workspace>,
    root: &Path,
    kind: TabKind,
) -> Option<TabId> {
    ws.update(cx, |w, cx| w.open_settings(root, kind, cx))
}

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
        ("users/folder.yml", &folder("Users")),
        ("environments/dev.yml", "name: dev\n"),
    ])
}

#[gpui_kit::test]
fn settings_tabs_open_once_and_stay(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let a = open(cx, &ws, &root, TabKind::Folder(ip("users"))).expect("a folder");
    assert_eq!(open(cx, &ws, &root, TabKind::Folder(ip("users"))), Some(a));
    open(cx, &ws, &root, TabKind::Environments).unwrap();
    let c = open(cx, &ws, &root, TabKind::Collection).unwrap();
    ws.read_with(cx, |w, _| {
        assert_eq!(w.tabs().entries().len(), 3);
        assert!(w.tabs().entries().iter().all(|e| !e.preview));
        assert_eq!(w.tabs().active().unwrap().id, c);
    });
    assert!(open(cx, &ws, &root, TabKind::Folder(ip("nope"))).is_none());
    assert!(open(cx, &ws, &root, TabKind::Request(ip("a.yml"))).is_none());
}

#[gpui_kit::test]
fn a_folder_tab_follows_a_rename_and_closes_with_its_folder(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    ws.update(cx, |w, cx| {
        w.open_settings(&root, TabKind::Folder(ip("users")), cx)
    })
    .unwrap();
    let rename = TreeOp::Rename {
        item: ip("users"),
        name: "people".into(),
        rename_file: true,
    };
    ws.update(cx, |w, cx| w.send(&root, Command::Tree(rename), cx));
    cx.run_until_parked();
    assert_eq!(
        ws.read_with(cx, |w, _| w.tabs().entries()[0].kind.clone()),
        TabKind::Folder(ip("people"))
    );
    ws.update(cx, |w, cx| {
        w.send(
            &root,
            Command::Tree(TreeOp::Delete { item: ip("people") }),
            cx,
        )
    });
    cx.run_until_parked();
    assert!(ws.read_with(cx, |w, _| w.tabs().entries().is_empty()));
}

#[gpui_kit::test]
fn a_closed_settings_tab_reopens(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let id = open(cx, &ws, &root, TabKind::Environments).unwrap();
    ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.close(id)));
    let again = ws.update(cx, |w, cx| w.reopen_tab(cx)).expect("reopened");
    assert_eq!(
        ws.read_with(cx, |w, _| w.tabs().get(again).map(|e| e.kind.clone())),
        Some(TabKind::Environments)
    );
}

#[gpui_kit::test]
fn only_request_tabs_reach_the_session(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    ws.update(cx, |w, cx| w.open_item(&root, ip("a.yml"), false, cx))
        .unwrap();
    ws.update(cx, |w, cx| {
        w.open_settings(&root, TabKind::Environments, cx)
    })
    .unwrap();
    cx.run_until_parked();
    let session = ws.read_with(cx, |w, _| w.session().clone());
    assert_eq!(
        session_value(cx, &session).collections[&root].tabs,
        ["a.yml"]
    );
}

#[gpui_kit::test]
fn closing_a_dirty_environments_tab_prompts(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let shared = ws.clone();
    let (win, area) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| TabArea::new(shared, w, cx))
            })
        })
        .unwrap();
    let id = ws
        .update(cx, |w, cx| {
            w.open_settings(&root, TabKind::Environments, cx)
        })
        .unwrap();
    frame(cx, win);
    assert!(
        present(cx, win, "env-editor") && area.read_with(cx, |a, _| a.env_editor(id).is_some())
    );
    let color = Edit::Color(Some("#00ff00".into()));
    ws.update(cx, |w, cx| {
        w.send(
            &root,
            Command::Edit {
                target: Target::Environment("dev".into()),
                edit: color,
            },
            cx,
        )
    });
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        area.update(cx, |a, cx| a.close(vec![id], w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(confirm_open(cx));
    answer_confirm(cx, "confirm-discard");
    cx.run_until_parked();
    assert!(!env_view(cx, &ws, &root, "dev").dirty);
    assert!(ws.read_with(cx, |w, _| w.tabs().entries().is_empty()));
}

const SECRET_DEV: &str = "name: dev\nvariables:\n  - secret: true\n    name: token\n";

// Types into the secret row of an open environments tab without committing it, then
// dispatches `action`, the action of a shell-level tab key; the typing must reach the store
// though the input never blurred.
fn secret_typing_survives(cx: &mut TestAppContext, action: impl gpui_kit::Action) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", SECRET_DEV),
    ]);
    let store = std::sync::Arc::new(apinata_core::MemoryStore::new());
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let shared = ws.clone();
    let (win, _area) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| TabArea::new(shared, w, cx))
            })
        })
        .unwrap();
    open(cx, &ws, &root, TabKind::Environments).unwrap();
    frame(cx, win);
    type_at_end(cx, win, None, ("env-value", 0usize), "typed");
    cx.dispatch_action(win, action);
    cx.run_until_parked();
    let key = apinata_core::Collection::open(col.path())
        .unwrap()
        .secret_key("dev", "token");
    assert_eq!(
        apinata_core::SecretStore::get(&*store, &key)
            .unwrap()
            .as_deref(),
        Some("typed")
    );
}

#[gpui_kit::test]
fn closing_an_environments_tab_stores_secret_typing(cx: &mut TestAppContext) {
    secret_typing_survives(cx, apinata_app::tab_area::CloseTab);
}

#[gpui_kit::test]
fn save_all_stores_secret_typing(cx: &mut TestAppContext) {
    secret_typing_survives(cx, apinata_app::tab_area::SaveAll);
}

// Adds an unsaved secret row `api_key` typed `hunter2` to an open environments tab, closes
// the tab and answers the save prompt with `answer` (or runs Save All for `None`); returns
// what the store holds for the row and the environment file afterwards.
fn close_with_unsaved_secret(
    cx: &mut TestAppContext,
    answer: Option<&'static str>,
) -> (Option<String>, String) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", SECRET_DEV),
    ]);
    let store = std::sync::Arc::new(apinata_core::MemoryStore::new());
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let shared = ws.clone();
    let (win, _area) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| TabArea::new(shared, w, cx))
            })
        })
        .unwrap();
    open(cx, &ws, &root, TabKind::Environments).unwrap();
    frame(cx, win);
    type_at_end(cx, win, None, ("env-name", 1usize), "api_key");
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        use gpui_kit::test::TestWindowExt as _;
        w.click(("env-secret", 1usize), cx)
    })
    .unwrap();
    frame(cx, win);
    type_at_end(cx, win, None, ("env-value", 1usize), "hunter2");
    match answer {
        Some(answer) => {
            cx.dispatch_action(win, apinata_app::tab_area::CloseTab);
            cx.run_until_parked();
            answer_confirm(cx, answer);
        }
        None => {
            cx.dispatch_action(win, apinata_app::tab_area::SaveAll);
            cx.run_until_parked();
        }
    }
    let key = apinata_core::Collection::open(col.path())
        .unwrap()
        .secret_key("dev", "api_key");
    let stored = apinata_core::SecretStore::get(&*store, &key).unwrap();
    let file = std::fs::read_to_string(col.path().join("environments/dev.yml")).unwrap();
    (stored, file)
}

#[gpui_kit::test]
fn dont_save_on_close_drops_an_unsaved_secret_rows_value(cx: &mut TestAppContext) {
    let (stored, file) = close_with_unsaved_secret(cx, Some("confirm-discard"));
    assert_eq!(stored, None);
    assert_eq!(file, SECRET_DEV);
}

#[gpui_kit::test]
fn save_on_close_stores_an_unsaved_secret_rows_value(cx: &mut TestAppContext) {
    let (stored, file) = close_with_unsaved_secret(cx, Some("confirm-save"));
    assert_eq!(stored.as_deref(), Some("hunter2"));
    assert!(file.contains("name: api_key"), "{file}");
    assert!(!file.contains("hunter2"), "{file}");
}

#[gpui_kit::test]
fn save_all_stores_an_unsaved_secret_rows_value(cx: &mut TestAppContext) {
    let (stored, file) = close_with_unsaved_secret(cx, None);
    assert_eq!(stored.as_deref(), Some("hunter2"));
    assert!(file.contains("name: api_key"), "{file}");
    assert!(!file.contains("hunter2"), "{file}");
}

#[gpui_kit::test]
fn a_folder_tab_edits_its_folder_after_a_rename(cx: &mut TestAppContext) {
    use apinata_app::config_editor::ConfigSection;
    use apinata_app::kv_table::KvRow;
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let shared = ws.clone();
    let (win, area) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| TabArea::new(shared, w, cx))
            })
        })
        .unwrap();
    let id = open(cx, &ws, &root, TabKind::Folder(ip("users"))).unwrap();
    frame(cx, win);
    assert!(
        present(cx, win, "config-editor")
            && area.read_with(cx, |a, _| a.config_editor(id).is_some())
    );
    let rename = TreeOp::Rename {
        item: ip("users"),
        name: "people".into(),
        rename_file: true,
    };
    ws.update(cx, |w, cx| w.send(&root, Command::Tree(rename), cx));
    cx.run_until_parked();
    frame(cx, win);
    let row = KvRow {
        name: "X-Trace".into(),
        value: "1".into(),
        enabled: true,
        ..KvRow::default()
    };
    cx.update_window(win, |_, w, cx| {
        let ed = area.read(cx).config_editor(id).expect("mounted").clone();
        ed.update(cx, |e, cx| {
            e.edit_rows(ConfigSection::Headers, vec![row], w, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    let people = ws
        .read_with(cx, |w, cx| w.snapshot(&root, cx))
        .unwrap()
        .content
        .folders[&ip("people")]
        .clone();
    assert!(
        people.dirty && people.headers.len() == 1,
        "the edit landed on the renamed folder"
    );
}

#[gpui_kit::test]
fn the_collection_settings_tab_names_its_collection(cx: &mut TestAppContext) {
    let a = open_area(cx);
    a.ws.update(cx, |w, cx| {
        let _ = w.open_settings(&a.root, TabKind::Collection, cx);
    });
    cx.run_until_parked();
    let entry =
        a.ws.read_with(cx, |w, _| w.tabs().active().cloned())
            .expect("a tab");
    assert_eq!(
        a.view.read_with(cx, |v, cx| v.tab_label(&entry, cx)),
        "c settings"
    );
}

// The tab stays bound to the collection it was opened for, so its title must say which.
#[gpui_kit::test]
fn the_environments_tab_names_its_collection(cx: &mut TestAppContext) {
    let a = open_area(cx);
    a.ws.update(cx, |w, cx| {
        let _ = w.open_settings(&a.root, TabKind::Environments, cx);
    });
    cx.run_until_parked();
    let entry =
        a.ws.read_with(cx, |w, _| w.tabs().active().cloned())
            .expect("a tab");
    assert_eq!(
        a.view.read_with(cx, |v, cx| v.tab_label(&entry, cx)),
        "c environments"
    );
}

// A folder's settings tab says which collection the folder is in, as the collection's own
// tabs do, and its title follows the page heading's sentence case.
#[gpui_kit::test]
fn a_folder_settings_tab_names_its_collection(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let a = open_area_of(cx, tree());
    open(cx, &a.ws, &a.root, TabKind::Folder(ip("users")));
    cx.run_until_parked();
    let entry =
        a.ws.read_with(cx, |w, _| w.tabs().active().cloned())
            .expect("a tab");
    assert_eq!(
        a.view.read_with(cx, |v, cx| v.tab_label(&entry, cx)),
        "Users settings"
    );
    let detail = cx
        .update_window(a.win, |_, w, cx| {
            w.render_frame(cx);
            (gpui_kit::base::test_support::texts(w).iter())
                .filter(|t| t.path().contains(&("tab-detail", 0usize).into()))
                .map(|t| t.text().to_owned())
                .collect::<String>()
        })
        .unwrap();
    assert_eq!(detail, "c");
}
