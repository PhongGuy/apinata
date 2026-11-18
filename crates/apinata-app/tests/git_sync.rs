//! Pull and push from the branch picker, against a local bare repository as the remote.

use crate::git_support::{Sandbox, path_with, write};
use crate::support::*;

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use apinata_app::branch_dialog;
use apinata_app::branch_picker::{Activity, BranchPicker, Notice, PickerEvent};
use apinata_app::command::{Command, Edit, Target};
use apinata_app::shell::{PullBranch, PushBranch};
use apinata_core::{MemoryStore, SecretStore as _};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext};

struct Rig {
    sb: Sandbox,
    s: Started,
    picker: Entity<BranchPicker>,
    events: Rc<RefCell<Vec<PickerEvent>>>,
}

impl Rig {
    fn root(&self) -> &Path {
        &self.s.root
    }
    fn seed(&self) -> std::path::PathBuf {
        self.sb.path().join("seed")
    }
    fn head(&self) -> String {
        rev(&self.sb, self.root(), "HEAD")
    }
}

fn rev(sb: &Sandbox, repo: &Path, name: &str) -> String {
    sb.git(repo, &["rev-parse", name]).trim().to_owned()
}

/// Commits `file` with `text` on origin's `main` from the seed work tree; returns the new tip.
fn advance(sb: &Sandbox, file: &str, text: &str) -> String {
    let seed = sb.path().join("seed");
    sb.git(&seed, &["switch", "-q", "main"]);
    write(&seed, file, text);
    sb.commit_all(&seed, file);
    sb.git(&seed, &["push", "-q", "origin", "main"]);
    rev(sb, &seed, "HEAD")
}

/// The real app over a clone of the sandbox origin with its branch picker open; `prep` runs
/// on the origin's seed before the clone.
fn rig(cx: &mut TestAppContext, sb: Sandbox, prep: impl FnOnce(&Sandbox)) -> Rig {
    rig_on(cx, sb, None, prep)
}

fn rig_on(
    cx: &mut TestAppContext,
    sb: Sandbox,
    options: Option<apinata_core::GitOptions>,
    prep: impl FnOnce(&Sandbox),
) -> Rig {
    let (dir, s) = crate::branch_shell::app(cx, &sb, prep);
    std::mem::forget(dir);
    if let Some(options) = options {
        s.ws.update(cx, |w, cx| w.set_git(Some(options), cx));
    }
    let (ws, root) = (s.ws.clone(), s.root.clone());
    let picker = cx
        .update_window(s.win, |_, w, cx| branch_dialog::open(&ws, root, w, cx))
        .unwrap();
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    cx.update(|cx| {
        cx.subscribe(&picker, move |_, e: &PickerEvent, _| {
            sink.borrow_mut().push(e.clone())
        })
        .detach()
    });
    let r = Rig {
        sb,
        s,
        picker,
        events,
    };
    idle(cx, &r);
    r
}

fn idle(cx: &mut TestAppContext, r: &Rig) {
    wait_for(cx, "the picker to be idle", |cx| {
        r.picker
            .read_with(cx, |p, _| p.activity() == Activity::Idle)
    });
}

fn notice(cx: &mut TestAppContext, r: &Rig) -> Option<Notice> {
    r.picker.read_with(cx, |p, _| p.notice().cloned())
}

fn notice_text(cx: &mut TestAppContext, r: &Rig) -> String {
    notice(cx, r)
        .map(|n| n.text().to_owned())
        .unwrap_or_default()
}

fn synced(r: &Rig) -> Vec<String> {
    (r.events.borrow().iter())
        .filter_map(|e| match e {
            PickerEvent::Synced { message } => Some(message.clone()),
            _ => None,
        })
        .collect()
}

fn pull(cx: &mut TestAppContext, r: &Rig) {
    r.picker.update(cx, |p, cx| p.pull(cx));
    idle(cx, r);
}

fn push(cx: &mut TestAppContext, r: &Rig) {
    r.picker.update(cx, |p, cx| p.push(cx));
    idle(cx, r);
}

