use crate::support;

use apinata_app::command::{Command, Edit, Target};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext, Focusable, TestAppContext, VisualTestContext};
use support::*;

fn focus_shell(cx: &mut TestAppContext, s: &Started) {
    let handle = s.shell.read_with(cx, |v, cx| v.focus_handle(cx));
    let mut v = VisualTestContext::from_window(s.win, cx);
    v.update(|w, cx| handle.focus(w, cx));
    frame(cx, s.win);
}

fn new_draft_by_key(cx: &mut TestAppContext, s: &Started) -> String {
    cx.simulate_keystrokes(s.win, &key("n"));
    frame(cx, s.win);
    s.ws.read_with(cx, |w, _| {
        w.tabs()
            .active()
            .and_then(|e| e.item())
            .expect("a draft")
            .as_str()
            .to_owned()
    })
}

fn touch(cx: &mut TestAppContext, s: &Started, draft: &str) {
    let edit = Command::Edit {
        target: Target::Request(ip(draft)),
        edit: Edit::Url("http://x/typed".into()),
    };
    s.ws.update(cx, |w, cx| w.send(&s.root, edit, cx));
    cx.run_until_parked();
}

fn requests(cx: &mut TestAppContext, s: &Started) -> usize {
    ws_snap(cx, &s.ws, &s.root).content.requests.len()
}

fn yml_files(col: &tempfile::TempDir) -> usize {
    std::fs::read_dir(col.path()).unwrap().count()
}

#[gpui_kit::test]
fn an_untouched_draft_closes_without_asking(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let (before, files) = (requests(cx, &s), yml_files(&col));
    focus_shell(cx, &s);
    new_draft_by_key(cx, &s);
    cx.simulate_keystrokes(s.win, &key("w"));
    frame(cx, s.win);
    assert!(!confirm_open(cx));
    assert!(tab_names(cx, &s.ws).is_empty());
    assert_eq!(requests(cx, &s), before, "the draft is gone");
    assert_eq!(yml_files(&col), files);
}

#[gpui_kit::test]
fn a_dirty_draft_asks_and_dont_save_leaves_nothing(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let (before, files) = (requests(cx, &s), yml_files(&col));
    focus_shell(cx, &s);
    let draft = new_draft_by_key(cx, &s);
    touch(cx, &s, &draft);
    cx.simulate_keystrokes(s.win, &key("w"));
    answer_confirm(cx, "confirm-cancel");
    assert_eq!(tab_names(cx, &s.ws), [draft]);
    cx.simulate_keystrokes(s.win, &key("w"));
    answer_confirm(cx, "confirm-discard");
    frame(cx, s.win);
    assert!(tab_names(cx, &s.ws).is_empty());
    assert_eq!(requests(cx, &s), before);
    assert_eq!(yml_files(&col), files, "nothing on disk");
}

#[gpui_kit::test]
fn save_from_the_close_prompt_names_the_file_then_closes(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    focus_shell(cx, &s);
    let draft = new_draft_by_key(cx, &s);
    touch(cx, &s, &draft);
    cx.simulate_keystrokes(s.win, &key("w"));
    answer_confirm(cx, "confirm-save");
    frame(cx, s.win);
    assert!(present(cx, s.win, "save-draft-dialog"));
    cx.update_window(s.win, |_, w, cx| w.press("escape", cx))
        .unwrap();
    frame(cx, s.win);
    assert_eq!(
        tab_names(cx, &s.ws),
        [draft],
        "a cancelled save keeps the tab"
    );
    cx.simulate_keystrokes(s.win, &key("w"));
    answer_confirm(cx, "confirm-save");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.input("Kept", cx);
        w.press("enter", cx);
    })
    .unwrap();
    wait_for(cx, "the tab to close", |cx| tab_names(cx, &s.ws).is_empty());
    let disk = std::fs::read_to_string(col.path().join("Kept.yml")).unwrap();
    assert!(disk.contains("http://x/typed"), "{disk}");
}

#[gpui_kit::test]
fn repeated_ctrl_w_closes_files_and_untouched_drafts(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let before = requests(cx, &s);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    focus_shell(cx, &s);
    new_draft_by_key(cx, &s);
    new_draft_by_key(cx, &s);
    assert_eq!(tab_names(cx, &s.ws).len(), 3);
    for left in [2, 1, 0] {
        cx.simulate_keystrokes(s.win, &key("w"));
        frame(cx, s.win);
        assert!(!confirm_open(cx));
        assert_eq!(tab_names(cx, &s.ws).len(), left);
    }
    assert_eq!(requests(cx, &s), before);
}

fn active_item(cx: &mut TestAppContext, s: &Started) -> String {
    s.ws.read_with(cx, |w, _| {
        w.tabs()
            .active()
            .and_then(|e| e.item())
            .expect("an active request")
            .as_str()
            .to_owned()
    })
}

