//! The branch picker in the real app: label, chevron, palette command, closing the collection.

use crate::git_support::{Sandbox, path_with};
use crate::support::*;

use apinata_app::branch_dialog;
use apinata_app::shell::SwitchBranch;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Focusable as _, TestAppContext};

/// The real app over a clone of the sandbox origin, git on; `prep` runs after the origin exists
/// and before the clone.
pub fn app(
    cx: &mut TestAppContext,
    sb: &Sandbox,
    prep: impl FnOnce(&Sandbox),
) -> (tempfile::TempDir, Started) {
    // The picker runs git on its own threads, which the test scheduler must tolerate.
    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    let bare = sb.origin();
    prep(sb);
    sb.git(
        sb.path(),
        &[
            "clone",
            "-q",
            bare.to_str().unwrap(),
            dir.path().to_str().unwrap(),
        ],
    );
    let s = started(cx, &dir);
    let options = sb.options();
    s.ws.update(cx, |w, cx| w.set_git(Some(options), cx));
    let root = s.root.clone();
    wait_for(cx, "the first git status", |cx| {
        s.ws.read_with(cx, |w, _| w.git_changes(&root).is_some())
    });
    frame(cx, s.win);
    (dir, s)
}

fn chevron(cx: &mut TestAppContext, s: &Started) -> bool {
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        w.within("title-bar")
            .try_find("git-branch-chevron")
            .is_some()
    })
    .unwrap()
}

fn label(cx: &mut TestAppContext, s: &Started) -> Option<String> {
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        w.within("title-bar")
            .try_find("git-label")
            .and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

pub fn open_picker(cx: &mut TestAppContext, s: &Started) {
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("git-branch-chevron", cx)
    })
    .unwrap();
    wait_for(cx, "the branch list", |cx| {
        cx.update(|cx| branch_dialog::current(cx))
            .is_some_and(|p| p.read_with(cx, |p, _| !p.branches().is_empty()))
    });
    frame(cx, s.win);
}

fn click_row(cx: &mut TestAppContext, s: &Started, name: &str) {
    let picker = cx.update(|cx| branch_dialog::current(cx)).expect("open");
    let ix = picker.read_with(cx, |p, _| {
        // `remote/name` picks the remote-only row; a bare name picks the local branch first.
        let rows = p.branches();
        let remote = |b: &apinata_core::Branch| {
            b.local.is_none()
                && b.upstream
                    .as_ref()
                    .is_some_and(|u| format!("{}/{}", u.remote, u.name) == name)
        };
        rows.iter()
            .position(remote)
            .or_else(|| rows.iter().position(|b| b.local.as_deref() == Some(name)))
            .or_else(|| {
                rows.iter().position(|b| {
                    b.local.is_none() && b.upstream.as_ref().is_some_and(|u| u.name == name)
                })
            })
            .expect("listed")
    });
    cx.update_window(s.win, |_, w, cx| w.click(("branch-row", ix), cx))
        .unwrap();
    frame(cx, s.win);
}

fn dialog_open(cx: &mut TestAppContext, s: &Started) -> bool {
    cx.update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap()
}

fn toasts(cx: &mut TestAppContext, s: &Started) -> usize {
    cx.update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

#[gpui_kit::test]
fn the_chevron_sits_beside_the_label_in_a_repository(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_dir, s) = app(cx, &sb, |_| {});
    assert!(chevron(cx, &s));
    assert_eq!(label(cx, &s).as_deref(), Some("main"));
}

#[gpui_kit::test]
fn a_collection_outside_a_repository_has_no_chevron(cx: &mut TestAppContext) {
    let plain = collection();
    let p = started(cx, &plain);
    assert!(!chevron(cx, &p));
}

#[gpui_kit::test]
fn switching_updates_the_label_without_a_notice(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_dir, s) = app(cx, &sb, |_| {});
    open_picker(cx, &s);
    click_row(cx, &s, "feature");
    wait_for(cx, "the dialog to close", |cx| !dialog_open(cx, &s));
    assert_eq!(label(cx, &s).as_deref(), Some("feature"));
    assert_eq!(toasts(cx, &s), 0);
}

