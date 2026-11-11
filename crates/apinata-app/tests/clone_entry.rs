//! Every way into the clone dialog: palette action, switcher row, welcome screen, app menu.

use crate::support::*;

use apinata_app::app_menu::app_menu_items;
use apinata_app::clone_dialog;
use apinata_app::commands::registry;
use apinata_app::palette::palette_items;
use apinata_app::shell::CloneRepository;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Focusable as _, TestAppContext};

fn dialog_open(cx: &mut TestAppContext, s: &Started) -> bool {
    cx.update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap()
}

#[test]
fn clone_is_in_the_palette_and_the_app_menu() {
    let cmd = registry()
        .into_iter()
        .find(|c| c.id == "collection.clone")
        .expect("registered");
    assert_eq!(cmd.label, "Clone repository…");
    assert!(cmd.in_palette && cmd.default_chords.is_empty());
    assert!(
        palette_items()
            .iter()
            .any(|e| e.label == "Clone repository…")
    );
    assert!(
        app_menu_items()
            .iter()
            .any(|(label, _)| *label == "Clone repository…")
    );
}

#[gpui_kit::test]
fn the_action_opens_the_dialog(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    let shell = s.shell.clone();
    cx.update_window(s.win, |_, w, cx| {
        shell.read(cx).focus_handle(cx).focus(w, cx);
        w.dispatch_action(Box::new(CloneRepository), cx);
    })
    .unwrap();
    frame(cx, s.win);
    assert!(dialog_open(cx, &s));
    assert!(cx.update(|cx| clone_dialog::current(cx).is_some()));
}

#[gpui_kit::test]
fn the_switcher_has_a_clone_row_after_the_other_actions(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("collection-switcher", cx)
    })
    .unwrap();
    frame(cx, s.win);
    let (new_top, clone_top) = cx
        .update_window(s.win, |_, w, _| {
            (
                w.find("switcher-new-collection").bounds().top(),
                w.find("switcher-clone-repository").bounds().top(),
            )
        })
        .unwrap();
    assert!(new_top < clone_top);
    cx.update_window(s.win, |_, w, cx| w.click("switcher-clone-repository", cx))
        .unwrap();
    frame(cx, s.win);
    assert!(dialog_open(cx, &s));
}

#[gpui_kit::test]
fn the_welcome_screen_offers_clone_last(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    let root = s.root.clone();
    s.ws.update(cx, |w, cx| w.close(&root, cx));
    frame(cx, s.win);
    let labels = cx
        .update_window(s.win, |_, w, _| {
            let scope = w.within("no-collection-actions");
            (0..5usize)
                .map(|ix| scope.find(("empty-action", ix)).label().map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap();
    assert_eq!(labels[4].as_deref(), Some("Clone repository…"));
    cx.update_window(s.win, |_, w, cx| {
        w.within("no-collection-actions")
            .click(("empty-action", 4usize), cx)
    })
    .unwrap();
    frame(cx, s.win);
    assert!(dialog_open(cx, &s));
}

fn open_clone(cx: &mut TestAppContext, s: &Started) -> gpui_kit::Entity<clone_dialog::CloneView> {
    let ws = s.ws.clone();
    let view = cx
        .update_window(s.win, |_, w, cx| clone_dialog::open(&ws, w, cx))
        .unwrap();
    frame(cx, s.win);
    view
}

fn escape(cx: &mut TestAppContext, s: &Started) {
    cx.update_window(s.win, |_, w, cx| w.press("escape", cx))
        .unwrap();
    frame(cx, s.win);
    assert!(!dialog_open(cx, s));
}

fn fields(
    cx: &mut TestAppContext,
    view: &gpui_kit::Entity<clone_dialog::CloneView>,
) -> (String, String) {
    view.read_with(cx, |v, cx| (v.url(cx), v.name(cx)))
}

#[gpui_kit::test]
fn a_typed_url_and_name_come_back_after_escape(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    open_clone(cx, &s);
    type_at_end(cx, s.win, None, "clone-url", "https://host/owner/repo.git");
    replace_input(cx, s.win, None, "clone-name", "mine");
    frame(cx, s.win);
    escape(cx, &s);
    let view = open_clone(cx, &s);
    assert_eq!(
        fields(cx, &view),
        ("https://host/owner/repo.git".to_owned(), "mine".to_owned())
    );
}

#[gpui_kit::test]
fn a_url_with_credentials_is_not_kept(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    open_clone(cx, &s);
    type_at_end(
        cx,
        s.win,
        None,
        "clone-url",
        "https://user:secret@host/owner/repo.git",
    );
    replace_input(cx, s.win, None, "clone-name", "mine");
    frame(cx, s.win);
    escape(cx, &s);
    let kept = cx.update(|cx| apinata_app::form_drafts::clone_draft(cx));
    assert!(!format!("{kept:?}").contains("secret"), "{kept:?}");
    let view = open_clone(cx, &s);
    assert_eq!(fields(cx, &view), (String::new(), "mine".to_owned()));
}

#[gpui_kit::test]
fn a_started_clone_leaves_no_draft(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    let parent = tempfile::tempdir().unwrap();
    let view = open_clone(cx, &s);
    type_at_end(cx, s.win, None, "clone-url", "/srv/x.git");
    frame(cx, s.win);
    let (tx, rx) = async_channel::unbounded();
    let (v, dir) = (view.clone(), parent.path().to_path_buf());
    cx.update_window(s.win, |_, w, cx| {
        v.update(cx, |v, cx| {
            v.set_parent(dir, cx);
            v.start_with(
                |_, _, _| apinata_app::clone_job::CloneJob::from_events(rx),
                w,
                cx,
            )
        })
    })
    .unwrap();
    frame(cx, s.win);
    assert_eq!(
        view.read_with(cx, |v, _| v.phase()),
        clone_dialog::Phase::Running
    );
    // A lost job goes back to the form, which Escape then closes.
    drop(tx);
    frame(cx, s.win);
    escape(cx, &s);
    let view = open_clone(cx, &s);
    assert_eq!(fields(cx, &view), (String::new(), String::new()));
}
