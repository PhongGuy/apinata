//! Autosave: off by default, saves one delay after the last edit, and never overwrites a file
//! that changed on disk.

use crate::support;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use apinata_app::app::AppState;
use apinata_app::autosave::{AUTOSAVE_DELAY, Autosave};
use apinata_app::command::{Command, Edit, NoticeKind, Target};
use apinata_app::snapshot::DiskState;
use apinata_app::workspace::WorkspaceEvent;
use apinata_core::import::{RequestFormat, parse_request};
use gpui_kit::component::WindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext};
use support::*;

const ENV: &str = "name: dev\nvariables:\n  - name: host\n    value: x\n";

fn files() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
        ("environments/dev.yml", ENV),
    ])
}

fn saver(cx: &mut TestAppContext, w: &Watched, on: bool) -> Entity<Autosave> {
    let ws = w.ws.clone();
    let a = cx.new(|cx| Autosave::new(ws, None, cx));
    a.update(cx, |a, cx| a.set_enabled(on, cx));
    cx.run_until_parked();
    a
}

fn draft(cx: &mut TestAppContext, w: &Watched, to: &str) {
    w.ws.update(cx, |ws, cx| ws.send(&w.root, edit_url("a.yml", to), cx));
    cx.run_until_parked();
}

fn wait(cx: &mut TestAppContext, d: Duration) {
    cx.executor().advance_clock(d);
    cx.run_until_parked();
}

fn disk(dir: &tempfile::TempDir) -> String {
    std::fs::read_to_string(dir.path().join("a.yml")).unwrap()
}

fn state(cx: &mut TestAppContext, w: &Watched) -> (bool, DiskState) {
    let s = ws_snap(cx, &w.ws, &w.root);
    let a = &s.content.requests[&ip("a.yml")];
    (a.dirty, a.disk)
}

#[gpui_kit::test]
fn off_writes_nothing(cx: &mut TestAppContext) {
    let dir = files();
    let w = watching(cx, &dir, None);
    let _a = saver(cx, &w, false);
    draft(cx, &w, "http://x/draft");
    wait(cx, AUTOSAVE_DELAY * 5);
    assert!(disk(&dir).contains("http://x/0"));
    assert!(state(cx, &w).0);
}

#[gpui_kit::test]
fn on_saves_one_delay_after_the_last_edit(cx: &mut TestAppContext) {
    let dir = files();
    let w = watching(cx, &dir, None);
    let _a = saver(cx, &w, true);
    draft(cx, &w, "http://x/one");
    wait(cx, AUTOSAVE_DELAY - Duration::from_millis(400));
    draft(cx, &w, "http://x/two");
    wait(cx, AUTOSAVE_DELAY - Duration::from_millis(100));
    assert!(
        disk(&dir).contains("http://x/0"),
        "a new edit restarts the delay"
    );
    wait(cx, Duration::from_millis(200));
    assert!(disk(&dir).contains("http://x/two"), "{}", disk(&dir));
    assert_eq!(state(cx, &w), (false, DiskState::Present));
}

#[gpui_kit::test]
fn an_environment_edit_is_saved_too(cx: &mut TestAppContext) {
    let dir = files();
    let w = watching(cx, &dir, None);
    let _a = saver(cx, &w, true);
    let color = Command::Edit {
        target: Target::Environment("dev".into()),
        edit: Edit::Color(Some("#ff0000".into())),
    };
    w.ws.update(cx, |ws, cx| ws.send(&w.root, color, cx));
    cx.run_until_parked();
    wait(cx, AUTOSAVE_DELAY + Duration::from_millis(50));
    let env = std::fs::read_to_string(dir.path().join("environments/dev.yml")).unwrap();
    assert!(env.contains("#ff0000"), "{env}");
}