#[gpui_kit::test]
fn a_branch_tracking_another_upstream_switches_with_one_notice(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (dir, s) = app(cx, &sb, |_| {});
    sb.git(dir.path(), &["branch", "--track", "feature", "origin/main"]);
    open_picker(cx, &s);
    click_row(cx, &s, "origin/feature");
    wait_for(cx, "the dialog to close", |cx| !dialog_open(cx, &s));
    assert_eq!(label(cx, &s).as_deref(), Some("feature"));
    assert_eq!(toasts(cx, &s), 1);
}

#[gpui_kit::test]
fn a_branch_without_the_collection_closes_it_with_one_notice(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_dir, s) = app(cx, &sb, |sb| {
        let seed = sb.path().join("seed");
        sb.git(&seed, &["switch", "-q", "-c", "nocol", "main"]);
        sb.git(&seed, &["rm", "-q", "opencollection.yml"]);
        sb.commit_all(&seed, "no collection");
        sb.git(&seed, &["push", "-q", "origin", "nocol"]);
    });
    open_picker(cx, &s);
    click_row(cx, &s, "nocol");
    wait_for(cx, "the collection to close", |cx| {
        s.ws.read_with(cx, |w, _| w.collections().is_empty())
    });
    assert!(!dialog_open(cx, &s));
    assert_eq!(toasts(cx, &s), 1);
}

#[gpui_kit::test]
fn the_palette_command_opens_the_picker(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_dir, s) = app(cx, &sb, |_| {});
    let shell = s.shell.clone();
    cx.update_window(s.win, |_, w, cx| {
        shell.read(cx).focus_handle(cx).focus(w, cx);
        w.dispatch_action(Box::new(SwitchBranch), cx);
    })
    .unwrap();
    frame(cx, s.win);
    assert!(dialog_open(cx, &s));
}

#[gpui_kit::test]
fn the_palette_command_explains_when_there_is_no_repository(cx: &mut TestAppContext) {
    let plain = collection();
    let p = started(cx, &plain);
    let shell = p.shell.clone();
    cx.update_window(p.win, |_, w, cx| {
        shell.read(cx).focus_handle(cx).focus(w, cx);
        w.dispatch_action(Box::new(SwitchBranch), cx);
    })
    .unwrap();
    frame(cx, p.win);
    assert!(!dialog_open(cx, &p));
    assert_eq!(toasts(cx, &p), 1);
}

/// `app` under a git whose `switch` and `checkout` first block on a latch, then run the real
/// git and exit with `exit_code`.
fn latched_app(
    cx: &mut TestAppContext,
    sb: &Sandbox,
    exit_code: u8,
) -> (tempfile::TempDir, Started, std::path::PathBuf) {
    let body = format!(
        "#!/bin/sh\ndir=${{0%/*}}\ncase \"$*\" in\n*switch*|*checkout*)\n  : > \"$dir/reached\"\n  \
         while [ ! -e \"$dir/release\" ]; do sleep 0.02; done\n  /usr/bin/git \"$@\"\n  \
         exit {exit_code} ;;\nesac\nexec /usr/bin/git \"$@\"\n"
    );
    let bin = sb.fake_git("latched-shell", &body);
    let (dir, s) = app(cx, sb, |_| {});
    let options = sb.options().env("PATH", path_with(&bin));
    s.ws.update(cx, |w, cx| w.set_git(Some(options), cx));
    (dir, s, bin)
}

fn start_held_switch(cx: &mut TestAppContext, s: &Started, bin: &std::path::Path) {
    open_picker(cx, s);
    click_row(cx, s, "feature");
    wait_for(cx, "git to reach the checkout", |_| {
        bin.join("reached").is_file()
    });
}

#[gpui_kit::test]
fn a_slow_switch_updates_the_label_when_it_ends(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_dir, s, bin) = latched_app(cx, &sb, 0);
    start_held_switch(cx, &s, &bin);
    assert_eq!(label(cx, &s).as_deref(), Some("main"));
    release_git(&bin);
    wait_for(cx, "the dialog to close", |cx| !dialog_open(cx, &s));
    assert_eq!(label(cx, &s).as_deref(), Some("feature"));
}