#[gpui_kit::test]
fn each_save_dialog_shows_the_draft_it_names(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    focus_shell(cx, &s);
    let first = new_draft_by_key(cx, &s);
    touch(cx, &s, &first);
    let second = new_draft_by_key(cx, &s);
    touch(cx, &s, &second);
    cx.simulate_keystrokes(s.win, &key("shift-w"));
    answer_confirm(cx, "confirm-save");
    frame(cx, s.win);
    assert!(present(cx, s.win, "save-draft-dialog"));
    assert_eq!(active_item(cx, &s), first);
    cx.update_window(s.win, |_, w, cx| w.press("escape", cx))
        .unwrap();
    frame(cx, s.win);
    assert!(present(cx, s.win, "save-draft-dialog"));
    assert_eq!(active_item(cx, &s), second);
    cx.update_window(s.win, |_, w, cx| {
        w.input("Second", cx);
        w.press("enter", cx);
    })
    .unwrap();
    wait_for(cx, "the second tab to close", |cx| {
        tab_names(cx, &s.ws).len() == 1
    });
    assert_eq!(tab_names(cx, &s.ws), [first]);
    let disk = std::fs::read_to_string(col.path().join("Second.yml")).unwrap();
    assert!(disk.contains("http://x/typed"), "{disk}");
}

#[gpui_kit::test]
fn ctrl_n_works_after_dont_save_closed_the_last_draft(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    focus_shell(cx, &s);
    new_draft_by_key(cx, &s);
    cx.update_window(s.win, |_, w, cx| w.input("http://x/1", cx))
        .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, &key("w"));
    answer_confirm(cx, "confirm-discard");
    frame(cx, s.win);
    assert!(tab_names(cx, &s.ws).is_empty());
    cx.simulate_keystrokes(s.win, &key("n"));
    frame(cx, s.win);
    assert_eq!(tab_names(cx, &s.ws).len(), 1);
}

// The platform may handle a prompt's answer before it draws the frame in which the dialog
// handed focus back to the closing tab, so the click and the close land in one update here,
// with raw events: the harness's click draws a frame after the release.
fn discard_and_close_before_a_frame(cx: &mut TestAppContext, s: &Started) {
    use gpui_kit::{InputEvent as _, MouseButton, MouseDownEvent, MouseUpEvent};
    cx.simulate_keystrokes(s.win, &key("w"));
    let win = confirm_window(cx).expect("a confirm dialog");
    let at = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find("confirm-discard").bounds().center()
        })
        .unwrap();
    let ws = s.ws.clone();
    cx.update_window(win, |_, w, cx| {
        let down = MouseDownEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Default::default(),
            click_count: 1,
            first_mouse: false,
        };
        let up = MouseUpEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Default::default(),
            click_count: 1,
        };
        w.dispatch_event(down.to_platform_input(), cx);
        w.dispatch_event(up.to_platform_input(), cx);
        let id = ws.read(cx).tabs().active().expect("a tab").id;
        ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.close(id)));
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    assert!(tab_names(cx, &s.ws).is_empty());
}

#[gpui_kit::test]
fn ctrl_n_works_when_a_draft_closes_before_the_next_frame(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    focus_shell(cx, &s);
    new_draft_by_key(cx, &s);
    cx.update_window(s.win, |_, w, cx| w.input("http://x/1", cx))
        .unwrap();
    frame(cx, s.win);
    discard_and_close_before_a_frame(cx, &s);
    cx.simulate_keystrokes(s.win, &key("n"));
    frame(cx, s.win);
    assert_eq!(tab_names(cx, &s.ws).len(), 1, "ctrl+n opens a draft");
}

#[gpui_kit::test]
fn ctrl_n_works_when_a_file_tab_closes_before_the_next_frame(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    type_at_end(cx, s.win, None, "url", "/typed");
    wait_for(cx, "the typed url to make the tab dirty", |cx| {
        view_of(cx, &s.ws, &s.root, "a.yml").dirty
    });
    discard_and_close_before_a_frame(cx, &s);
    cx.simulate_keystrokes(s.win, &key("n"));
    frame(cx, s.win);
    assert_eq!(tab_names(cx, &s.ws).len(), 1, "ctrl+n opens a draft");
}

#[gpui_kit::test]
fn the_close_prompt_says_dont_save_in_sentence_case(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    focus_shell(cx, &s);
    let draft = new_draft_by_key(cx, &s);
    touch(cx, &s, &draft);
    cx.simulate_keystrokes(s.win, &key("w"));
    let win = confirm_window(cx).expect("the save prompt");
    let label = cx
        .update_window(win, |_, w, _| {
            w.find("confirm-discard").label().map(str::to_owned)
        })
        .unwrap();
    assert_eq!(label.as_deref(), Some("Don't save"));
    answer_confirm(cx, "confirm-cancel");
}