#[gpui_kit::test]
fn turning_on_saves_existing_edits_and_off_cancels(cx: &mut TestAppContext) {
    let dir = files();
    let w = watching(cx, &dir, None);
    let a = saver(cx, &w, false);
    draft(cx, &w, "http://x/early");
    wait(cx, AUTOSAVE_DELAY * 2);
    a.update(cx, |a, cx| a.set_enabled(true, cx));
    wait(cx, AUTOSAVE_DELAY / 2);
    a.update(cx, |a, cx| a.set_enabled(false, cx));
    wait(cx, AUTOSAVE_DELAY * 3);
    assert!(
        disk(&dir).contains("http://x/0"),
        "off cancels the pending save"
    );
    a.update(cx, |a, cx| a.set_enabled(true, cx));
    wait(cx, AUTOSAVE_DELAY + Duration::from_millis(50));
    assert!(disk(&dir).contains("http://x/early"));
}

#[gpui_kit::test]
fn enabling_again_keeps_the_pending_save(cx: &mut TestAppContext) {
    let dir = files();
    let w = watching(cx, &dir, None);
    let a = saver(cx, &w, true);
    draft(cx, &w, "http://x/draft");
    wait(cx, AUTOSAVE_DELAY - Duration::from_millis(400));
    a.update(cx, |a, cx| a.set_enabled(true, cx));
    wait(cx, Duration::from_millis(450));
    assert!(
        disk(&dir).contains("http://x/draft"),
        "no restart from a repeated enable"
    );
}

#[gpui_kit::test]
fn a_change_the_watcher_saw_is_never_overwritten(cx: &mut TestAppContext) {
    let dir = files();
    let w = watched(cx, &dir);
    let _a = saver(cx, &w, true);
    draft(cx, &w, "http://x/draft");
    let external = request("a", "http://x/external");
    std::fs::write(dir.path().join("a.yml"), &external).unwrap();
    wait_for(cx, "the changed-on-disk mark", |cx| {
        state(cx, &w).1 == DiskState::Changed
    });
    wait(cx, AUTOSAVE_DELAY * 3);
    assert_eq!(disk(&dir), external);
    assert_eq!(state(cx, &w), (true, DiskState::Changed));
}

#[gpui_kit::test]
fn a_change_after_load_but_before_the_save_is_never_overwritten(cx: &mut TestAppContext) {
    // No watcher: the file changes and nothing reloads before the autosave fires.
    let dir = files();
    let w = watching(cx, &dir, None);
    let _a = saver(cx, &w, true);
    draft(cx, &w, "http://x/draft");
    let external = request("a", "http://x/external");
    std::fs::write(dir.path().join("a.yml"), &external).unwrap();
    wait(cx, AUTOSAVE_DELAY + Duration::from_millis(50));
    assert_eq!(disk(&dir), external);
    assert_eq!(state(cx, &w), (true, DiskState::Changed));
    wait(cx, AUTOSAVE_DELAY * 3);
    assert_eq!(disk(&dir), external, "a later pass skips the changed file");
}

#[gpui_kit::test]
fn a_file_deleted_outside_the_app_is_never_recreated(cx: &mut TestAppContext) {
    let dir = files();
    let w = watched(cx, &dir);
    let _a = saver(cx, &w, true);
    let notices = Rc::new(RefCell::new(0usize));
    let seen = notices.clone();
    let _sub = cx.update(|cx| {
        cx.subscribe(&w.ws, move |_, e: &WorkspaceEvent, _| {
            if let WorkspaceEvent::Notices { notices, .. } = e {
                *seen.borrow_mut() += notices.len();
            }
        })
    });
    draft(cx, &w, "http://x/draft");
    std::fs::remove_file(dir.path().join("a.yml")).unwrap();
    wait_for(cx, "the removed mark", |cx| {
        state(cx, &w).1 == DiskState::Removed
    });
    wait(cx, AUTOSAVE_DELAY * 3);
    assert!(
        !dir.path().join("a.yml").exists(),
        "autosave recreated the file"
    );
    assert_eq!(state(cx, &w), (true, DiskState::Removed));
    assert_eq!(*notices.borrow(), 0, "no save was sent");
}