#[gpui_kit::test]
fn a_switch_whose_reload_times_out_still_updates_the_label(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_dir, s, bin) = latched_app(cx, &sb, 0);
    // Reopen the collection with an actor that can be stalled.
    let stall = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let root = s.root.clone();
    s.ws.update(cx, |w, cx| {
        w.close(&root, cx);
        w.set_open_faults(apinata_app::actor::Faults {
            stall: Some(stall.clone()),
            ..Default::default()
        });
        w.open(root.clone(), cx);
    });
    cx.run_until_parked();
    frame(cx, s.win);
    start_held_switch(cx, &s, &bin);
    stall.store(true, std::sync::atomic::Ordering::SeqCst);
    release_git(&bin);
    let picker = cx.update(|cx| branch_dialog::current(cx)).expect("open");
    wait_for(cx, "git to finish", |cx| {
        picker.read_with(cx, |p, _| p.current() == Some("feature"))
    });
    cx.executor()
        .advance_clock(apinata_app::branch_picker::RELOAD_BOUND * 2);
    wait_for(cx, "the picker to be idle", |cx| {
        picker.read_with(cx, |p, _| {
            p.activity() == apinata_app::branch_picker::Activity::Idle
        })
    });
    assert_eq!(label(cx, &s).as_deref(), Some("feature"));
    stall.store(false, std::sync::atomic::Ordering::SeqCst);
}

/// Git moved HEAD before it exited non-zero, so the switch landed and is not a failure.
#[gpui_kit::test]
fn a_switch_that_exits_non_zero_after_moving_head_counts_as_switched(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_dir, s, bin) = latched_app(cx, &sb, 1);
    start_held_switch(cx, &s, &bin);
    release_git(&bin);
    wait_for(cx, "the dialog to close", |cx| !dialog_open(cx, &s));
    assert_eq!(label(cx, &s).as_deref(), Some("feature"));
    assert_eq!(toasts(cx, &s), 0);
}

#[gpui_kit::test]
fn a_checkout_outside_the_app_moves_the_label_with_the_count(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (dir, s) = app(cx, &sb, |_| {});
    // The sandbox app has no watcher; turn the real one on and let its first reload settle.
    s.ws.update(cx, |w, cx| {
        w.set_watch_factory(Some(apinata_app::watch::set::notify_factory()), cx)
    });
    let root = s.root.clone();
    wait_for(cx, "the watcher's first reload", |cx| {
        ws_snap(cx, &s.ws, &root).seq >= 2
    });
    assert_eq!(label(cx, &s).as_deref(), Some("main"));
    let runs = s.ws.read_with(cx, |w, _| w.git_runs(&root));
    // Only HEAD moves; the window is never re-activated.
    sb.git(dir.path(), &["switch", "-q", "feature"]);
    wait_for(cx, "the label to follow HEAD", |cx| {
        label(cx, &s).as_deref() == Some("feature")
    });
    // The count was re-read by a run that finished no later than the label update.
    assert!(s.ws.read_with(cx, |w, _| w.git_runs(&root)) > runs);
    assert!(s.ws.read_with(cx, |w, _| w.git_changes(&root).is_some()));
}

fn picker_open(cx: &mut TestAppContext, s: &Started) -> bool {
    frame(cx, s.win);
    dialog_open(cx, s) && cx.update(|cx| branch_dialog::current(cx)).is_some()
}

fn opens_by(cx: &mut TestAppContext, key: &str) {
    let sb = Sandbox::new();
    let (_dir, s) = app(cx, &sb, |_| {});
    tab_to(cx, s.win, "git-branch");
    cx.update_window(s.win, |_, w, cx| w.press(key, cx))
        .unwrap();
    assert!(picker_open(cx, &s), "{key} opens the branch dialog");
}

#[gpui_kit::test]
fn enter_on_the_clean_label_opens_the_branch_dialog(cx: &mut TestAppContext) {
    opens_by(cx, "enter");
}

#[gpui_kit::test]
fn space_on_the_clean_label_opens_the_branch_dialog(cx: &mut TestAppContext) {
    opens_by(cx, "space");
}

#[gpui_kit::test]
fn picking_a_local_only_branch_updates_the_label_without_a_notice(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (dir, s) = app(cx, &sb, |_| {});
    sb.git(dir.path(), &["branch", "topic", "main"]);
    open_picker(cx, &s);
    click_row(cx, &s, "topic");
    wait_for(cx, "the dialog to close", |cx| !dialog_open(cx, &s));
    assert_eq!(label(cx, &s).as_deref(), Some("topic"));
    assert_eq!(toasts(cx, &s), 0);
}
