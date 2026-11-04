//! The clone dialog end to end over local repositories: form rules, progress, cancel, opening.

use crate::git_support::{Sandbox, path_with, wait_gone, write};
use crate::support::*;

use std::path::{Path, PathBuf};

use apinata_app::app::AppState;
use apinata_app::clone_dialog::{self, CloneView, Phase};
use apinata_app::clone_job::{CloneEvent, CloneJob};
use apinata_core::{CloneError, GitOptions};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};

fn app(cx: &mut TestAppContext, options: GitOptions) -> (tempfile::TempDir, Started) {
    // The job's thread wakes the pump from outside the test thread.
    cx.executor().allow_parking();
    let col = collection();
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| w.set_git(Some(options), cx));
    frame(cx, s.win);
    (col, s)
}

fn open_dialog(cx: &mut TestAppContext, s: &Started) -> Entity<CloneView> {
    let ws = s.ws.clone();
    let view = cx
        .update_window(s.win, |_, w, cx| clone_dialog::open(&ws, w, cx))
        .unwrap();
    frame(cx, s.win);
    view
}

fn fill(
    cx: &mut TestAppContext,
    s: &Started,
    view: &Entity<CloneView>,
    url: &str,
    parent: &Path,
    name: &str,
) {
    let (view, parent) = (view.clone(), parent.to_path_buf());
    cx.update_window(s.win, |_, w, cx| {
        view.update(cx, |v, cx| {
            v.set_url(url, w, cx);
            v.set_name(name, w, cx);
            v.set_parent(parent, cx);
        })
    })
    .unwrap();
}

fn click(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) {
    cx.update_window(win, |_, w, cx| w.click(id, cx)).unwrap();
    frame(cx, win);
}

