//! The Changes dialog's commit flow: the Commit button's rules, success, a failing hook, a
//! missing identity and the outside-staged refusal.

use crate::git_support;
use crate::h;
use crate::support;

use apinata_app::changes_dialog::Notice;
use git_support::{Sandbox, write};
use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext, px, size};
use h::*;
use support::*;

fn set_message(cx: &mut TestAppContext, s: &Started, text: &str) {
    cx.update_window(s.win, |_, w, cx| {
        apinata_app::changes_dialog::current(cx)
            .expect("the dialog is open")
            .update(cx, |v, cx| v.set_message(text, w, cx))
    })
    .unwrap();
}

fn can_commit(cx: &mut TestAppContext) -> bool {
    view(cx).read_with(cx, |v, cx| v.can_commit(cx))
}

fn notice(cx: &mut TestAppContext) -> Option<Notice> {
    view(cx).read_with(cx, |v, _| v.notice().cloned())
}

fn commits(sb: &Sandbox, root: &std::path::Path) -> String {
    sb.git(root, &["rev-list", "--count", "HEAD"])
        .trim()
        .to_owned()
}

#[gpui_kit::test]
fn commit_needs_a_staged_file_and_a_message(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    assert!(!can_commit(cx));
    click(cx, &s, ("changes-stage", 0usize));
    idle(cx, &s);
    assert!(!can_commit(cx), "a staged file but no message");
    set_message(cx, &s, "  \n ");
    assert!(!can_commit(cx), "a blank message");
    set_message(cx, &s, "Edit a");
    assert!(can_commit(cx));
    click(cx, &s, "changes-commit");
    idle(cx, &s);
    assert_eq!(commits(&sb, col.path()), "2");
}

#[gpui_kit::test]
fn a_blank_message_click_commits_nothing(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    click(cx, &s, "changes-toggle-all");
    idle(cx, &s);
    click(cx, &s, "changes-commit");
    idle(cx, &s);
    assert_eq!(commits(&sb, col.path()), "1");
}

