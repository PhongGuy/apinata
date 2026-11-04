//! The branch picker model over a clone of a local bare repository.

use crate::git_support::{Sandbox, path_with, wait_gone, write};
use crate::support::*;

use std::cell::RefCell;
use std::rc::Rc;

use apinata_app::branch_picker::{Activity, BranchPicker, Notice, PickerEvent};
use apinata_app::command::{Command, Edit, Target};
use apinata_app::session::Session;
use apinata_app::workspace::Workspace;
use apinata_core::Branch;
use gpui_kit::{AppContext as _, Entity, TestAppContext};

struct Rig {
    sb: Sandbox,
    dir: tempfile::TempDir,
    ws: Entity<Workspace>,
    picker: Entity<BranchPicker>,
    events: Rc<RefCell<Vec<PickerEvent>>>,
}

/// A clone of the sandbox origin (on `main`) opened as a collection, with git on; `prep` runs
/// after the origin exists and before the clone.
fn rig(
    cx: &mut TestAppContext,
    sb: Sandbox,
    options: apinata_core::GitOptions,
    prep: impl FnOnce(&Sandbox),
) -> Rig {
    rig_with(cx, sb, options, apinata_app::actor::Faults::default(), prep)
}

/// `rig`, with actor faults on the collection it opens.
fn rig_with(
    cx: &mut TestAppContext,
    sb: Sandbox,
    options: apinata_core::GitOptions,
    faults: apinata_app::actor::Faults,
    prep: impl FnOnce(&Sandbox),
) -> Rig {
    init(cx);
    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    let bare = sb.origin();
    prep(&sb); // origin() runs once; extra branches are pushed from its seed work tree
    sb.git(
        sb.path(),
        &[
            "clone",
            "-q",
            bare.to_str().unwrap(),
            dir.path().to_str().unwrap(),
        ],
    );
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, Session::default());
    std::mem::forget(state);
    let ws = workspace(cx, &session);
    ws.update(cx, |w, cx| {
        w.set_open_faults(faults);
        w.set_git(Some(options), cx)
    });
    let root = dir.path().to_path_buf();
    ws.update(cx, |w, cx| w.open(root.clone(), cx));
    cx.run_until_parked();
    let picker = {
        let ws = ws.clone();
        cx.new(|cx| BranchPicker::new(ws, root, cx))
    };
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    cx.update(|cx| {
        cx.subscribe(&picker, move |_, e: &PickerEvent, _| {
            sink.borrow_mut().push(e.clone())
        })
        .detach()
    });
    let rig = Rig {
        sb,
        dir,
        ws,
        picker,
        events,
    };
    idle(cx, &rig);
    rig
}

fn idle(cx: &mut TestAppContext, r: &Rig) {
    wait_for(cx, "the picker to be idle", |cx| {
        r.picker
            .read_with(cx, |p, _| p.activity() == Activity::Idle)
    });
}

/// Each listed row as (local, upstream `remote/name`, head).
fn rows(cx: &mut TestAppContext, r: &Rig) -> Vec<(Option<String>, Option<String>, bool)> {
    r.picker.read_with(cx, |p, _| {
        p.branches()
            .iter()
            .map(|b| {
                let up = b
                    .upstream
                    .as_ref()
                    .map(|u| format!("{}/{}", u.remote, u.name));
                (b.local.clone(), up, b.head)
            })
            .collect()
    })
}

/// The branch names on offer: a local row's name or a remote-only row's branch name.
fn names(cx: &mut TestAppContext, r: &Rig) -> Vec<String> {
    rows(cx, r)
        .into_iter()
        .filter_map(|(local, up, _)| local.or_else(|| Some(up?.rsplit('/').next()?.to_owned())))
        .collect()
}

/// The remote-only row of `origin/<name>`.
fn find(cx: &mut TestAppContext, r: &Rig, name: &str) -> Branch {
    r.picker.read_with(cx, |p, _| {
        p.branches()
            .iter()
            .find(|b| b.local.is_none() && b.upstream.as_ref().is_some_and(|u| u.name == name))
            .cloned()
            .expect("listed")
    })
}

