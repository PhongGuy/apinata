//! A collection's `.env` is kept out of git when it opens in a repository: a missing rule is
//! added once with a notice, and a tracked `.env` gets a notice saying how to untrack it.

use crate::git_support;
use crate::h;
use crate::support;

use apinata_app::workspace::WorkspaceEvent;
use git_support::{Sandbox, write};
use gpui_kit::TestAppContext;
use h::*;
use support::*;

// The dotenv notices `seen` holds, in order.
fn notices(seen: &std::rc::Rc<std::cell::RefCell<Vec<WorkspaceEvent>>>) -> Vec<String> {
    (seen.borrow().iter())
        .filter_map(|e| match e {
            WorkspaceEvent::DotenvNotice { message, .. } => Some(message.clone()),
            _ => None,
        })
        .collect()
}

fn listed(cx: &mut TestAppContext, s: &Started, col: &tempfile::TempDir) -> Vec<String> {
    let changes = s.ws.read_with(cx, |w, _| w.git_changes(col.path()));
    (changes.expect("git state").files.keys())
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

#[gpui_kit::test]
fn an_unignored_dotenv_gains_a_rule_once_with_a_notice(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = committed(&sb, &[("opencollection.yml", ROOT)]);
    write(col.path(), ".env", "API_KEY=abc\n");
    let s = started(cx, &col);
    let seen = events(cx, &s.ws);
    let options = sb.options();
    s.ws.update(cx, |w, cx| w.set_git(Some(options), cx));
    wait_for(cx, "the notice", |_| !notices(&seen).is_empty());
    frame(cx, s.win);
    let gitignore = std::fs::read_to_string(col.path().join(".gitignore")).unwrap();
    assert_eq!(gitignore, ".env\n");
    assert_eq!(notices(&seen).len(), 1);
    assert!(notices(&seen)[0].contains("Added .env to .gitignore"));
    assert!(present(cx, s.win, "toast-warning"), "the notice is shown");
    wait_for(cx, "the status", |cx| {
        s.ws.read_with(cx, |w, _| w.git_changes(col.path()))
            .is_some()
    });
    assert_eq!(listed(cx, &s, &col), [".gitignore"], ".env is not offered");

    // A rule taken out by hand stays out for the rest of this open.
    std::fs::remove_file(col.path().join(".gitignore")).unwrap();
    let runs = s.ws.read_with(cx, |w, _| w.git_runs(col.path()));
    s.ws.update(cx, |w, cx| w.refresh_git(col.path(), cx));
    wait_for(cx, "another run", |cx| {
        s.ws.read_with(cx, |w, _| w.git_runs(col.path())) > runs
    });
    assert!(!col.path().join(".gitignore").exists());
    assert_eq!(notices(&seen).len(), 1);
}

#[gpui_kit::test]
fn a_tracked_dotenv_is_left_alone_and_the_notice_says_how_to_untrack(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = committed(
        &sb,
        &[("opencollection.yml", ROOT), (".env", "API_KEY=abc\n")],
    );
    let s = started(cx, &col);
    let seen = events(cx, &s.ws);
    let options = sb.options();
    s.ws.update(cx, |w, cx| w.set_git(Some(options), cx));
    wait_for(cx, "the notice", |_| !notices(&seen).is_empty());
    assert!(!col.path().join(".gitignore").exists());
    let text = &notices(&seen)[0];
    assert!(
        text.contains("tracked") && text.contains("git rm --cached .env"),
        "{text}"
    );
}

#[gpui_kit::test]
fn without_a_dotenv_or_a_repository_nothing_is_written(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = committed(&sb, &[("opencollection.yml", ROOT)]);
    let plain = write_tree(&[("opencollection.yml", ROOT), (".env", "API_KEY=abc\n")]);
    for dir in [&col, &plain] {
        let s = started(cx, dir);
        let seen = events(cx, &s.ws);
        let options = sb.options();
        s.ws.update(cx, |w, cx| w.set_git(Some(options), cx));
        wait_for(cx, "a status run", |cx| {
            s.ws.read_with(cx, |w, _| w.git_runs(dir.path())) > 0
        });
        assert!(!dir.path().join(".gitignore").exists());
        assert!(notices(&seen).is_empty());
    }
}