fn dialog_open(cx: &mut TestAppContext, win: AnyWindowHandle) -> bool {
    cx.update_window(win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap()
}

fn toasts(cx: &mut TestAppContext, win: AnyWindowHandle) -> usize {
    cx.update_window(win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

fn phase(cx: &mut TestAppContext, view: &Entity<CloneView>) -> Phase {
    view.read_with(cx, |v, _| v.phase())
}

fn dest_parent(sb: &Sandbox) -> PathBuf {
    let p = sb.path().join("dest");
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[gpui_kit::test]
fn a_clone_with_a_collection_at_its_root_opens_it_and_remembers_the_parent(
    cx: &mut TestAppContext,
) {
    let sb = Sandbox::new();
    let bare = sb.origin();
    let (_col, s) = app(cx, sb.options());
    let view = open_dialog(cx, &s);
    let parent = dest_parent(&sb);
    fill(cx, &s, &view, bare.to_str().unwrap(), &parent, "col");
    click(cx, s.win, "clone-start");
    let root = parent.join("col");
    wait_for(cx, "the collection to open", |cx| {
        s.ws.read_with(cx, |w, _| w.collections().iter().any(|c| c.root() == root))
    });
    assert!(!dialog_open(cx, s.win));
    let saved = cx.update(|cx| {
        cx.global::<AppState>()
            .settings
            .read(cx)
            .get()
            .clone_parent
            .clone()
    });
    assert_eq!(saved, Some(parent));
}

#[gpui_kit::test]
fn a_repository_without_a_collection_stays_on_disk_with_a_notice(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let plain = sb.repo("plain");
    write(&plain, "readme.txt", "hi\n");
    sb.commit_all(&plain, "init");
    let (_col, s) = app(cx, sb.options());
    let view = open_dialog(cx, &s);
    let parent = dest_parent(&sb);
    fill(
        cx,
        &s,
        &view,
        plain.to_str().unwrap(),
        &parent,
        "plain-clone",
    );
    click(cx, s.win, "clone-start");
    wait_for(cx, "the dialog to close", |cx| !dialog_open(cx, s.win));
    assert!(parent.join("plain-clone/readme.txt").is_file());
    assert_eq!(toasts(cx, s.win), 1);
    let opened = s.ws.read_with(cx, |w, _| w.collections().len());
    assert_eq!(opened, 1, "only the collection the app started with");
}

#[gpui_kit::test]
fn an_existing_folder_is_refused_inline_and_left_alone(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let bare = sb.origin();
    let (_col, s) = app(cx, sb.options());
    let view = open_dialog(cx, &s);
    let parent = dest_parent(&sb);
    write(&parent, "col/keep.txt", "mine\n");
    fill(cx, &s, &view, bare.to_str().unwrap(), &parent, "col");
    click(cx, s.win, "clone-start");
    assert_eq!(
        view.read_with(cx, |v, _| v.error().map(str::to_owned))
            .as_deref(),
        Some("That folder already exists")
    );
    assert_eq!(phase(cx, &view), Phase::Form);
    assert!(dialog_open(cx, s.win) && present(cx, s.win, "clone-error"));
    assert_eq!(
        std::fs::read_to_string(parent.join("col/keep.txt")).unwrap(),
        "mine\n"
    );
}

#[gpui_kit::test]
fn a_rejected_url_never_starts_git(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_col, s) = app(cx, sb.options());
    let view = open_dialog(cx, &s);
    fill(cx, &s, &view, "-oops", &dest_parent(&sb), "col");
    click(cx, s.win, "clone-start");
    assert_eq!(phase(cx, &view), Phase::Form);
    let error = view
        .read_with(cx, |v, _| v.error().map(str::to_owned))
        .unwrap();
    assert!(error.contains("invalid repository URL"), "{error}");
}

#[gpui_kit::test]
fn typing_the_url_fills_the_name_until_it_is_edited(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_col, s) = app(cx, sb.options());
    let view = open_dialog(cx, &s);
    replace_input(
        cx,
        s.win,
        None,
        "clone-url",
        "https://example.com/team/api.git",
    );
    assert_eq!(view.read_with(cx, |v, cx| v.name(cx)), "api");
    cx.update_window(s.win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_name("mine", w, cx))
    })
    .unwrap();
    cx.update_window(s.win, |_, w, cx| {
        view.update(cx, |v, cx| {
            v.set_url("https://example.com/other.git", w, cx)
        })
    })
    .unwrap();
    assert_eq!(view.read_with(cx, |v, cx| v.name(cx)), "mine");
}

#[gpui_kit::test]
fn a_failed_clone_returns_to_the_form_with_gits_text_and_a_hint(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let body = "#!/bin/sh\ncase \"$*\" in *\" config \"*) exit 1 ;; esac\nfor last; do :; done\nmkdir -p \"$last\"\necho 'fatal: Authentication failed' >&2\nexit 128\n";
    let bin = sb.fake_git("failing", body);
    let (_col, s) = app(cx, sb.options().env("PATH", path_with(&bin)));
    let view = open_dialog(cx, &s);
    let parent = dest_parent(&sb);
    fill(cx, &s, &view, "/srv/x.git", &parent, "col");
    click(cx, s.win, "clone-start");
    wait_for(cx, "the failure", |cx| {
        view.read_with(cx, |v, _| v.error().is_some())
    });
    let error = view
        .read_with(cx, |v, _| v.error().map(str::to_owned))
        .unwrap();
    assert!(
        error.contains("Authentication failed") && error.contains("credential helper"),
        "{error}"
    );
    assert_eq!(phase(cx, &view), Phase::Form);
    assert!(dialog_open(cx, s.win));
    assert!(!parent.join("col").exists());
}