/// The row of local branch `name`.
fn find_local(cx: &mut TestAppContext, r: &Rig, name: &str) -> Branch {
    r.picker.read_with(cx, |p, _| {
        p.branches()
            .iter()
            .find(|b| b.local.as_deref() == Some(name))
            .cloned()
            .expect("listed")
    })
}

/// Pushes a new branch `name` from the seed work tree, optionally removing the collection file.
fn push_branch(sb: &Sandbox, name: &str, drop_collection: bool) {
    let seed = sb.path().join("seed");
    sb.git(&seed, &["switch", "-q", "-c", name, "main"]);
    if drop_collection {
        sb.git(&seed, &["rm", "-q", "opencollection.yml"]);
    } else {
        write(&seed, &format!("{name}.txt"), "x\n");
    }
    sb.commit_all(&seed, name);
    sb.git(&seed, &["push", "-q", "origin", name]);
}

#[gpui_kit::test]
fn it_lists_local_branches_then_untracked_remote_ones(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let options = sb.options();
    let r = rig(cx, sb, options, |_| {});
    let some = |s: &str| Some(s.to_owned());
    assert_eq!(
        rows(cx, &r),
        [
            (some("main"), some("origin/main"), true),
            (None, some("origin/feature"), false)
        ]
    );
    assert_eq!(
        r.picker
            .read_with(cx, |p, _| p.current().map(str::to_owned))
            .as_deref(),
        Some("main")
    );
}

#[gpui_kit::test]
fn fetch_finds_a_branch_pushed_after_the_clone(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let options = sb.options();
    let r = rig(cx, sb, options, |_| {});
    push_branch(&r.sb, "later", false);
    assert!(names(cx, &r).iter().all(|n| n != "later"));
    r.picker.update(cx, |p, cx| p.fetch(cx));
    idle(cx, &r);
    assert!(names(cx, &r).iter().any(|n| n == "later"));
    assert_eq!(r.picker.read_with(cx, |p, _| p.notice().cloned()), None);
}

#[gpui_kit::test]
fn switching_creates_a_tracking_branch_and_reports_it(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let options = sb.options();
    let r = rig(cx, sb, options, |_| {});
    let feature = find(cx, &r, "feature");
    r.picker.update(cx, |p, cx| p.switch(feature, cx));
    idle(cx, &r);
    assert_eq!(
        r.picker
            .read_with(cx, |p, _| p.current().map(str::to_owned))
            .as_deref(),
        Some("feature")
    );
    assert!(r.dir.path().join("b.txt").is_file());
    assert_eq!(
        *r.events.borrow(),
        [PickerEvent::Switched {
            branch: "feature".into()
        }]
    );
    assert_eq!(
        r.sb.git(
            r.dir.path(),
            &["rev-parse", "--abbrev-ref", "feature@{upstream}"]
        )
        .trim(),
        "origin/feature"
    );
}

#[gpui_kit::test]
fn a_local_branch_tracking_another_branch_switches_and_says_so(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let options = sb.options();
    let r = rig(cx, sb, options, |_| {});
    r.sb.git(
        r.dir.path(),
        &["branch", "--track", "feature", "origin/main"],
    );
    let feature = find(cx, &r, "feature");
    r.picker.update(cx, |p, cx| p.switch(feature, cx));
    idle(cx, &r);
    assert_eq!(
        r.picker
            .read_with(cx, |p, _| p.current().map(str::to_owned))
            .as_deref(),
        Some("feature")
    );
    assert_eq!(
        *r.events.borrow(),
        [PickerEvent::Switched {
            branch: "feature".into()
        }]
    );
    let notice = r.picker.read_with(cx, |p, _| p.notice().cloned());
    assert_eq!(
        notice.as_ref().map(Notice::text),
        Some("feature tracks origin/main, not origin/feature; its upstream was left as is.")
    );
    assert_eq!(
        r.sb.git(
            r.dir.path(),
            &["rev-parse", "--abbrev-ref", "feature@{upstream}"]
        )
        .trim(),
        "origin/main"
    );
}

