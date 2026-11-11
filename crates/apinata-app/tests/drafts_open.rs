use crate::support;

use apinata_app::command::{Command, Edit, Target, TreeOp};
use apinata_core::ItemPath;
use gpui_kit::{AppContext, Focusable, TestAppContext, VisualTestContext};
use support::*;

fn tree_with(base: &str) -> tempfile::TempDir {
    let users = format!(
        "info:\n  name: users\n  type: folder\n  seq: 2\nrequest:\n  variables:\n    - name: base\n      value: \"{base}\"\n"
    );
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/a")),
        ("users/folder.yml", &users),
        ("users/u.yml", &request("u", "http://x/u")),
    ])
}

fn listing(root: &std::path::Path) -> Vec<String> {
    fn walk(dir: &std::path::Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                out.push(path.to_string_lossy().into_owned());
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out.sort();
    out
}

fn focus_shell(cx: &mut TestAppContext, s: &Started) {
    let handle = s.shell.read_with(cx, |v, cx| v.focus_handle(cx));
    let mut v = VisualTestContext::from_window(s.win, cx);
    v.update(|w, cx| handle.focus(w, cx));
    frame(cx, s.win);
}

fn active_item(
    cx: &mut TestAppContext,
    ws: &gpui_kit::Entity<apinata_app::workspace::Workspace>,
) -> Option<String> {
    ws.read_with(cx, |w, _| {
        w.tabs()
            .active()
            .and_then(|e| e.item())
            .map(|p| p.as_str().to_owned())
    })
}

#[gpui_kit::test]
fn ctrl_n_opens_a_focused_untitled_draft(cx: &mut TestAppContext) {
    let col = tree_with("http://x");
    let s = started(cx, &col);
    let files = listing(col.path());
    let rows = ws_snap(cx, &s.ws, &s.root).content.rows.len();
    focus_shell(cx, &s);
    cx.simulate_keystrokes(s.win, &key("n"));
    frame(cx, s.win);
    let item = ip(&active_item(cx, &s.ws).expect("a tab opened"));
    assert!(item.is_draft(), "{item:?}");
    assert_eq!(item.parent(), Some(ItemPath::root()));
    let snap = ws_snap(cx, &s.ws, &s.root);
    assert_eq!(snap.content.requests[&item].name, "Untitled request");
    assert_eq!(snap.content.rows.len(), rows, "the sidebar lists no draft");
    assert_eq!(listing(col.path()), files, "nothing written");
    let area = s.shell.read_with(cx, |v, _| v.tab_area().clone());
    let focus = area
        .read_with(cx, |a, cx| a.active_focus(cx))
        .expect("an active tab");
    let focused = cx
        .update_window(s.win, |_, w, cx| focus.contains_focused(w, cx))
        .unwrap();
    assert!(focused, "the draft has focus");
    {
        use gpui_kit::test::TestWindowExt as _;
        cx.update_window(s.win, |_, w, cx| w.input("http://x/1", cx))
            .unwrap();
    }
    frame(cx, s.win);
    wait_for(cx, "the typed url to reach the draft", |cx| {
        ws_snap(cx, &s.ws, &s.root).content.requests[&item].url == "http://x/1"
    });
    cx.simulate_keystrokes(s.win, &key("n"));
    frame(cx, s.win);
    assert_eq!(tab_names(cx, &s.ws).len(), 2, "each ctrl+n is a new draft");
}

#[gpui_kit::test]
fn ctrl_n_puts_the_draft_in_the_selected_folder(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let col = tree_with("http://x");
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| w.click(row("users"), cx))
        .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, &key("n"));
    frame(cx, s.win);
    let drafts: Vec<ItemPath> = tab_names(cx, &s.ws)
        .iter()
        .map(|t| ip(t))
        .filter(ItemPath::is_draft)
        .collect();
    let [draft] = drafts.as_slice() else {
        panic!("{drafts:?}")
    };
    assert_eq!(draft.parent(), Some(ip("users")));
}

#[gpui_kit::test]
fn a_draft_is_sent_from_memory_with_its_folders_variables(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let col = tree_with(&server.uri());
    let (_state, ws, root) = open_dir(cx, &col);
    ws.update(cx, |w, cx| w.new_draft(&root, ip("users"), cx))
        .expect("open");
    cx.run_until_parked();
    let draft = ip(&active_item(cx, &ws).expect("a draft tab"));
    let edit = Command::Edit {
        target: Target::Request(draft.clone()),
        edit: Edit::Url("{{base}}/ok".into()),
    };
    ws.update(cx, |w, cx| w.send(&root, edit, cx));
    cx.run_until_parked();
    let model = sender(cx, &ws);
    let (r, d) = (root.clone(), draft.clone());
    assert!(model.update(cx, |m, cx| m.send(&r, d, cx)));
    finished(cx, &model);
    assert_eq!(served(cx, &server), ["/ok"]);
    let unsaved = ws.read_with(cx, |w, cx| w.unsaved(None, cx));
    let targets: Vec<_> = unsaved
        .iter()
        .map(|u| (u.root.clone(), u.targets.clone()))
        .collect();
    assert_eq!(
        targets,
        vec![(root.clone(), vec![Target::Request(draft.clone())])]
    );
}

