//! The Changes dialog's identity form: a commit without a git name and email asks for them,
//! saves them at the chosen scope and commits the kept message. Every git call reads temp
//! config files only.

use crate::git_support;
use crate::h;
use crate::support;

use std::path::{Path, PathBuf};

use apinata_app::changes_dialog::Notice;
use git_support::Sandbox;
use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext, px, size};
use h::*;
use support::*;

const ASK: &str = "Git needs your name and email before it can commit";
const SAVE_FAILED: &str = "Git could not save your name and email. Check that its config file can be written, then try again.";

fn global_file(sb: &Sandbox) -> PathBuf {
    sb.path().join("global.gitconfig")
}

/// A dialog on a changed collection whose git has no identity anywhere, with everything staged
/// and "Edit a" typed, after Commit was clicked once.
fn asked(cx: &mut TestAppContext, sb: &Sandbox) -> (Started, tempfile::TempDir) {
    let col = changed(sb);
    std::fs::write(global_file(sb), "").unwrap();
    let options = h::without_identity(sb).env("GIT_CONFIG_GLOBAL", global_file(sb));
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| w.set_git(Some(options), cx));
    frame(cx, s.win);
    open_on(cx, &s, col.path());
    click(cx, &s, "changes-toggle-all");
    idle(cx, &s);
    cx.update_window(s.win, |_, w, cx| {
        apinata_app::changes_dialog::current(cx)
            .expect("the dialog is open")
            .update(cx, |v, cx| v.set_message("Edit a", w, cx))
    })
    .unwrap();
    click(cx, &s, "changes-commit");
    idle(cx, &s);
    (s, col)
}

fn notice(cx: &mut TestAppContext) -> Option<Notice> {
    view(cx).read_with(cx, |v, _| v.notice().cloned())
}

fn commits(sb: &Sandbox, root: &Path) -> String {
    sb.git(root, &["rev-list", "--count", "HEAD"])
        .trim()
        .to_owned()
}

fn fill(cx: &mut TestAppContext, s: &Started, name: &str, email: &str) {
    replace_input(cx, s.win, None, "changes-identity-name", name);
    replace_input(cx, s.win, None, "changes-identity-email", email);
    frame(cx, s.win);
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

#[gpui_kit::test]
fn a_missing_identity_asks_in_plain_words_and_keeps_the_message(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (s, col) = asked(cx, &sb);
    assert_eq!(notice(cx), Some(Notice::NeedsIdentity { error: None }));
    assert_eq!(label(cx, &s, "changes-identity-note"), Some(ASK.to_owned()));
    // Git's own text never reaches the dialog.
    assert_eq!(label(cx, &s, "changes-notice"), None);
    assert!(present(cx, s.win, "changes-identity-name"));
    assert!(present(cx, s.win, "changes-identity-email"));
    assert!(present(cx, s.win, "changes-identity-repo"));
    assert!(present(cx, s.win, "changes-identity-all"));
    assert_eq!(view(cx).read_with(cx, |v, cx| v.message(cx)), "Edit a");
    assert_eq!(commits(&sb, col.path()), "1");
}

#[gpui_kit::test]
fn save_writes_this_repository_and_commits_the_kept_message(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (s, col) = asked(cx, &sb);
    fill(cx, &s, "Ann Example", "ann@example.com");
    click(cx, &s, "changes-identity-save");
    idle(cx, &s);
    assert!(
        matches!(notice(cx), Some(Notice::Committed(_))),
        "{:?}",
        notice(cx)
    );
    assert_eq!(
        sb.git(col.path(), &["log", "-1", "--format=%an <%ae> %s"])
            .trim(),
        "Ann Example <ann@example.com> Edit a"
    );
    assert!(read(&col.path().join(".git/config")).contains("name = Ann Example"));
    assert_eq!(read(&global_file(&sb)), "");
}

#[gpui_kit::test]
fn all_repositories_writes_the_global_config_only(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (s, col) = asked(cx, &sb);
    fill(cx, &s, "Ann Example", "ann@example.com");
    click(cx, &s, "changes-identity-all");
    frame(cx, s.win);
    click(cx, &s, "changes-identity-save");
    idle(cx, &s);
    assert!(matches!(notice(cx), Some(Notice::Committed(_))));
    assert!(read(&global_file(&sb)).contains("email = ann@example.com"));
    assert!(!read(&col.path().join(".git/config")).contains("[user]"));
    assert_eq!(commits(&sb, col.path()), "2");
}

#[gpui_kit::test]
fn an_unsafe_name_is_refused_in_the_form_and_nothing_is_written(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (s, col) = asked(cx, &sb);
    fill(cx, &s, "-x", "ann@example.com");
    click(cx, &s, "changes-identity-save");
    idle(cx, &s);
    match notice(cx) {
        Some(Notice::NeedsIdentity { error: Some(text) }) => {
            assert!(text.contains("name"), "{text}")
        }
        other => panic!("expected the form with an error, got {other:?}"),
    }
    assert!(label(cx, &s, "changes-identity-error").is_some());
    assert!(!read(&col.path().join(".git/config")).contains("[user]"));
    assert_eq!(read(&global_file(&sb)), "");
    assert_eq!(commits(&sb, col.path()), "1");
    assert_eq!(view(cx).read_with(cx, |v, cx| v.message(cx)), "Edit a");
}

#[gpui_kit::test]
fn save_waits_for_both_fields(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (s, col) = asked(cx, &sb);
    fill(cx, &s, "Ann Example", "  ");
    click(cx, &s, "changes-identity-save");
    frame(cx, s.win);
    assert_eq!(notice(cx), Some(Notice::NeedsIdentity { error: None }));
    assert_eq!(commits(&sb, col.path()), "1");
}

#[gpui_kit::test]
fn the_form_and_the_footer_fit_a_1000_by_500_window(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (s, _col) = asked(cx, &sb);
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1000.), px(500.)));
    frame(cx, s.win);
    use gpui_kit::test::TestWindowExt as _;
    cx.update_window(s.win, |_, w, _| {
        for id in [
            "changes-commit",
            "changes-close",
            "changes-identity-save",
            "changes-identity-name",
            "changes-identity-email",
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

/// A git that refuses any call naming `user.email` and runs the real git otherwise.
#[cfg(unix)]
fn email_refused(sb: &Sandbox) -> std::ffi::OsString {
    let body = format!(
        "#!/bin/sh\ncase \"$*\" in *user.email*) echo 'error: could not lock config file' >&2; exit 255 ;; esac\nexec '{}' \"$@\"\n",
        git_support::real_git().display()
    );
    git_support::path_with(&sb.fake_git("no-email", &body))
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_failed_save_says_so_plainly_and_writes_no_name(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (s, col) = asked(cx, &sb);
    let options = h::without_identity(&sb)
        .env("GIT_CONFIG_GLOBAL", global_file(&sb))
        .env("PATH", email_refused(&sb));
    s.ws.update(cx, |w, cx| w.set_git(Some(options), cx));
    frame(cx, s.win);
    fill(cx, &s, "Ann Example", "ann@example.com");
    click(cx, &s, "changes-identity-save");
    idle(cx, &s);
    match notice(cx) {
        Some(Notice::NeedsIdentity { error: Some(text) }) => {
            assert_eq!(text, SAVE_FAILED);
        }
        other => panic!("expected the form with an error, got {other:?}"),
    }
    assert!(!read(&col.path().join(".git/config")).contains("[user]"));
    assert_eq!(commits(&sb, col.path()), "1");
}