#[gpui_kit::test]
fn a_dirty_tree_refuses_the_switch_and_changes_nothing(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let options = sb.options();
    let r = rig(cx, sb, options, |_| {});
    write(r.dir.path(), "scratch.txt", "mine\n");
    let feature = find(cx, &r, "feature");
    r.picker.update(cx, |p, cx| p.switch(feature, cx));
    idle(cx, &r);
    assert_eq!(
        r.picker.read_with(cx, |p, _| p.notice().cloned()),
        Some(Notice::Dirty)
    );
    assert_eq!(
        r.picker
            .read_with(cx, |p, _| p.current().map(str::to_owned))
            .as_deref(),
        Some("main")
    );
    assert!(r.events.borrow().is_empty());
    assert_eq!(
        std::fs::read_to_string(r.dir.path().join("scratch.txt")).unwrap(),
        "mine\n"
    );
}

#[gpui_kit::test]
fn a_branch_without_the_collection_file_closes_the_collection(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let options = sb.options();
    let r = rig(cx, sb, options, |sb| push_branch(sb, "nocol", true));
    let nocol = find(cx, &r, "nocol");
    r.picker.update(cx, |p, cx| p.switch(nocol, cx));
    idle(cx, &r);
    assert!(!r.dir.path().join("opencollection.yml").exists());
    assert!(r.ws.read_with(cx, |w, _| w.collections().is_empty()));
    let events = r.events.borrow();
    assert!(
        matches!(events.as_slice(), [PickerEvent::CollectionClosed { branch, message }]
        if branch == "nocol" && message.contains("nocol")),
        "{events:?}"
    );
}

#[gpui_kit::test]
fn a_switch_over_a_held_head_lock_says_git_is_busy(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let options = sb.options();
    let r = rig(cx, sb, options, |_| {});
    let held = r.dir.path().join(".git/HEAD.lock");
    std::fs::write(&held, "").unwrap();
    let feature = find(cx, &r, "feature");
    r.picker.update(cx, |p, cx| p.switch(feature, cx));
    idle(cx, &r);
    let notice = r.picker.read_with(cx, |p, _| p.notice().cloned());
    assert!(
        matches!(&notice, Some(Notice::Failed(t))
            if t.starts_with("git is busy") && t.contains(".git/HEAD.lock")),
        "{notice:?}"
    );
    assert!(held.exists(), "HEAD.lock was deleted");
}

#[gpui_kit::test]
fn fetching_without_a_remote_says_so(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    init(cx);
    cx.executor().allow_parking();
    let solo = sb.repo("solo");
    write(&solo, "opencollection.yml", ROOT);
    sb.commit_all(&solo, "init");
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, Session::default());
    let ws = workspace(cx, &session);
    let options = sb.options();
    ws.update(cx, |w, cx| w.set_git(Some(options), cx));
    let picker = {
        let (ws, root) = (ws.clone(), solo.clone());
        cx.new(|cx| BranchPicker::new(ws, root, cx))
    };
    wait_for(cx, "listing", |cx| {
        picker.read_with(cx, |p, _| p.activity() == Activity::Idle)
    });
    picker.update(cx, |p, cx| p.fetch(cx));
    wait_for(cx, "the fetch", |cx| {
        picker.read_with(cx, |p, _| p.activity() == Activity::Idle)
    });
    let notice = picker.read_with(cx, |p, _| p.notice().cloned());
    assert!(matches!(notice, Some(Notice::Failed(t)) if t.contains("no remote")));
}