#[gpui_kit::test]
fn only_edits_move_the_edit_generation(cx: &mut TestAppContext) {
    let dir = files();
    let w = watching(cx, &dir, None);
    let generation = |cx: &mut TestAppContext| w.ws.read_with(cx, |ws, _| ws.edit_generation());
    let start = generation(cx);
    draft(cx, &w, "http://x/draft");
    assert_eq!(generation(cx), start + 1);
    w.ws.update(cx, |ws, cx| {
        ws.send(&w.root, Command::Save(Target::Request(ip("a.yml"))), cx)
    });
    cx.run_until_parked();
    assert_eq!(generation(cx), start + 1);
    let request = parse_request(Some(RequestFormat::Curl), "curl https://x.test/i")
        .expect("parsed")
        .request;
    let import = Command::ApplyImport {
        item: ip("a.yml"),
        request,
    };
    w.ws.update(cx, |ws, cx| ws.send(&w.root, import, cx));
    cx.run_until_parked();
    assert_eq!(generation(cx), start + 2);
}

fn set_autosave(cx: &mut TestAppContext, on: bool) {
    cx.update(|cx| {
        let s = cx.global::<AppState>().settings.clone();
        s.update(cx, |p, cx| p.update(cx, |s| s.autosave = on));
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_app_follows_the_setting(cx: &mut TestAppContext) {
    let col = files();
    let s = started(cx, &col);
    let send = |cx: &mut TestAppContext, to: &str| {
        s.ws.update(cx, |ws, cx| ws.send(&s.root, edit_url("a.yml", to), cx));
        cx.run_until_parked();
    };
    send(cx, "http://x/off");
    wait(cx, AUTOSAVE_DELAY * 3);
    assert!(disk(&col).contains("http://x/0"));
    set_autosave(cx, true);
    assert!(
        s.shell
            .read_with(cx, |sh, cx| sh.autosave().read(cx).enabled())
    );
    send(cx, "http://x/on");
    wait(cx, AUTOSAVE_DELAY + Duration::from_millis(50));
    assert!(disk(&col).contains("http://x/on"));
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_failed_autosave_toasts_once_and_waits_for_the_next_edit(cx: &mut TestAppContext) {
    let col = files();
    let _g = crate::support::ModeGuard::set(col.path(), 0o555);
    // Root writes through 0555, so the save would not fail; nothing to test then.
    if std::fs::write(col.path().join("probe"), b"").is_ok() {
        return;
    }
    let s = started(cx, &col);
    set_autosave(cx, true);
    let failures = Rc::new(RefCell::new(0usize));
    let seen = failures.clone();
    let _sub = cx.update(|cx| {
        cx.subscribe(&s.ws, move |_, e: &WorkspaceEvent, _| {
            if let WorkspaceEvent::Notices { notices, .. } = e {
                *seen.borrow_mut() += notices
                    .iter()
                    .filter(|n| matches!(n.kind, NoticeKind::Failed { .. }))
                    .count();
            }
        })
    });
    let toasts = |cx: &mut TestAppContext| {
        cx.update_window(s.win, |_, w, cx| w.notifications(cx).len())
            .unwrap()
    };
    let before = toasts(cx);
    s.ws.update(cx, |ws, cx| {
        ws.send(&s.root, edit_url("a.yml", "http://x/one"), cx)
    });
    cx.run_until_parked();
    wait(cx, AUTOSAVE_DELAY + Duration::from_millis(50));
    assert_eq!(
        (*failures.borrow(), toasts(cx) - before),
        (1, 1),
        "one failed save, one toast"
    );
    wait(cx, AUTOSAVE_DELAY * 5);
    assert_eq!(
        (*failures.borrow(), toasts(cx) - before),
        (1, 1),
        "no further save until the next edit"
    );
}
