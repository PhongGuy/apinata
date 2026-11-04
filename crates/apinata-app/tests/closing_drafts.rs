use crate::support;

use apinata_app::command::{Command, Edit, Target};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Focusable, TestAppContext, VisualTestContext};
use support::*;

fn close_window(cx: &mut TestAppContext, s: &Started) -> bool {
    VisualTestContext::from_window(s.win, cx).simulate_close()
}

fn windows(cx: &mut TestAppContext) -> usize {
    cx.update(|cx| cx.windows().len())
}

fn entries(col: &tempfile::TempDir) -> usize {
    std::fs::read_dir(col.path()).unwrap().count()
}

// ctrl+n with the shell focused; returns the draft's item path.
fn new_draft(cx: &mut TestAppContext, s: &Started) -> String {
    let handle = s.shell.read_with(cx, |v, cx| v.focus_handle(cx));
    let mut v = VisualTestContext::from_window(s.win, cx);
    v.update(|w, cx| handle.focus(w, cx));
    frame(cx, s.win);
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

fn has_request(cx: &mut TestAppContext, s: &Started, rel: &str) -> bool {
    ws_snap(cx, &s.ws, &s.root)
        .content
        .requests
        .contains_key(&ip(rel))
}

#[gpui_kit::test]
fn an_untouched_draft_lets_the_window_close(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let before = entries(&col);
    new_draft(cx, &s);
    assert!(close_window(cx, &s));
    assert!(!confirm_open(cx));
    assert_eq!(entries(&col), before);
}

#[gpui_kit::test]
fn a_draft_with_content_asks_and_cancel_keeps_it(cx: &mut TestAppContext) {
    let col = two_requests();
    let (s, _drafts) = started_without_drafts(cx, &col);
    let draft = new_draft(cx, &s);
    touch(cx, &s, &draft);
    assert!(!close_window(cx, &s));
    assert!(confirm_open(cx));
    answer_confirm(cx, "confirm-cancel");
    cx.run_until_parked();
    assert_eq!(windows(cx), 1);
    assert!(has_request(cx, &s, &draft), "the draft is kept");
}

#[gpui_kit::test]
fn dont_save_closes_and_writes_nothing(cx: &mut TestAppContext) {
    let col = two_requests();
    let (s, _drafts) = started_without_drafts(cx, &col);
    let before = entries(&col);
    let draft = new_draft(cx, &s);
    touch(cx, &s, &draft);
    assert!(!close_window(cx, &s));
    answer_confirm(cx, "confirm-discard");
    cx.run_until_parked();
    assert_eq!(windows(cx), 0);
    assert_eq!(entries(&col), before);
}

#[gpui_kit::test]
fn save_closes_the_window_after_naming_the_draft(cx: &mut TestAppContext) {
    let col = two_requests();
    let (s, _drafts) = started_without_drafts(cx, &col);
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, edit_url("a.yml", "http://x/9"), cx)
    });
    cx.run_until_parked();
    let draft = new_draft(cx, &s);
    touch(cx, &s, &draft);
    assert!(!close_window(cx, &s));
    answer_confirm(cx, "confirm-save");
    frame(cx, s.win);
    assert!(present(cx, s.win, "save-draft-dialog"));
    cx.update_window(s.win, |_, w, cx| {
        w.input("Kept", cx);
        w.press("enter", cx);
    })
    .unwrap();
    wait_for(cx, "the window to close", |cx| windows(cx) == 0);
    let kept = std::fs::read_to_string(col.path().join("Kept.yml")).unwrap();
    assert!(kept.contains("http://x/typed"), "{kept}");
    let a = std::fs::read_to_string(col.path().join("a.yml")).unwrap();
    assert!(a.contains("http://x/9"), "{a}");
}

#[gpui_kit::test]
fn escape_on_the_draft_dialog_keeps_the_window(cx: &mut TestAppContext) {
    let col = two_requests();
    let (s, _drafts) = started_without_drafts(cx, &col);
    let before = entries(&col);
    let draft = new_draft(cx, &s);
    touch(cx, &s, &draft);
    assert!(!close_window(cx, &s));
    answer_confirm(cx, "confirm-save");
    frame(cx, s.win);
    assert!(present(cx, s.win, "save-draft-dialog"));
    cx.update_window(s.win, |_, w, cx| w.press("escape", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(windows(cx), 1);
    assert!(has_request(cx, &s, &draft));
    assert_eq!(entries(&col), before);
}