#[gpui_kit::test]
fn pull_fast_forwards_to_the_upstream_and_says_so(cx: &mut TestAppContext) {
    let r = rig(cx, Sandbox::new(), |_| {});
    let tip = advance(&r.sb, "c.txt", "c\n");
    pull(cx, &r);
    assert_eq!(r.head(), tip);
    assert_eq!(notice(cx, &r), None);
    let said = synced(&r);
    assert!(
        matches!(said.as_slice(), [m] if m.starts_with("Fast-forwarded main to origin/main")),
        "{said:?}"
    );
    assert!(!r.s.ws.read_with(cx, |w, _| w.is_switching(&r.s.root)));
}

#[gpui_kit::test]
fn pull_with_nothing_new_says_it_is_up_to_date(cx: &mut TestAppContext) {
    let r = rig(cx, Sandbox::new(), |_| {});
    let head = r.head();
    pull(cx, &r);
    assert_eq!(r.head(), head);
    assert_eq!(synced(&r), ["main is up to date with origin/main"]);
}

#[gpui_kit::test]
fn a_diverged_branch_reports_both_counts_and_points_at_a_terminal(cx: &mut TestAppContext) {
    let r = rig(cx, Sandbox::new(), |_| {});
    advance(&r.sb, "c.txt", "c\n");
    write(r.root(), "d.txt", "d\n");
    r.sb.commit_all(r.root(), "local");
    let head = r.head();
    pull(cx, &r);
    assert_eq!(r.head(), head);
    let text = notice_text(cx, &r);
    assert!(
        text.contains("diverged (1 local and 1 upstream commits)"),
        "{text}"
    );
    assert!(text.contains("merge or rebase in a terminal"), "{text}");
    assert!(!text.contains("apinata"), "no command names: {text}");
    assert!(synced(&r).is_empty());
}

#[gpui_kit::test]
fn unsaved_edits_refuse_the_pull(cx: &mut TestAppContext) {
    let r = rig(cx, Sandbox::new(), |_| {});
    let tip = advance(&r.sb, "c.txt", "c\n");
    let root = r.s.root.clone();
    let head = r.head();
    r.s.ws.update(cx, |w, cx| {
        w.send(
            &root,
            Command::Edit {
                target: Target::Collection,
                edit: Edit::Docs("draft".into()),
            },
            cx,
        )
    });
    cx.run_until_parked();
    pull(cx, &r);
    assert_eq!(notice(cx, &r), Some(Notice::Unsaved));
    assert_eq!(r.head(), head);
    assert_ne!(r.head(), tip);
}

#[gpui_kit::test]
fn a_dirty_working_tree_asks_to_commit_or_discard_first(cx: &mut TestAppContext) {
    let r = rig(cx, Sandbox::new(), |_| {});
    advance(&r.sb, "c.txt", "c\n");
    write(r.root(), "scratch.txt", "mine\n");
    let head = r.head();
    pull(cx, &r);
    assert_eq!(notice(cx, &r), Some(Notice::Dirty));
    assert_eq!(notice_text(cx, &r), "Commit or discard your changes first");
    assert_eq!(r.head(), head);
}

#[gpui_kit::test]
fn push_without_an_upstream_offers_publish_which_sets_it(cx: &mut TestAppContext) {
    let r = rig(cx, Sandbox::new(), |_| {});
    r.sb.git(r.root(), &["switch", "-q", "-c", "topic"]);
    write(r.root(), "t.txt", "t\n");
    r.sb.commit_all(r.root(), "topic");
    push(cx, &r);
    assert_eq!(
        notice(cx, &r),
        Some(Notice::Publish {
            branch: "topic".into(),
            remote: "origin".into()
        })
    );
    r.picker.update(cx, |p, cx| p.publish(cx));
    idle(cx, &r);
    assert_eq!(
        r.sb.git(r.root(), &["rev-parse", "--abbrev-ref", "topic@{u}"])
            .trim(),
        "origin/topic"
    );
    let said = synced(&r);
    assert!(
        matches!(said.as_slice(), [m] if m.contains("new branch") && m.contains("now tracks origin/topic")),
        "{said:?}"
    );
}

#[gpui_kit::test]
fn push_without_an_upstream_and_two_remotes_names_them(cx: &mut TestAppContext) {
    let r = rig(cx, Sandbox::new(), |_| {});
    let bare = r.sb.path().join("origin.git");
    r.sb.git(
        r.root(),
        &["remote", "add", "backup", bare.to_str().unwrap()],
    );
    r.sb.git(r.root(), &["switch", "-q", "-c", "topic"]);
    push(cx, &r);
    let text = notice_text(cx, &r);
    assert!(text.contains("backup") && text.contains("origin"), "{text}");
    assert!(text.contains("terminal"), "{text}");
    assert!(!matches!(notice(cx, &r), Some(Notice::Publish { .. })));
}