#[gpui_kit::test]
fn drafts_stay_out_of_the_session_and_the_reopen_stack(cx: &mut TestAppContext) {
    let col = tree_with("http://x");
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, apinata_app::session::Session::default());
    let ws = workspace(cx, &session);
    let root = col.path().to_path_buf();
    ws.update(cx, |w, cx| w.open(root.clone(), cx));
    cx.run_until_parked();
    open_tab(cx, &ws, &root, "a.yml");
    ws.update(cx, |w, cx| w.new_draft(&root, ItemPath::root(), cx))
        .expect("open");
    cx.run_until_parked();
    assert_eq!(tab_names(cx, &ws).len(), 2);
    settle(cx);
    let saved = session_value(cx, &session)
        .collections
        .get(&root)
        .map(|c| c.tabs.clone())
        .unwrap_or_default();
    assert_eq!(saved, ["a.yml"]);
    let draft = ws
        .read_with(cx, |w, _| w.tabs().active().map(|e| e.id))
        .unwrap();
    ws.update(cx, |w, cx| {
        w.update_tabs(cx, |t| {
            t.close(draft);
        })
    });
    assert!(
        !ws.read_with(cx, |w, _| w.tabs().has_closed()),
        "a closed draft leaves nothing to reopen"
    );
    ws.update(cx, |w, cx| w.reopen_tab(cx));
    cx.run_until_parked();
    assert_eq!(
        tab_names(cx, &ws),
        ["a.yml"],
        "a closed draft is not reopened"
    );
}

// Command ids restart with each actor, so a draft asked for before a close must not open
// under a command of the reopened collection.
#[gpui_kit::test]
fn a_closed_collection_forgets_its_pending_drafts(cx: &mut TestAppContext) {
    let col = tree_with("http://x");
    let (_state, ws, root) = open_dir(cx, &col);
    ws.update(cx, |w, cx| w.new_draft(&root, ItemPath::root(), cx))
        .expect("open");
    ws.update(cx, |w, cx| w.close(&root, cx));
    cx.run_until_parked();
    ws.update(cx, |w, cx| w.open(root.clone(), cx));
    cx.run_until_parked();
    let folder = TreeOp::CreateFolder {
        parent: ItemPath::root(),
        name: "x".into(),
    };
    let draft = TreeOp::CreateDraft {
        parent: ItemPath::root(),
        kind: apinata_core::RequestKind::Http,
    };
    for op in [draft, folder] {
        ws.update(cx, |w, cx| w.send(&root, Command::Tree(op), cx));
        cx.run_until_parked();
    }
    assert_eq!(tab_names(cx, &ws), Vec::<String>::new());
}

#[gpui_kit::test]
fn two_drafts_get_names_that_tell_their_tabs_apart(cx: &mut TestAppContext) {
    let col = tree_with("http://x");
    let s = started(cx, &col);
    focus_shell(cx, &s);
    for _ in 0..2 {
        cx.simulate_keystrokes(s.win, &key("n"));
        frame(cx, s.win);
    }
    let snap = ws_snap(cx, &s.ws, &s.root);
    let entries = s.ws.read_with(cx, |w, _| w.tabs().entries().to_vec());
    let labels: Vec<String> = entries
        .iter()
        .map(|e| snap.content.requests[e.item().unwrap()].name.clone())
        .collect();
    assert_eq!(labels, ["Untitled request", "Untitled request 2"]);
    let places = vec![vec!["c".to_string()]; 2];
    let details = apinata_app::tabs::tab_details(&entries, &labels, &places);
    assert_eq!(details, [None, None], "distinct names need no folder");
}

#[test]
fn a_drafts_breadcrumb_shows_its_name() {
    use apinata_app::request_editor::crumbs;
    let draft = ip("users/untitled-3.draft");
    assert!(draft.is_draft());
    assert_eq!(
        crumbs(&draft, "Untitled request 2"),
        (
            vec![("users".to_string(), ip("users"))],
            "Untitled request 2".to_string()
        )
    );
    assert_eq!(
        crumbs(&ip("users/u.yml"), "u"),
        (vec![("users".to_string(), ip("users"))], "u".to_string()),
        "a saved request shows its display name too"
    );
}