#[gpui_kit::test]
fn a_commit_clears_the_message_and_reports_the_short_id(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    click(cx, &s, "changes-toggle-all");
    idle(cx, &s);
    set_message(cx, &s, "Edit a and add n");
    click(cx, &s, "changes-commit");
    idle(cx, &s);
    assert_eq!(
        sb.git(col.path(), &["log", "-1", "--format=%s"]).trim(),
        "Edit a and add n"
    );
    let id = sb.short_head(col.path());
    assert_eq!(notice(cx), Some(Notice::Committed(id.clone())));
    assert_eq!(
        label(cx, &s, "changes-notice"),
        Some(format!("Committed {id}"))
    );
    assert_eq!(view(cx).read_with(cx, |v, cx| v.message(cx)), "");
    wait_for(cx, "an empty list", |cx| {
        view(cx).read_with(cx, |v, _| v.files().is_empty())
    });
    assert!(present(cx, s.win, "changes-empty"));
    assert!(
        cx.update(|cx| apinata_app::changes_dialog::current(cx))
            .is_some(),
        "the dialog stays open"
    );
    assert!(present(cx, s.win, "changes-push-note"));
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_failing_hook_shows_its_output_and_keeps_the_message(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let hook = col.path().join(".git/hooks/pre-commit");
    h::hook(
        col.path(),
        "pre-commit",
        "#!/bin/sh\necho 'hook says no' >&2\nexit 1\n",
    );
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    click(cx, &s, "changes-toggle-all");
    idle(cx, &s);
    set_message(cx, &s, "Edit a");
    click(cx, &s, "changes-commit");
    idle(cx, &s);
    match notice(cx) {
        Some(Notice::Failed(text)) => assert!(text.contains("hook says no"), "{text}"),
        other => panic!("expected a failure, got {other:?}"),
    }
    assert_eq!(view(cx).read_with(cx, |v, cx| v.message(cx)), "Edit a");
    assert_eq!(commits(&sb, col.path()), "1");
    std::fs::remove_file(&hook).unwrap();
    click(cx, &s, "changes-commit");
    idle(cx, &s);
    assert!(matches!(notice(cx), Some(Notice::Committed(_))));
    assert_eq!(commits(&sb, col.path()), "2");
}

#[gpui_kit::test]
fn a_missing_identity_shows_gits_text_with_the_hint(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = started(cx, &col);
    let options = h::without_identity(&sb);
    s.ws.update(cx, |w, cx| w.set_git(Some(options), cx));
    frame(cx, s.win);
    open_on(cx, &s, col.path());
    click(cx, &s, "changes-toggle-all");
    idle(cx, &s);
    set_message(cx, &s, "Edit a");
    click(cx, &s, "changes-commit");
    idle(cx, &s);
    match notice(cx) {
        Some(Notice::Failed(text)) => {
            assert!(text.contains("identity"), "{text}");
            assert!(text.contains("git config user.name"), "{text}");
        }
        other => panic!("expected a failure, got {other:?}"),
    }
}

#[gpui_kit::test]
fn staged_files_outside_a_nested_collection_are_named(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let repo = committed(
        &sb,
        &[
            ("col/opencollection.yml", ROOT),
            ("col/a.yml", &request("a", "http://x/a")),
            ("other.txt", "one\n"),
        ],
    );
    let sub = repo.path().join("col");
    write(&sub, "a.yml", &request("a", "http://x/changed"));
    write(repo.path(), "other.txt", "two\n");
    sb.git(repo.path(), &["add", "other.txt"]);
    let host = collection_dir();
    let s = git_started(cx, &sb, &host);
    s.ws.update(cx, |w, cx| w.open(sub.clone(), cx));
    wait_for(cx, "the nested collection's status", |cx| {
        s.ws.read_with(cx, |w, _| w.git_changes(&sub)).is_some()
    });
    open_on(cx, &s, &sub);
    click(cx, &s, "changes-toggle-all");
    idle(cx, &s);
    set_message(cx, &s, "Edit a");
    click(cx, &s, "changes-commit");
    idle(cx, &s);
    match notice(cx) {
        Some(Notice::Failed(text)) => assert!(text.contains("other.txt"), "{text}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn collection_dir() -> tempfile::TempDir {
    write_tree(&[("opencollection.yml", ROOT)])
}

fn focus_message(cx: &mut TestAppContext, s: &Started) {
    cx.update_window(s.win, |_, w, cx| {
        apinata_app::changes_dialog::current(cx)
            .unwrap()
            .update(cx, |v, cx| v.focus_message(w, cx))
    })
    .unwrap();
}

#[gpui_kit::test]
fn ctrl_enter_in_the_message_commits_without_a_newline(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    click(cx, &s, "changes-toggle-all");
    idle(cx, &s);
    focus_message(cx, &s);
    cx.simulate_input(s.win, "Edit a");
    cx.simulate_keystrokes(s.win, &secondary("enter"));
    idle(cx, &s);
    assert_eq!(
        sb.git(col.path(), &["log", "-1", "--format=%B"]).trim_end(),
        "Edit a"
    );
    assert_eq!(view(cx).read_with(cx, |v, cx| v.message(cx)), "");
}

/// A busy dialog rejects typing into the focused message in every theme.
#[gpui_kit::test]
fn typing_into_the_message_while_busy_changes_nothing(cx: &mut TestAppContext) {
    use apinata_app::theme::{DARK, HC_DARK, HC_LIGHT, LIGHT};
    for theme in [HC_LIGHT, HC_DARK, LIGHT, DARK] {
        let sb = Sandbox::new();
        let col = changed(&sb);
        let s = git_started(cx, &sb, &col);
        crate::support::use_theme(cx, theme);
        open_on(cx, &s, col.path());
        focus_message(cx, &s);
        view(cx).update(cx, |v, cx| {
            v.toggle(0, cx);
            assert!(v.busy());
        });
        cx.update_window(s.win, |_, w, cx| {
            use gpui_kit::test::TestWindowExt as _;
            w.render_frame(cx)
        })
        .unwrap();
        cx.simulate_input(s.win, "x");
        assert_eq!(view(cx).read_with(cx, |v, cx| v.message(cx)), "", "{theme}");
        idle(cx, &s);
    }
}

#[gpui_kit::test]
fn plain_enter_adds_a_newline_and_never_commits(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    click(cx, &s, "changes-toggle-all");
    idle(cx, &s);
    focus_message(cx, &s);
    cx.simulate_input(s.win, "Edit a");
    cx.simulate_keystrokes(s.win, "enter");
    cx.run_until_parked();
    assert_eq!(view(cx).read_with(cx, |v, cx| v.message(cx)), "Edit a\n");
    assert_eq!(commits(&sb, col.path()), "1");
}

#[gpui_kit::test]
fn ctrl_enter_with_nothing_staged_changes_nothing(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    focus_message(cx, &s);
    cx.simulate_input(s.win, "Edit a");
    cx.simulate_keystrokes(s.win, &secondary("enter"));
    cx.run_until_parked();
    assert_eq!(view(cx).read_with(cx, |v, cx| v.message(cx)), "Edit a");
    assert_eq!(commits(&sb, col.path()), "1");
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_long_failure_scrolls_and_the_footer_stays_inside_a_1000_by_500_window(
    cx: &mut TestAppContext,
) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let lines = (0..60)
        .map(|i| format!("echo 'line {i}' >&2\n"))
        .collect::<String>();
    h::hook(
        col.path(),
        "pre-commit",
        &format!("#!/bin/sh\n{lines}exit 1\n"),
    );
    let s = git_started(cx, &sb, &col);
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1000.), px(500.)));
    open_on(cx, &s, col.path());
    click(cx, &s, "changes-toggle-all");
    idle(cx, &s);
    set_message(cx, &s, "Edit a");
    click(cx, &s, "changes-commit");
    idle(cx, &s);
    frame(cx, s.win);
    use gpui_kit::test::TestWindowExt as _;
    cx.update_window(s.win, |_, w, _| {
        for id in [
            "changes-commit",
            "changes-close",
            "changes-notice",
            "changes-message",
        ] {
            let b = w.find(id).bounds();
            assert!(
                b.bottom() <= px(500.) && b.right() <= px(1000.),
                "{id} at {b:?}"
            );
        }
    })
    .unwrap();
}