#[gpui_kit::test]
fn a_closed_event_channel_without_a_result_returns_to_the_form(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let bin = sb.fake_git("hang", crate::git_support::HANG_GIT);
    let (_col, s) = app(cx, sb.options().env("PATH", path_with(&bin)));
    let view = open_dialog(cx, &s);
    fill(cx, &s, &view, "/srv/x.git", &dest_parent(&sb), "col");
    click(cx, s.win, "clone-start");
    assert_eq!(phase(cx, &view), Phase::Running);
    cx.update_window(s.win, |_, w, cx| {
        view.update(cx, |v, cx| v.events_lost(w, cx))
    })
    .unwrap();
    assert_eq!(phase(cx, &view), Phase::Form);
    let error = view
        .read_with(cx, |v, _| v.error().map(str::to_owned))
        .unwrap();
    assert!(error.contains("stopped unexpectedly"), "{error}");
    assert!(dialog_open(cx, s.win));
}

#[gpui_kit::test]
fn cancel_stops_a_running_clone_and_leaves_nothing_behind(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let bin = stalling_git(&sb, "clone");
    let (_col, s) = app(cx, sb.options().env("PATH", path_with(&bin)));
    let view = open_dialog(cx, &s);
    let parent = dest_parent(&sb);
    fill(
        cx,
        &s,
        &view,
        "https://me:s3cret@example.com/x.git",
        &parent,
        "col",
    );
    click(cx, s.win, "clone-start");
    wait_for(cx, "10% progress", |cx| {
        view.read_with(cx, |v, _| {
            v.progress().is_some_and(|p| p.percent == Some(10))
        })
    });
    assert_eq!(phase(cx, &view), Phase::Running);
    let shown = view.read_with(cx, |v, _| v.shown_url().map(str::to_owned));
    assert_eq!(shown.as_deref(), Some("https://example.com/x.git"));
    frame(cx, s.win);
    click(cx, s.win, "clone-cancel");
    wait_for(cx, "the dialog to close", |cx| !dialog_open(cx, s.win));
    assert!(
        !parent.join("col").exists(),
        "core removed the folder before the dialog closed"
    );
    assert!(wait_gone(sleeper_pid(&bin)));
    assert_eq!(toasts(cx, s.win), 0, "a cancel is not an error");
}

#[gpui_kit::test]
fn escape_cancels_like_the_button(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let bin = stalling_git(&sb, "clone");
    let (_col, s) = app(cx, sb.options().env("PATH", path_with(&bin)));
    let view = open_dialog(cx, &s);
    let parent = dest_parent(&sb);
    fill(cx, &s, &view, "/srv/x.git", &parent, "col");
    click(cx, s.win, "clone-start");
    wait_for(cx, "10% progress", |cx| {
        view.read_with(cx, |v, _| v.progress().is_some())
    });
    cx.update_window(s.win, |_, w, cx| w.press("escape", cx))
        .unwrap();
    wait_for(cx, "the dialog to close", |cx| !dialog_open(cx, s.win));
    assert!(!parent.join("col").exists());
}

#[gpui_kit::test]
fn a_long_folder_path_is_clipped_and_browse_stays_inside_the_dialog(cx: &mut TestAppContext) {
    use gpui_kit::{px, size};
    let (_col, s) = app(cx, GitOptions::default());
    let view = open_dialog(cx, &s);
    let long = PathBuf::from(format!("/{}", "very-long-folder-name/".repeat(30)));
    let measure = |cx: &mut TestAppContext| {
        frame(cx, s.win);
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            (
                w.find("clone-dialog").bounds(),
                w.find("clone-browse").bounds(),
                w.bounds(),
            )
        })
        .unwrap()
    };
    let (short_dialog, _, _) = measure(cx);
    for width in [1000., 420.] {
        gpui_kit::VisualTestContext::from_window(s.win, cx)
            .simulate_resize(size(px(width), px(700.)));
        let v = view.clone();
        let p = long.clone();
        cx.update_window(s.win, |_, _, cx| v.update(cx, |v, cx| v.set_parent(p, cx)))
            .unwrap();
        let (dialog, browse, window) = measure(cx);
        assert!(
            browse.left() >= dialog.left() && browse.right() <= dialog.right(),
            "at {width}: browse {browse:?} outside dialog {dialog:?}"
        );
        assert!(
            dialog.right() <= window.right(),
            "at {width}: dialog {dialog:?} outside window {window:?}"
        );
        assert!(
            dialog.size.width <= short_dialog.size.width,
            "at {width}: dialog grew to {dialog:?} from {short_dialog:?}"
        );
    }
}