#[gpui_kit::test]
fn a_cancelled_fetch_stops_git_and_shows_no_error(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let bin = stalling_git(&sb, "fetch");
    let options = sb.options().env("PATH", path_with(&bin));
    let r = rig(cx, sb, options, |_| {});
    r.picker.update(cx, |p, cx| p.fetch(cx));
    wait_for(cx, "git to stall", |_| bin.join("pid").is_file());
    r.picker.update(cx, |p, _| p.cancel_fetch());
    idle(cx, &r);
    assert_eq!(r.picker.read_with(cx, |p, _| p.notice().cloned()), None);
    assert!(wait_gone(sleeper_pid(&bin)));
}

#[gpui_kit::test]
fn a_collection_with_an_unsaved_edit_refuses_the_switch(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let options = sb.options();
    let r = rig(cx, sb, options, |_| {});
    let head = r.sb.git(r.dir.path(), &["rev-parse", "HEAD"]);
    let root = r.dir.path().to_path_buf();
    r.ws.update(cx, |w, cx| {
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
    assert!(
        !r.ws
            .read_with(cx, |w, cx| w.unsaved(Some(&root), cx))
            .is_empty()
    );
    let feature = find(cx, &r, "feature");
    r.picker.update(cx, |p, cx| p.switch(feature, cx));
    idle(cx, &r);
    assert_eq!(
        r.picker.read_with(cx, |p, _| p.notice().cloned()),
        Some(Notice::Unsaved)
    );
    assert_eq!(r.sb.git(r.dir.path(), &["rev-parse", "HEAD"]), head);
    assert_eq!(
        r.picker
            .read_with(cx, |p, _| p.current().map(str::to_owned))
            .as_deref(),
        Some("main")
    );
    assert!(r.events.borrow().is_empty());
    assert!(!r.ws.read_with(cx, |w, _| w.collections().is_empty()));
}

fn edit_docs(r: &Rig, cx: &mut TestAppContext) -> Option<u64> {
    let root = r.dir.path().to_path_buf();
    r.ws.update(cx, |w, cx| {
        w.send(
            &root,
            Command::Edit {
                target: Target::Collection,
                edit: Edit::Docs("draft".into()),
            },
            cx,
        )
    })
}

/// Starts the switch to `name` under a latched git and waits until git is held.
fn start_held_switch(cx: &mut TestAppContext, r: &Rig, bin: &std::path::Path, name: &str) {
    let branch = find(cx, r, name);
    r.picker.update(cx, |p, cx| p.switch(branch, cx));
    wait_for(cx, "git to reach the checkout", |_| {
        bin.join("reached").is_file()
    });
}

#[gpui_kit::test]
fn edits_are_refused_while_a_switch_runs(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let bin = latched_git(&sb);
    let options = sb.options().env("PATH", path_with(&bin));
    let r = rig(cx, sb, options, |_| {});
    start_held_switch(cx, &r, &bin, "feature");
    let root = r.dir.path().to_path_buf();
    assert!(r.ws.read_with(cx, |w, _| w.is_switching(&root)));
    assert_eq!(edit_docs(&r, cx), None, "no edit lands mid-switch");
    assert!(
        r.ws.read_with(cx, |w, cx| w.unsaved(Some(&root), cx))
            .is_empty()
    );
    release_git(&bin);
    idle(cx, &r);
    assert!(!r.ws.read_with(cx, |w, _| w.is_switching(&root)));
    assert!(edit_docs(&r, cx).is_some(), "edits work after the switch");
}

#[gpui_kit::test]
fn the_open_tab_shows_the_new_branch_as_soon_as_the_switch_lands(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let options = sb.options();
    let r = rig(cx, sb, options, |sb| {
        let seed = sb.path().join("seed");
        write(&seed, "r.yml", &request("r", "http://feature/"));
        sb.commit_all(&seed, "r on feature");
        sb.git(&seed, &["push", "-q", "origin", "feature"]);
        sb.git(&seed, &["switch", "-q", "main"]);
        write(&seed, "r.yml", &request("r", "http://main/"));
        sb.commit_all(&seed, "r on main");
        sb.git(&seed, &["push", "-q", "origin", "main"]);
    });
    let root = r.dir.path().to_path_buf();
    r.ws.update(cx, |w, cx| w.open_item(&root, ip("r.yml"), false, cx));
    let url = |cx: &mut TestAppContext| {
        r.ws.read_with(cx, |w, cx| {
            w.request(&root, &ip("r.yml"), cx).map(|v| v.url.clone())
        })
    };
    assert_eq!(url(cx).as_deref(), Some("http://main/"));
    let feature = find(cx, &r, "feature");
    r.picker.update(cx, |p, cx| p.switch(feature, cx));
    idle(cx, &r);
    // No further pumping: the tab must already hold the new branch's content.
    assert_eq!(url(cx).as_deref(), Some("http://feature/"));
}

#[gpui_kit::test]
fn unsaved_work_that_appears_mid_switch_keeps_the_collection_open(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let bin = latched_git(&sb);
    let options = sb.options().env("PATH", path_with(&bin));
    let r = rig(cx, sb, options, |sb| push_branch(sb, "nocol", true));
    start_held_switch(cx, &r, &bin, "nocol");
    let root = r.dir.path().to_path_buf();
    // A draft restored as unsaved is not an edit of an open tab, so the lock lets it through.
    let snapshot = apinata_core::DraftSnapshot::from_json(
        r#"{"info":{"name":"Late","type":"http"},"http":{}}"#,
    )
    .unwrap();
    r.ws.update(cx, |w, cx| {
        w.send(
            &root,
            Command::Tree(apinata_app::command::TreeOp::CreateDraftFrom {
                parent: apinata_core::ItemPath::root(),
                snapshot,
                unsaved: true,
            }),
            cx,
        )
    });
    cx.run_until_parked();
    assert!(
        !r.ws
            .read_with(cx, |w, cx| w.unsaved(Some(&root), cx))
            .is_empty()
    );
    release_git(&bin);
    idle(cx, &r);
    assert_eq!(
        r.picker.read_with(cx, |p, _| p.notice().cloned()),
        Some(Notice::KeptOpen)
    );
    assert!(!r.ws.read_with(cx, |w, _| w.collections().is_empty()));
    assert!(r.events.borrow().is_empty());
    assert!(!r.ws.read_with(cx, |w, _| w.is_switching(&root)));
}

#[gpui_kit::test]
fn tree_changes_are_refused_while_a_switch_runs(cx: &mut TestAppContext) {
    use apinata_app::command::TreeOp;
    let sb = Sandbox::new();
    let bin = latched_git(&sb);
    let options = sb.options().env("PATH", path_with(&bin));
    let r = rig(cx, sb, options, |_| {});
    start_held_switch(cx, &r, &bin, "feature");
    let root = r.dir.path().to_path_buf();
    let ops = [
        TreeOp::Rename {
            item: ip("r.yml"),
            name: "x".into(),
            rename_file: true,
        },
        TreeOp::Delete { item: ip("r.yml") },
        TreeOp::CreateFolder {
            parent: apinata_core::ItemPath::root(),
            name: "f".into(),
        },
        TreeOp::CreateDraft {
            parent: apinata_core::ItemPath::root(),
            kind: apinata_core::RequestKind::Http,
        },
    ];
    for op in ops {
        let sent =
            r.ws.update(cx, |w, cx| w.send(&root, Command::Tree(op), cx));
        assert_eq!(sent, None);
    }
    release_git(&bin);
    idle(cx, &r);
}

#[gpui_kit::test]
fn the_tab_is_reloaded_by_the_time_the_switch_is_announced(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let options = sb.options();
    let r = rig(cx, sb, options, |sb| {
        let seed = sb.path().join("seed");
        write(&seed, "r.yml", &request("r", "http://feature/"));
        sb.commit_all(&seed, "r on feature");
        sb.git(&seed, &["push", "-q", "origin", "feature"]);
        sb.git(&seed, &["switch", "-q", "main"]);
        write(&seed, "r.yml", &request("r", "http://main/"));
        sb.commit_all(&seed, "r on main");
        sb.git(&seed, &["push", "-q", "origin", "main"]);
    });
    let root = r.dir.path().to_path_buf();
    r.ws.update(cx, |w, cx| w.open_item(&root, ip("r.yml"), false, cx));
    let seen = Rc::new(RefCell::new(None));
    let (sink, ws, at) = (seen.clone(), r.ws.clone(), root.clone());
    cx.update(|cx| {
        cx.subscribe(&r.picker, move |_, _: &PickerEvent, cx| {
            *sink.borrow_mut() = ws
                .read(cx)
                .request(&at, &ip("r.yml"), cx)
                .map(|v| v.url.clone());
        })
        .detach()
    });
    let feature = find(cx, &r, "feature");
    r.picker.update(cx, |p, cx| p.switch(feature, cx));
    idle(cx, &r);
    assert_eq!(seen.borrow().as_deref(), Some("http://feature/"));
}

#[gpui_kit::test]
fn a_reload_that_never_answers_releases_the_collection_with_a_notice(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let bin = latched_git(&sb);
    let options = sb.options().env("PATH", path_with(&bin));
    let stall = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let faults = apinata_app::actor::Faults {
        stall: Some(stall.clone()),
        ..Default::default()
    };
    let r = rig_with(cx, sb, options, faults, |_| {});
    let root = r.dir.path().to_path_buf();
    start_held_switch(cx, &r, &bin, "feature");
    // The actor goes quiet while git is held, so the reload sent afterwards is never applied.
    stall.store(true, std::sync::atomic::Ordering::SeqCst);
    release_git(&bin);
    // The label moves once git is done, which is just before the reload is sent.
    wait_for(cx, "git to finish", |cx| {
        r.picker
            .read_with(cx, |p, _| p.current() == Some("feature"))
    });
    cx.executor()
        .advance_clock(apinata_app::branch_picker::RELOAD_BOUND * 2);
    idle(cx, &r);
    let notice = r.picker.read_with(cx, |p, _| p.notice().cloned());
    assert!(matches!(notice, Some(Notice::Failed(t)) if t.contains("not reloaded")));
    assert!(!r.ws.read_with(cx, |w, _| w.is_switching(&root)));
    assert!(r.events.borrow().is_empty());
    stall.store(false, std::sync::atomic::Ordering::SeqCst);
}

#[gpui_kit::test]
fn a_local_only_branch_is_listed_and_switches_without_touching_its_upstream(
    cx: &mut TestAppContext,
) {
    let sb = Sandbox::new();
    let options = sb.options();
    let r = rig(cx, sb, options, |_| {});
    r.sb.git(r.dir.path(), &["branch", "topic", "main"]);
    r.picker.update(cx, |p, cx| p.fetch(cx));
    idle(cx, &r);
    assert!(names(cx, &r).iter().any(|n| n == "topic"));
    let topic = find_local(cx, &r, "topic");
    r.picker.update(cx, |p, cx| p.switch(topic, cx));
    idle(cx, &r);
    assert_eq!(
        r.picker
            .read_with(cx, |p, _| p.current().map(str::to_owned))
            .as_deref(),
        Some("topic")
    );
    assert_eq!(r.picker.read_with(cx, |p, _| p.notice().cloned()), None);
    assert_eq!(
        *r.events.borrow(),
        [PickerEvent::Switched {
            branch: "topic".into()
        }]
    );
    let config = r.sb.git(r.dir.path(), &["config", "--local", "--list"]);
    assert!(!config.contains("branch.topic."), "{config}");
}

// The dialog stays open after a kept-open switch, so its rows must show the branch HEAD is on.
#[gpui_kit::test]
fn a_kept_open_switch_relists_with_the_new_branch_checked_out(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let bin = latched_git(&sb);
    let options = sb.options().env("PATH", path_with(&bin));
    let r = rig(cx, sb, options, |sb| push_branch(sb, "nocol", true));
    start_held_switch(cx, &r, &bin, "nocol");
    let root = r.dir.path().to_path_buf();
    let snapshot = apinata_core::DraftSnapshot::from_json(
        r#"{"info":{"name":"Late","type":"http"},"http":{}}"#,
    )
    .unwrap();
    r.ws.update(cx, |w, cx| {
        w.send(
            &root,
            Command::Tree(apinata_app::command::TreeOp::CreateDraftFrom {
                parent: apinata_core::ItemPath::root(),
                snapshot,
                unsaved: true,
            }),
            cx,
        )
    });
    cx.run_until_parked();
    release_git(&bin);
    idle(cx, &r);
    assert_eq!(
        r.picker.read_with(cx, |p, _| p.notice().cloned()),
        Some(Notice::KeptOpen)
    );
    let some = |s: &str| Some(s.to_owned());
    wait_for(cx, "the rows to show nocol checked out", |cx| {
        rows(cx, &r)
            == [
                (some("main"), some("origin/main"), false),
                (some("nocol"), some("origin/nocol"), true),
                (None, some("origin/feature"), false),
            ]
    });
}

/// A git that, while `hold` exists beside it, blocks the branch listing (marking `listing`)
/// until `list-go` appears and marks `listed` once that listing ran; `fetch` always blocks
/// (marking `fetching`) until `fetch-go`.
fn slow_listing_git(sb: &Sandbox) -> std::path::PathBuf {
    let body = format!(
        "#!/bin/sh\ndir=${{0%/*}}\ncase \"$*\" in\n\
         *'%(HEAD)'*) if [ -e \"$dir/hold\" ]; then : > \"$dir/listing\"; \
         while [ ! -e \"$dir/list-go\" ]; do sleep 0.02; done; \
         '{real}' \"$@\"; rc=$?; : > \"$dir/listed\"; exit $rc; fi ;;\n\
         *' fetch '*) : > \"$dir/fetching\"; \
         while [ ! -e \"$dir/fetch-go\" ]; do sleep 0.02; done ;;\n\
         esac\nexec '{real}' \"$@\"\n",
        real = crate::git_support::real_git().display()
    );
    sb.fake_git("slow-listing", &body)
}

// The listing a switch starts may end after a fetch began; it must not end that fetch.
#[gpui_kit::test]
fn a_late_listing_from_a_switch_never_ends_a_running_fetch(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let bin = slow_listing_git(&sb);
    let options = sb.options().env("PATH", path_with(&bin));
    let r = rig(cx, sb, options, |_| {});
    std::fs::write(bin.join("hold"), "").unwrap();
    let feature = find(cx, &r, "feature");
    r.picker.update(cx, |p, cx| p.switch(feature, cx));
    idle(cx, &r);
    wait_for(cx, "the post-switch listing to start", |_| {
        bin.join("listing").is_file()
    });
    std::fs::remove_file(bin.join("hold")).unwrap();
    r.picker.update(cx, |p, cx| p.fetch(cx));
    wait_for(cx, "git to reach the fetch", |_| {
        bin.join("fetching").is_file()
    });
    std::fs::write(bin.join("list-go"), "").unwrap();
    wait_for(cx, "the stale listing to run", |_| {
        bin.join("listed").is_file()
    });
    // Its result crosses a thread and a channel before the picker sees it.
    for _ in 0..10 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        cx.run_until_parked();
    }
    assert_eq!(
        r.picker.read_with(cx, |p, _| p.activity()),
        Activity::Fetching
    );
    std::fs::write(bin.join("fetch-go"), "").unwrap();
    idle(cx, &r);
    assert!(names(cx, &r).iter().any(|n| n == "feature"));
}