#[gpui_kit::test]
fn a_rejected_push_says_to_pull_first(cx: &mut TestAppContext) {
    let r = rig(cx, Sandbox::new(), |_| {});
    advance(&r.sb, "c.txt", "c\n");
    write(r.root(), "d.txt", "d\n");
    r.sb.commit_all(r.root(), "local");
    push(cx, &r);
    let text = notice_text(cx, &r);
    assert!(text.contains("rejected"), "{text}");
    assert!(text.ends_with("Pull first, then push again."), "{text}");
    assert!(synced(&r).is_empty());
}

#[gpui_kit::test]
fn push_sends_local_commits_and_says_so(cx: &mut TestAppContext) {
    let r = rig(cx, Sandbox::new(), |_| {});
    write(r.root(), "d.txt", "d\n");
    r.sb.commit_all(r.root(), "local");
    push(cx, &r);
    let bare = r.sb.path().join("origin.git");
    assert_eq!(rev(&r.sb, &bare, "main"), r.head());
    let said = synced(&r);
    assert!(
        matches!(said.as_slice(), [m] if m.starts_with("Pushed main to origin/main")),
        "{said:?}"
    );
}

#[gpui_kit::test]
fn an_open_request_the_pull_deletes_is_gone_when_the_pull_is_announced(cx: &mut TestAppContext) {
    let r = rig(cx, Sandbox::new(), |sb| {
        advance(sb, "r.yml", &request("r", "http://x/"));
    });
    let root = r.s.root.clone();
    r.s.ws
        .update(cx, |w, cx| w.open_item(&root, ip("r.yml"), false, cx));
    wait_for(cx, "the tab", |cx| {
        r.s.ws
            .read_with(cx, |w, cx| w.request(&root, &ip("r.yml"), cx).is_some())
    });
    let seed = r.seed();
    r.sb.git(&seed, &["rm", "-q", "r.yml"]);
    r.sb.commit_all(&seed, "drop r");
    r.sb.git(&seed, &["push", "-q", "origin", "main"]);
    let seen = Rc::new(RefCell::new(None));
    let (sink, ws, at) = (seen.clone(), r.s.ws.clone(), root.clone());
    cx.update(|cx| {
        cx.subscribe(&r.picker, move |_, _: &PickerEvent, cx| {
            *sink.borrow_mut() = Some(ws.read(cx).request(&at, &ip("r.yml"), cx).is_some());
        })
        .detach()
    });
    pull(cx, &r);
    assert!(!root.join("r.yml").exists());
    assert_eq!(
        *seen.borrow(),
        Some(false),
        "reloaded before it was announced"
    );
}

#[gpui_kit::test]
fn watcher_reloads_wait_for_the_pull(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let bin = merge_latch(&sb);
    let options = sb.options().env("PATH", path_with(&bin));
    let r = rig_on(cx, sb, Some(options), |_| {});
    let root = r.s.root.clone();
    r.s.ws.update(cx, |w, cx| {
        w.set_watch_factory(Some(apinata_app::watch::set::notify_factory()), cx)
    });
    advance(&r.sb, "c.txt", "c\n");
    r.picker.update(cx, |p, cx| p.pull(cx));
    wait_for(cx, "git to reach the merge", |_| {
        bin.join("reached").is_file()
    });
    assert!(
        r.s.ws.read_with(cx, |w, _| w.is_switching(&root)),
        "read-only"
    );
    write(r.root(), "late.yml", &request("late", "http://x/"));
    // The watcher has time to report; the reload waits for the pull.
    for _ in 0..10 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        cx.run_until_parked();
    }
    assert!(r.s.ws.read_with(cx, |w, _| w.reloads_held(&root)));
    std::fs::remove_file(r.root().join("late.yml")).unwrap();
    std::fs::write(bin.join("release"), "").unwrap();
    idle(cx, &r);
    assert!(!r.s.ws.read_with(cx, |w, _| w.reloads_held(&root)));
    assert!(!r.s.ws.read_with(cx, |w, _| w.is_switching(&root)));
}