/// While a stage or commit runs the message is locked like a locked input: read-only and
/// never disabled in every theme, so it keeps focus and its text still selects.
#[gpui_kit::test]
fn the_message_is_locked_read_only_in_every_theme(cx: &mut TestAppContext) {
    use apinata_app::theme::{DARK, HC_DARK, HC_LIGHT, LIGHT};
    for theme in [HC_LIGHT, HC_DARK, LIGHT, DARK] {
        let sb = Sandbox::new();
        let col = changed(&sb);
        let s = git_started(cx, &sb, &col);
        crate::support::use_theme(cx, theme);
        open_on(cx, &s, col.path());
        let lock = |cx: &mut TestAppContext| view(cx).read_with(cx, |v, cx| v.message_lock(cx));
        assert_eq!(lock(cx), (false, false), "{theme}: idle");
        view(cx).update(cx, |v, cx| {
            v.toggle(0, cx);
            assert!(v.busy());
        });
        // A bare render, so the work cannot finish before the frame is drawn.
        cx.update_window(s.win, |_, w, cx| {
            use gpui_kit::test::TestWindowExt as _;
            w.render_frame(cx)
        })
        .unwrap();
        assert_eq!(lock(cx), (true, false), "{theme}: busy");
        idle(cx, &s);
    }
}
