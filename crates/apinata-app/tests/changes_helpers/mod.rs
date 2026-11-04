//! Fixtures shared by the Changes dialog's stage and commit tests. A test file declares
//! `tests/main.rs` declares `git_support`, `support` and this module as `h`.
#![allow(dead_code)]

use std::path::PathBuf;

use crate::git_support::{Sandbox, write};
use crate::support::*;
use apinata_app::changes_dialog::{ChangesView, current, open_changes};
use apinata_app::git_status::GitStage;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext};

pub fn committed(sb: &Sandbox, files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = write_tree(files);
    sb.git(dir.path(), &["init", "-q", "-b", "main"]);
    sb.commit_all(dir.path(), "init");
    dir
}

/// `a.yml` modified, `b.yml` committed unchanged, `new.yml` untracked.
pub fn changed(sb: &Sandbox) -> tempfile::TempDir {
    let col = committed(
        sb,
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &request("a", "http://x/a")),
            ("b.yml", &request("b", "http://x/b")),
        ],
    );
    write(col.path(), "a.yml", &request("a", "http://x/changed"));
    write(col.path(), "new.yml", &request("n", "http://x/n"));
    col
}

pub fn git_started(cx: &mut TestAppContext, sb: &Sandbox, col: &tempfile::TempDir) -> Started {
    let s = started(cx, col);
    let options = sb.options();
    s.ws.update(cx, |w, cx| w.set_git(Some(options), cx));
    frame(cx, s.win);
    s
}

/// Writes an executable hook with a child shell, so no descriptor of this process is open on
/// it when another test forks (the same reason `Sandbox::fake_git` does it).
#[cfg(unix)]
pub fn hook(repo: &std::path::Path, name: &str, body: &str) {
    let path = repo.join(".git/hooks").join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let ok = std::process::Command::new("sh")
        .arg("-c")
        .arg("printf '%s' \"$1\" > \"$2\" && chmod 755 \"$2\"")
        .arg("sh")
        .arg(body)
        .arg(&path)
        .status()
        .unwrap();
    assert!(ok.success());
}

/// Sandbox options with every author and committer variable removed; fails clearly when the
/// test process itself has any set.
pub fn without_identity(sb: &Sandbox) -> apinata_core::GitOptions {
    for (name, _) in std::env::vars() {
        assert!(
            !name.starts_with("GIT_AUTHOR_") && !name.starts_with("GIT_COMMITTER_"),
            "{name} is set in the test process; unset it to run the identity tests"
        );
    }
    sb.env()
        .into_iter()
        .filter(|(k, _)| {
            let k = k.to_string_lossy();
            !k.starts_with("GIT_AUTHOR_") && !k.starts_with("GIT_COMMITTER_")
        })
        .fold(apinata_core::GitOptions::new(), |o, (k, v)| o.env(k, v))
}

pub fn open_on(cx: &mut TestAppContext, s: &Started, root: &std::path::Path) {
    let (ws, root) = (s.ws.clone(), root.to_path_buf());
    cx.update_window(s.win, |_, w, cx| {
        open_changes(&ws, &root, None::<PathBuf>, w, cx)
    })
    .unwrap();
    frame(cx, s.win);
}

pub fn view(cx: &mut TestAppContext) -> Entity<ChangesView> {
    cx.update(|cx| current(cx)).expect("the dialog is open")
}

pub fn stages(cx: &mut TestAppContext) -> Vec<GitStage> {
    view(cx).read_with(cx, |v, _| v.stages().to_vec())
}

pub fn click(cx: &mut TestAppContext, s: &Started, id: impl Into<gpui_kit::ElementId>) {
    let id = id.into();
    cx.update_window(s.win, |_, w, cx| w.click(id, cx)).unwrap();
}

/// Waits until the dialog has no operation running, then draws a frame.
pub fn idle(cx: &mut TestAppContext, s: &Started) {
    wait_for(cx, "the dialog to go idle", |cx| {
        !view(cx).read_with(cx, |v, _| v.busy())
    });
    frame(cx, s.win);
}

pub fn label(
    cx: &mut TestAppContext,
    s: &Started,
    id: impl Into<gpui_kit::ElementId>,
) -> Option<String> {
    let id = id.into();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}