/// Starts the dialog on a scripted job; the returned sender plays git's thread.
fn scripted(
    cx: &mut TestAppContext,
    s: &Started,
    view: &Entity<CloneView>,
    parent: &Path,
) -> async_channel::Sender<CloneEvent> {
    fill(cx, s, view, "/srv/x.git", parent, "col");
    let (tx, rx) = async_channel::unbounded();
    let v = view.clone();
    cx.update_window(s.win, |_, w, cx| {
        v.update(cx, |v, cx| {
            v.start_with(|_, _, _| CloneJob::from_events(rx), w, cx)
        })
    })
    .unwrap();
    frame(cx, s.win);
    assert_eq!(phase(cx, view), Phase::Running);
    tx
}

fn url_focused(cx: &mut TestAppContext, s: &Started, view: &Entity<CloneView>) -> bool {
    let v = view.clone();
    cx.update_window(s.win, |_, w, cx| v.read(cx).url_focused(w, cx))
        .unwrap()
}

#[gpui_kit::test]
fn a_panicking_clone_thread_returns_to_the_form_through_the_pump(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_col, s) = app(cx, sb.options());
    let view = open_dialog(cx, &s);
    let tx = scripted(cx, &s, &view, &dest_parent(&sb));
    drop(tx);
    wait_for(cx, "the form", |cx| phase(cx, &view) == Phase::Form);
    let error = view.read_with(cx, |v, _| v.error().map(str::to_owned));
    assert!(error.unwrap().contains("stopped unexpectedly"));
    assert!(dialog_open(cx, s.win));
    frame(cx, s.win);
    assert!(url_focused(cx, &s, &view), "the URL field has focus again");
}

#[gpui_kit::test]
fn a_failed_clone_focuses_the_url_again(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_col, s) = app(cx, sb.options());
    let view = open_dialog(cx, &s);
    let tx = scripted(cx, &s, &view, &dest_parent(&sb));
    let failure = CloneError::Failed {
        stderr: "fatal: nope".into(),
    };
    tx.try_send(CloneEvent::Finished(Err(failure))).unwrap();
    wait_for(cx, "the form", |cx| phase(cx, &view) == Phase::Form);
    frame(cx, s.win);
    assert!(url_focused(cx, &s, &view), "the URL field has focus again");
}

#[gpui_kit::test]
fn losing_the_events_while_cancelling_closes_quietly(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_col, s) = app(cx, sb.options());
    let view = open_dialog(cx, &s);
    let tx = scripted(cx, &s, &view, &dest_parent(&sb));
    click(cx, s.win, "clone-cancel");
    assert_eq!(phase(cx, &view), Phase::Cancelling);
    drop(tx);
    wait_for(cx, "the dialog to close", |cx| !dialog_open(cx, s.win));
    assert_eq!(toasts(cx, s.win), 0);
}

#[gpui_kit::test]
fn a_clone_that_finishes_after_a_cancel_is_discarded(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_col, s) = app(cx, sb.options());
    let view = open_dialog(cx, &s);
    let parent = dest_parent(&sb);
    // A folder that was already there must survive.
    write(&parent, "other/keep.txt", "mine\n");
    let tx = scripted(cx, &s, &view, &parent);
    click(cx, s.win, "clone-cancel");
    assert_eq!(phase(cx, &view), Phase::Cancelling);
    // The job won the race: its folder exists and Finished(Ok) is the last event.
    write(&parent, "col/opencollection.yml", "opencollection: 1.0.0\n");
    let root = parent.join("col");
    tx.try_send(CloneEvent::Finished(Ok(root.clone()))).unwrap();
    wait_for(cx, "the dialog to close", |cx| !dialog_open(cx, s.win));
    assert!(!root.exists(), "the discarded clone is removed");
    assert!(parent.join("other/keep.txt").is_file());
    let opened = s.ws.read_with(cx, |w, _| w.collections().len());
    assert_eq!(opened, 1, "nothing was opened");
    let saved = cx.update(|cx| {
        cx.global::<AppState>()
            .settings
            .read(cx)
            .get()
            .clone_parent
            .clone()
    });
    assert_eq!(saved, None, "no parent remembered");
    assert_eq!(toasts(cx, s.win), 0);
}