/// A git whose `merge` blocks until `release` appears in the returned folder; `reached` appears
/// once it waits.
fn merge_latch(sb: &Sandbox) -> std::path::PathBuf {
    const BODY: &str = r#"#!/bin/sh
dir=${0%/*}
case " $* " in
*" merge "*)
  : > "$dir/reached"
  while [ ! -e "$dir/release" ]; do sleep 0.02; done ;;
esac
PATH=${PATH#*:}; exec git "$@"
"#;
    sb.fake_git("merge-latch", BODY)
}

#[gpui_kit::test]
fn cancel_is_off_once_the_fetch_has_finished(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let bin = merge_latch(&sb);
    let options = sb.options().env("PATH", path_with(&bin));
    let r = rig_on(cx, sb, Some(options), |_| {});
    let tip = advance(&r.sb, "c.txt", "c\n");
    r.picker.update(cx, |p, cx| p.pull(cx));
    assert!(
        r.picker.read_with(cx, |p, _| p.can_cancel()),
        "the fetch can stop"
    );
    wait_for(cx, "git to reach the merge", |_| {
        bin.join("reached").is_file()
    });
    wait_for(cx, "cancel to turn off", |cx| {
        !r.picker.read_with(cx, |p, _| p.can_cancel())
    });
    frame(cx, r.s.win);
    let disabled = cx
        .update_window(r.s.win, |_, w, _| w.find("branch-fetch-cancel").disabled())
        .unwrap();
    assert_eq!(disabled, Some(true), "the Stop button is off");
    // A cancel now changes nothing: the fast-forward finishes.
    r.picker.update(cx, |p, _| p.cancel_fetch());
    std::fs::write(bin.join("release"), "").unwrap();
    idle(cx, &r);
    assert_eq!(r.head(), tip);
}

#[gpui_kit::test]
fn a_pull_that_renames_a_secret_offers_to_move_it(cx: &mut TestAppContext) {
    const DEV: &str = "name: dev\nvariables:\n  - name: token\n    secret: true\n";
    const DEV_RENAMED: &str = "name: dev\nvariables:\n  - name: api_token\n    secret: true\n";
    let sb = Sandbox::new();
    init(cx);
    cx.executor().allow_parking();
    let bare = sb.origin();
    let seed = sb.path().join("seed");
    sb.git(&seed, &["switch", "-q", "main"]);
    write(&seed, "environments/dev.yml", DEV);
    sb.commit_all(&seed, "dev");
    sb.git(&seed, &["push", "-q", "origin", "main"]);
    let col = tempfile::tempdir().unwrap();
    sb.git(
        sb.path(),
        &[
            "clone",
            "-q",
            bare.to_str().unwrap(),
            col.path().to_str().unwrap(),
        ],
    );
    let key = |var: &str| {
        apinata_core::Collection::open(col.path())
            .unwrap()
            .secret_key("dev", var)
    };
    let store = Arc::new(MemoryStore::new());
    store.set(&key("token"), "s3cret").unwrap();
    let s = started_with_store(cx, &col, store.clone());
    s.ws.update(cx, |w, cx| {
        w.set_git(Some(sb.options()), cx);
        w.set_watch_factory(Some(apinata_app::watch::set::notify_factory()), cx);
    });
    let at = rev(&sb, col.path(), "HEAD");
    wait_for(cx, "the first HEAD", |cx| {
        s.ws.read_with(cx, |w, _| w.seen_head(&s.root)) == Some(at.clone())
    });
    write(&seed, "environments/dev.yml", DEV_RENAMED);
    sb.commit_all(&seed, "rename token");
    sb.git(&seed, &["push", "-q", "origin", "main"]);
    let (ws, root) = (s.ws.clone(), s.root.clone());
    let picker = cx
        .update_window(s.win, |_, w, cx| branch_dialog::open(&ws, root, w, cx))
        .unwrap();
    wait_for(cx, "listing", |cx| {
        picker.read_with(cx, |p, _| p.activity() == Activity::Idle)
    });
    picker.update(cx, |p, cx| p.pull(cx));
    wait_for(cx, "the rename row", |cx| {
        s.ws.read_with(cx, |w, _| w.secret_renames(&s.root))
            .is_some_and(|r| r.open_rows().len() == 1)
    });
    drop(col);
}

#[gpui_kit::test]
fn the_pull_command_opens_the_picker_and_pulls(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_dir, s) = crate::branch_shell::app(cx, &sb, |_| {});
    let tip = advance(&sb, "c.txt", "c\n");
    let shell = s.shell.clone();
    cx.update_window(s.win, |_, w, cx| {
        use gpui_kit::Focusable as _;
        shell.read(cx).focus_handle(cx).focus(w, cx);
        w.dispatch_action(Box::new(PullBranch), cx);
    })
    .unwrap();
    wait_for(cx, "the pull", |_| rev(&sb, &s.root, "HEAD") == tip);
    let _ = PushBranch;
}

#[gpui_kit::test]
fn the_palette_offers_pull_and_push_in_a_repository(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_dir, s) = crate::branch_shell::app(cx, &sb, |_| {});
    assert_eq!(
        crate::branch_shell::palette_rows(cx, &s, "pull"),
        ["Pull from remote"]
    );
}

fn team_app(cx: &mut TestAppContext) -> (Sandbox, Started) {
    use crate::support::team::{shared_origin, start_team_as, team_started};
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    http_ready(cx);
    let s = team_started(cx, &sb, &col, |_, _| {}).0;
    start_team_as(
        cx,
        &sb,
        &s,
        apinata_core::team::Role::Write,
        std::time::Duration::from_millis(50),
    );
    std::mem::forget(col);
    (sb, s)
}

#[gpui_kit::test]
fn a_team_collection_hides_the_pull_and_push_buttons(cx: &mut TestAppContext) {
    let (_sb, s) = team_app(cx);
    let (ws, root) = (s.ws.clone(), s.root.clone());
    cx.update_window(s.win, |_, w, cx| branch_dialog::open(&ws, root, w, cx))
        .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        assert!(w.try_find("branch-fetch").is_some(), "the dialog is open");
        assert!(w.try_find("branch-pull").is_none());
        assert!(w.try_find("branch-push").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_team_collection_leaves_pull_and_push_out_of_the_palette(cx: &mut TestAppContext) {
    let (_sb, s) = team_app(cx);
    assert!(crate::branch_shell::palette_rows(cx, &s, "pull from").is_empty());
    cx.simulate_keystrokes(s.win, "escape");
    frame(cx, s.win);
    assert!(crate::branch_shell::palette_rows(cx, &s, "push to").is_empty());
}

/// The branch the dialog's list marks checked out, and the picker's own idea of it.
fn shown_head(cx: &mut TestAppContext, r: &Rig) -> (Option<String>, Option<String>) {
    r.picker.read_with(cx, |p, _| {
        let row = p.branches().iter().find(|b| b.head);
        (
            row.and_then(|b| b.local.clone()),
            p.current().map(str::to_owned),
        )
    })
}

fn wait_shown_head(cx: &mut TestAppContext, r: &Rig, branch: &str) {
    let want = (Some(branch.to_owned()), Some(branch.to_owned()));
    wait_for(cx, "the list to show the real checked-out branch", |cx| {
        shown_head(cx, r) == want
    });
}

#[gpui_kit::test]
fn a_failed_push_refreshes_the_list_after_an_outside_switch(cx: &mut TestAppContext) {
    let r = rig(cx, Sandbox::new(), |_| {});
    assert_eq!(shown_head(cx, &r).0.as_deref(), Some("main"));
    r.sb.git(
        r.root(),
        &["switch", "-q", "-c", "other", "--track", "origin/main"],
    );
    write(r.root(), "d.txt", "d\n");
    r.sb.commit_all(r.root(), "local");
    advance(&r.sb, "c.txt", "c\n");
    push(cx, &r);
    assert!(notice_text(cx, &r).contains("rejected"), "the push failed");
    wait_shown_head(cx, &r, "other");
}

#[gpui_kit::test]
fn a_failed_pull_refreshes_the_list_after_an_outside_switch(cx: &mut TestAppContext) {
    let r = rig(cx, Sandbox::new(), |_| {});
    r.sb.git(r.root(), &["switch", "-q", "-c", "loose"]);
    pull(cx, &r);
    assert!(notice(cx, &r).is_some(), "the pull failed");
    wait_shown_head(cx, &r, "loose");
}
