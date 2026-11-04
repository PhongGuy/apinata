use apinata_app::persisted::{Persisted, SAVE_DELAY, SaveFailed};
use gpui_kit::{AppContext, Entity, TestAppContext};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

#[derive(serde::Serialize, serde::Deserialize, Default, Clone, Debug, PartialEq)]
struct Doc {
    n: u32,
}

fn doc(cx: &mut TestAppContext, path: PathBuf) -> Entity<Persisted<Doc>> {
    cx.new(|_| Persisted::new(Doc::default(), path))
}

fn set(cx: &mut TestAppContext, p: &Entity<Persisted<Doc>>, n: u32) {
    p.update(cx, |p, cx| p.update(cx, |d| d.n = n));
}

#[gpui_kit::test]
fn edits_within_the_delay_cost_one_write(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a/doc.json");
    let p = doc(cx, path.clone());
    set(cx, &p, 1);
    set(cx, &p, 2);
    cx.executor()
        .advance_clock(SAVE_DELAY - Duration::from_millis(100));
    cx.run_until_parked();
    assert!(!path.exists());
    cx.executor().advance_clock(Duration::from_millis(100));
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "{\n  \"n\": 2\n}\n"
    );
}

#[gpui_kit::test]
fn flush_writes_without_waiting_for_the_delay(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("doc.json");
    let p = doc(cx, path.clone());
    set(cx, &p, 3);
    let task = p.update(cx, |p, cx| p.flush(cx));
    cx.run_until_parked();
    drop(task);
    assert!(std::fs::read_to_string(&path).unwrap().contains("3"));
    assert_eq!(p.read_with(cx, |p, _| p.get().clone()), Doc { n: 3 });
}

#[gpui_kit::test]
fn an_older_write_never_overwrites_a_newer_one(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("doc.json");
    let p = doc(cx, path.clone());
    set(cx, &p, 1);
    let first = p.update(cx, |p, cx| p.flush(cx));
    set(cx, &p, 2);
    let second = p.update(cx, |p, cx| p.flush(cx));
    cx.run_until_parked();
    drop((first, second));
    assert!(std::fs::read_to_string(&path).unwrap().contains("2"));
}

#[gpui_kit::test]
fn a_failed_write_emits_save_failed(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("blocker"), "a file, not a directory").unwrap();
    let p = doc(cx, dir.path().join("blocker/doc.json"));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    cx.update(|cx| {
        cx.subscribe(&p, move |_, e: &SaveFailed, _| {
            sink.borrow_mut().push(e.0.clone())
        })
        .detach()
    });
    set(cx, &p, 1);
    let task = p.update(cx, |p, cx| p.flush(cx));
    cx.run_until_parked();
    drop(task);
    assert_eq!(seen.borrow().len(), 1);
    assert!(!seen.borrow()[0].is_empty());
    assert_eq!(p.read_with(cx, |p, _| p.get().n), 1);
}

#[gpui_kit::test]
fn discard_cancels_a_waiting_save_and_removes_the_file(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("doc.json");
    let p = doc(cx, path.clone());
    set(cx, &p, 1);
    let task = p.update(cx, |p, cx| p.flush(cx));
    cx.run_until_parked();
    drop(task);
    assert!(path.exists());
    set(cx, &p, 2);
    p.update(cx, |p, cx| p.discard(cx)).detach();
    cx.run_until_parked();
    cx.executor().advance_clock(SAVE_DELAY * 2);
    cx.run_until_parked();
    assert!(!path.exists());
    set(cx, &p, 3);
    cx.executor().advance_clock(SAVE_DELAY * 2);
    cx.run_until_parked();
    assert!(!path.exists(), "a discarded value never writes again");
    assert!(!p.read_with(cx, |p, _| p.is_written()));
}

#[gpui_kit::test]
fn a_write_after_the_discard_landed_cannot_recreate_the_file(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("doc.json");
    let p = doc(cx, path.clone());
    set(cx, &p, 1);
    p.update(cx, |p, cx| p.discard(cx)).detach();
    cx.run_until_parked();
    // The discard has run, so only the marker can stop this flush from writing.
    let write = p.update(cx, |p, cx| p.flush(cx));
    cx.run_until_parked();
    drop(write);
    assert!(!path.exists());
    assert_eq!(p.update(cx, |p, _| p.flush_now()).ok(), Some(()));
    assert!(!path.exists(), "flush_now honours the marker too");
}

#[gpui_kit::test]
fn is_written_follows_the_latest_value(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let p = doc(cx, dir.path().join("doc.json"));
    assert!(
        p.read_with(cx, |p, _| p.is_written()),
        "nothing changed yet"
    );
    set(cx, &p, 1);
    assert!(!p.read_with(cx, |p, _| p.is_written()));
    cx.executor().advance_clock(SAVE_DELAY);
    cx.run_until_parked();
    assert!(p.read_with(cx, |p, _| p.is_written()));
}

#[gpui_kit::test]
fn flush_now_writes_on_the_calling_thread(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("doc.json");
    let p = doc(cx, path.clone());
    set(cx, &p, 5);
    p.update(cx, |p, _| p.flush_now()).unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "{\n  \"n\": 5\n}\n"
    );
    assert!(p.read_with(cx, |p, _| p.is_written()));
    cx.executor().advance_clock(SAVE_DELAY * 2);
    cx.run_until_parked();
    assert!(std::fs::read_to_string(&path).unwrap().contains('5'));
}

#[gpui_kit::test]
fn discard_takes_effect_before_its_task_runs(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("doc.json");
    let p = doc(cx, path.clone());
    set(cx, &p, 1);
    p.update(cx, |p, _| p.flush_now()).unwrap();
    assert!(path.exists());
    set(cx, &p, 2);
    // The returned task is dropped unpolled; the discard must not depend on it.
    drop(p.update(cx, |p, cx| p.discard(cx)));
    assert!(!p.read_with(cx, |p, _| p.is_written()));
    p.update(cx, |p, _| p.flush_now()).unwrap();
    assert!(!path.exists(), "flush_now after discard writes nothing");
}

#[gpui_kit::test]
fn an_update_after_discard_never_writes(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("doc.json");
    let p = doc(cx, path.clone());
    set(cx, &p, 1);
    drop(p.update(cx, |p, cx| p.discard(cx)));
    set(cx, &p, 2);
    p.update(cx, |p, _| p.flush_now()).unwrap();
    assert!(!path.exists());
    assert!(!p.read_with(cx, |p, _| p.is_written()));
    cx.executor().advance_clock(SAVE_DELAY * 2);
    cx.run_until_parked();
    assert!(!path.exists(), "no save was scheduled by the update");
}