#[gpui_kit::test]
fn enter_on_the_focused_browse_button_asks_for_a_folder(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_col, s) = app(cx, sb.options());
    let _view = open_dialog(cx, &s);
    enter_on(cx, s.win, "clone-browse");
    frame(cx, s.win);
    assert!(dialog_open(cx, s.win), "the dialog stays open");
    assert!(cx.did_prompt_for_paths(), "Enter pressed Browse");
}

fn press(cx: &mut TestAppContext, win: AnyWindowHandle, key: &str) {
    cx.update_window(win, |_, w, cx| w.press(key, cx)).unwrap();
    cx.run_until_parked();
    frame(cx, win);
}

#[gpui_kit::test]
fn enter_with_an_empty_url_keeps_the_form_without_an_error(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_col, s) = app(cx, sb.options());
    let view = open_dialog(cx, &s);
    assert!(url_focused(cx, &s, &view));
    press(cx, s.win, "enter");
    assert!(dialog_open(cx, s.win), "the dialog stays open");
    assert_eq!(phase(cx, &view), Phase::Form);
    assert_eq!(
        view.read_with(cx, |v, _| v.error().map(str::to_owned)),
        None
    );
}

#[gpui_kit::test]
fn enter_in_the_url_field_clones_like_the_button(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let bare = sb.origin();
    let (_col, s) = app(cx, sb.options());
    let view = open_dialog(cx, &s);
    let parent = dest_parent(&sb);
    fill(cx, &s, &view, bare.to_str().unwrap(), &parent, "col");
    assert!(url_focused(cx, &s, &view));
    press(cx, s.win, "enter");
    let root = parent.join("col");
    wait_for(cx, "the collection to open", |cx| {
        s.ws.read_with(cx, |w, _| w.collections().iter().any(|c| c.root() == root))
    });
    assert!(!dialog_open(cx, s.win));
}

#[gpui_kit::test]
fn enter_in_the_name_field_with_a_rejected_url_shows_the_error(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_col, s) = app(cx, sb.options());
    let view = open_dialog(cx, &s);
    fill(cx, &s, &view, "-oops", &dest_parent(&sb), "col");
    type_at_end(cx, s.win, None, "clone-name", "");
    press(cx, s.win, "enter");
    assert!(dialog_open(cx, s.win), "the dialog stays open");
    assert_eq!(phase(cx, &view), Phase::Form);
    let error = view
        .read_with(cx, |v, _| v.error().map(str::to_owned))
        .expect("the button's error");
    assert!(error.contains("invalid repository URL"), "{error}");
}

#[gpui_kit::test]
fn escape_on_the_form_closes_the_dialog(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_col, s) = app(cx, sb.options());
    let _view = open_dialog(cx, &s);
    press(cx, s.win, "escape");
    assert!(!dialog_open(cx, s.win));
}

#[gpui_kit::test]
fn enter_while_a_clone_runs_keeps_the_dialog(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_col, s) = app(cx, sb.options());
    let view = open_dialog(cx, &s);
    let _tx = scripted(cx, &s, &view, &dest_parent(&sb));
    press(cx, s.win, "enter");
    assert!(dialog_open(cx, s.win), "the dialog stays open");
    assert_eq!(phase(cx, &view), Phase::Running);
}
