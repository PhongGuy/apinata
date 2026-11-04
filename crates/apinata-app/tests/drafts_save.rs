use crate::support;

use apinata_app::command::{Command, Edit, Target};
use apinata_app::save_draft::{self, SaveDraftForm};
use apinata_core::ItemPath;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext, Focusable, SharedString, TestAppContext, VisualTestContext};
use support::*;

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/a")),
        ("users/folder.yml", &folder("users")),
        ("users/admin/folder.yml", &folder("admin")),
    ])
}

fn focus_shell(cx: &mut TestAppContext, s: &Started) {
    let handle = s.shell.read_with(cx, |v, cx| v.focus_handle(cx));
    let mut v = VisualTestContext::from_window(s.win, cx);
    v.update(|w, cx| handle.focus(w, cx));
    frame(cx, s.win);
}

fn active(cx: &mut TestAppContext, s: &Started) -> (apinata_app::tabs::TabId, String) {
    s.ws.read_with(cx, |w, _| {
        let e = w.tabs().active().expect("an active tab");
        (e.id, e.item().expect("a request").as_str().to_owned())
    })
}

#[gpui_kit::test]
fn the_folder_list_is_the_root_then_every_folder(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let snap = ws_snap(cx, &ws, &root);
    let listed: Vec<(String, String)> = save_draft::folders(&snap.content)
        .into_iter()
        .map(|(p, l)| (p.as_str().to_owned(), l.to_string()))
        .collect();
    let want = vec![
        (String::new(), "Collection root".to_owned()),
        ("users".to_owned(), "users".to_owned()),
        ("users/admin".to_owned(), "users/admin".to_owned()),
    ];
    assert_eq!(listed, want);
}

#[gpui_kit::test]
fn the_form_answers_a_trimmed_name_and_the_chosen_folder(cx: &mut TestAppContext) {
    support::init(cx);
    let folders: Vec<(ItemPath, SharedString)> = vec![
        (ItemPath::root(), "Collection root".into()),
        (ip("users"), "users".into()),
    ];
    let (win, form) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| SaveDraftForm::new(folders, ip("users"), "Untitled request", w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    assert_eq!(form.read_with(cx, |f, cx| f.folder(cx)), ip("users"));
    let answer = form.read_with(cx, |f, cx| f.answer(cx)).expect("a name");
    assert_eq!(
        (answer.name.as_str(), answer.parent),
        ("Untitled request", ip("users"))
    );
    cx.update_window(win, |_, w, cx| {
        form.update(cx, |f, cx| f.set_folder(&ItemPath::root(), w, cx))
    })
    .unwrap();
    let input = form.read_with(cx, |f, _| f.name_input().clone());
    cx.update_window(win, |_, w, cx| {
        input.update(cx, |i, cx| i.set_value("  List users ", w, cx))
    })
    .unwrap();
    let answer = form.read_with(cx, |f, cx| f.answer(cx)).expect("a name");
    assert_eq!(
        (answer.name.as_str(), answer.parent),
        ("List users", ItemPath::root())
    );
    cx.update_window(win, |_, w, cx| {
        input.update(cx, |i, cx| i.set_value("   ", w, cx))
    })
    .unwrap();
    assert!(
        form.read_with(cx, |f, cx| f.answer(cx)).is_none(),
        "a blank name cannot be saved"
    );
}

#[gpui_kit::test]
fn ctrl_s_on_a_draft_asks_then_the_same_tab_shows_the_file(cx: &mut TestAppContext) {
    let col = tree();
    let s = started(cx, &col);
    focus_shell(cx, &s);
    cx.simulate_keystrokes(s.win, &key("n"));
    frame(cx, s.win);
    let (tab, draft) = active(cx, &s);
    let edit = Command::Edit {
        target: Target::Request(ip(&draft)),
        edit: Edit::Url("http://x/list".into()),
    };
    s.ws.update(cx, |w, cx| w.send(&s.root, edit, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes(s.win, &key("s"));
    frame(cx, s.win);
    assert!(present(cx, s.win, "save-draft-dialog"));
    cx.update_window(s.win, |_, w, cx| {
        w.input("List users", cx);
        w.press("enter", cx);
    })
    .unwrap();
    wait_for(cx, "the save", |_| {
        col.path().join("List users.yml").exists()
    });
    frame(cx, s.win);
    assert!(!present(cx, s.win, "save-draft-dialog"));
    let disk = std::fs::read_to_string(col.path().join("List users.yml")).unwrap();
    assert!(disk.contains("http://x/list"), "{disk}");
    assert_eq!(
        active(cx, &s),
        (tab, "List users.yml".to_owned()),
        "same tab, now the file"
    );
    let snap = ws_snap(cx, &s.ws, &s.root);
    assert!(!snap.content.requests[&ip("List users.yml")].dirty);
    assert!(
        snap.content
            .rows
            .iter()
            .any(|r| r.path == ip("List users.yml"))
    );
    assert!(!snap.content.requests.contains_key(&ip(&draft)));
}

#[gpui_kit::test]
fn escape_keeps_the_draft_and_the_default_folder_is_its_own(cx: &mut TestAppContext) {
    let col = tree();
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| w.new_draft(&s.root, ip("users"), cx))
        .expect("open");
    cx.run_until_parked();
    let (_, draft) = active(cx, &s);
    focus_shell(cx, &s);
    cx.simulate_keystrokes(s.win, &key("s"));
    frame(cx, s.win);
    assert!(present(cx, s.win, "save-draft-dialog"));
    cx.update_window(s.win, |_, w, cx| w.press("escape", cx))
        .unwrap();
    frame(cx, s.win);
    assert!(!present(cx, s.win, "save-draft-dialog"));
    assert_eq!(active(cx, &s).1, draft, "the draft stays open");
    cx.simulate_keystrokes(s.win, &key("s"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    wait_for(cx, "the save", |_| {
        col.path().join("users/Untitled request.yml").exists()
    });
}

#[gpui_kit::test]
fn the_folder_picker_is_as_wide_as_the_name_field(cx: &mut TestAppContext) {
    let col = tree();
    let s = started(cx, &col);
    focus_shell(cx, &s);
    cx.simulate_keystrokes(s.win, &key("n"));
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, &key("s"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let name = w.find("save-draft-name").bounds();
        // The picker's drawn box is its inner input, which can outgrow the picker itself.
        let folder = w.within("save-draft-folder").find("input").bounds();
        let dialog = w.find("save-draft-dialog").bounds();
        assert_eq!(folder.left(), name.left());
        assert_eq!(folder.right(), name.right(), "{folder:?} vs {name:?}");
        assert!(folder.right() <= dialog.right(), "{folder:?} in {dialog:?}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn enter_on_the_focused_folder_select_opens_it_without_saving(cx: &mut TestAppContext) {
    let col = tree();
    let s = started(cx, &col);
    focus_shell(cx, &s);
    cx.simulate_keystrokes(s.win, &key("n"));
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, &key("s"));
    frame(cx, s.win);
    enter_on(cx, s.win, "save-draft-folder");
    frame(cx, s.win);
    assert!(
        present(cx, s.win, "save-draft-dialog"),
        "the dialog stays open"
    );
    let open = cx
        .update_window(s.win, |_, w, _| w.find("save-draft-folder").expanded())
        .unwrap();
    assert_eq!(open, Some(true), "Enter opens the folders");
    assert!(
        !col.path().join("Untitled request.yml").exists(),
        "nothing saved"
    );
}
