//! Failures the user would otherwise never see: a file dialog that could not open, and a
//! script trust answer that could not be stored, each end in an error toast.

use crate::support;

use std::path::PathBuf;

use apinata_app::file_dialog::{answer, failed_text};
use gpui_kit::component::WindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext};
use support::*;

type Reply = Result<Result<Option<Vec<PathBuf>>, String>, String>;

fn toasts(cx: &mut TestAppContext, win: AnyWindowHandle) -> usize {
    cx.update_window(win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

// Hands `reply` to the shared helper in the shell window, as a dialog's await would.
fn answered(cx: &mut TestAppContext, s: &Started, reply: Reply) -> Option<Vec<PathBuf>> {
    let got = cx
        .update_window(s.win, |_, w, cx| answer(reply, "open a collection", w, cx))
        .unwrap();
    frame(cx, s.win);
    got
}

fn empty() -> tempfile::TempDir {
    write_tree(&[("opencollection.yml", ROOT)])
}

#[gpui_kit::test]
fn a_cancelled_dialog_stays_silent(cx: &mut TestAppContext) {
    let col = empty();
    let s = started(cx, &col);
    let before = toasts(cx, s.win);
    assert_eq!(answered(cx, &s, Ok(Ok(None))), None);
    assert_eq!(toasts(cx, s.win), before);
    assert!(!present(cx, s.win, "toast-error"));
}

#[gpui_kit::test]
fn chosen_paths_come_back_without_a_toast(cx: &mut TestAppContext) {
    let col = empty();
    let s = started(cx, &col);
    let before = toasts(cx, s.win);
    let paths = vec![PathBuf::from("/a"), PathBuf::from("/b")];
    assert_eq!(answered(cx, &s, Ok(Ok(Some(paths.clone())))), Some(paths));
    assert_eq!(toasts(cx, s.win), before);
}

#[gpui_kit::test]
fn a_platform_error_shows_one_error_toast(cx: &mut TestAppContext) {
    let col = empty();
    let s = started(cx, &col);
    let before = toasts(cx, s.win);
    assert_eq!(answered(cx, &s, Ok(Err("no portal".into()))), None);
    assert_eq!(toasts(cx, s.win), before + 1);
    assert!(present(cx, s.win, "toast-error"));
}

#[gpui_kit::test]
fn a_dropped_dialog_shows_one_error_toast(cx: &mut TestAppContext) {
    let col = empty();
    let s = started(cx, &col);
    let before = toasts(cx, s.win);
    assert_eq!(answered(cx, &s, Err("oneshot canceled".into())), None);
    assert_eq!(toasts(cx, s.win), before + 1);
    assert!(present(cx, s.win, "toast-error"));
}

#[test]
fn the_toast_names_the_action_and_the_error() {
    assert_eq!(
        failed_text("open a collection", "no portal"),
        "Could not open the file dialog to open a collection: no portal"
    );
}

// Every dialog goes through the shared helper; a hand-rolled match would drop failures again.
#[test]
fn no_call_site_drops_a_dialog_failure() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    let mut dirs = vec![src];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                if text.contains("let Ok(Ok(Some(") || text.contains(".ok()?.ok()?") {
                    offenders.push(path);
                }
            }
        }
    }
    assert!(offenders.is_empty(), "{offenders:?}");
}
