//! A runner's options are part of the saved session: they come back when the app restarts,
//! per runner target, and nothing is written into the collection.

use crate::support;

use apinata_app::persist::load_json;
use apinata_app::runner_model::RunnerModel;
use apinata_app::runner_view::RunnerView;
use apinata_app::session::{CollectionSession, SavedRunnerOptions, Session};
use apinata_core::ItemPath;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use std::path::{Path, PathBuf};
use support::*;

struct Open {
    ws: Entity<apinata_app::workspace::Workspace>,
    win: AnyWindowHandle,
    view: Entity<RunnerView>,
    model: Entity<RunnerModel>,
}

fn collection_with_folder() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://127.0.0.1:9/a")),
        ("sub/folder.yml", &folder("sub")),
        ("sub/b.yml", &request("b", "http://127.0.0.1:9/b")),
    ])
}

// Opens a fresh workspace over `start` and a runner view for `target`, as a restart would.
fn open_with(
    cx: &mut TestAppContext,
    col: &tempfile::TempDir,
    state: &tempfile::TempDir,
    start: Session,
    target: &str,
) -> Open {
    support::init(cx);
    let session = session_at(cx, state, start);
    let ws = workspace(cx, &session);
    let root = col.path().to_path_buf();
    ws.update(cx, |w, cx| w.open(root.clone(), cx));
    cx.run_until_parked();
    let (shared, target) = (ws.clone(), ip(target));
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |_, cx| {
                cx.new(|cx| RunnerView::new(shared, root, target, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    let model = view.read_with(cx, |v, _| v.model().clone());
    Open {
        ws,
        win,
        view,
        model,
    }
}

fn session_of(cx: &mut TestAppContext, o: &Open) -> Session {
    let session = o.ws.read_with(cx, |w, _| w.session().clone());
    session_value(cx, &session)
}

fn set_everything(cx: &mut TestAppContext, o: &Open, data: &Path) {
    let data = data.to_path_buf();
    o.model.update(cx, |m, cx| {
        m.set_bail(true, cx);
        m.set_delay_ms(120, cx);
        m.set_retries(3, cx);
        m.set_retry_delay_ms(250, cx);
        m.set_retry_unsafe(true, cx);
        m.set_iteration_count(Some(4), cx);
        m.set_data_file(Some(data), cx);
    });
}

fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push((p.clone(), std::fs::read(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

fn data_label(cx: &mut TestAppContext, o: &Open) -> Option<String> {
    frame(cx, o.win);
    cx.update_window(o.win, |_, w, _| {
        w.try_find("run-data-name")
            .and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

#[gpui_kit::test]
fn every_saved_option_comes_back_after_a_restart(cx: &mut TestAppContext) {
    let col = collection_with_folder();
    let data_dir = write_tree(&[("rows.csv", "x\n1\n2\n")]);
    let data = data_dir.path().join("rows.csv");
    let (s1, s2) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let first = open_with(cx, &col, &s1, Session::default(), "");
    set_everything(cx, &first, &data);
    let saved = session_of(cx, &first);

    let second = open_with(cx, &col, &s2, saved, "");
    second.model.read_with(cx, |m, _| {
        assert!(m.bail() && m.retry_unsafe());
        assert_eq!(
            (m.delay_ms(), m.retries(), m.retry_delay_ms()),
            (120, 3, 250)
        );
        assert_eq!(m.iteration_count(), Some(4));
        assert_eq!(m.data_file(), Some(data.as_path()));
        assert!(!m.data_file_missing());
    });
    let numbers = second.view.read_with(cx, |v, cx| v.number_field_texts(cx));
    assert_eq!(numbers, Some(["120".into(), "4".into()]));
    let retries = second.view.read_with(cx, |v, cx| v.retry_field_texts(cx));
    assert_eq!(retries, Some(["3".into(), "250".into()]));
    assert_eq!(data_label(cx, &second).as_deref(), Some("rows.csv"));
    // Restoring is not an edit, so the new session holds what it was given and nothing more.
    assert_eq!(session_of(cx, &second).collections.len(), 1);
}

#[gpui_kit::test]
fn without_saved_options_the_runner_shows_defaults(cx: &mut TestAppContext) {
    let col = collection_with_folder();
    let state = tempfile::tempdir().unwrap();
    let o = open_with(cx, &col, &state, Session::default(), "");
    o.model.read_with(cx, |m, _| {
        assert!(!m.bail() && !m.retry_unsafe());
        assert_eq!((m.delay_ms(), m.retries(), m.retry_delay_ms()), (0, 0, 0));
        assert_eq!((m.iteration_count(), m.data_file()), (None, None));
    });
    assert!(session_of(cx, &o).collections.is_empty());
}

#[gpui_kit::test]
fn saving_and_restoring_never_touch_the_collection_directory(cx: &mut TestAppContext) {
    let col = collection_with_folder();
    let before = snapshot(col.path());
    let data_dir = write_tree(&[("rows.csv", "x\n1\n")]);
    let (s1, s2) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let first = open_with(cx, &col, &s1, Session::default(), "");
    set_everything(cx, &first, &data_dir.path().join("rows.csv"));
    let saved = session_of(cx, &first);
    let _second = open_with(cx, &col, &s2, saved, "");
    assert_eq!(snapshot(col.path()), before);
}

#[gpui_kit::test]
fn an_unreadable_session_file_gives_a_runner_with_defaults(cx: &mut TestAppContext) {
    let col = collection_with_folder();
    let state = tempfile::tempdir().unwrap();
    let path = state.path().join("session.json");
    std::fs::write(&path, "{ not json").unwrap();
    let loaded: Session = load_json(&path).value;
    let o = open_with(cx, &col, &state, loaded, "");
    o.model.read_with(cx, |m, _| {
        assert_eq!((m.delay_ms(), m.retries(), m.bail()), (0, 0, false));
        assert_eq!(m.data_file(), None);
    });
}

#[gpui_kit::test]
fn a_missing_data_file_is_kept_and_marked(cx: &mut TestAppContext) {
    let col = collection_with_folder();
    let state = tempfile::tempdir().unwrap();
    let gone = state.path().join("gone.csv");
    let mut start = Session::default();
    start.collections.insert(
        col.path().to_path_buf(),
        CollectionSession {
            runners: [(
                String::new(),
                SavedRunnerOptions {
                    data_file: Some(gone.clone()),
                    ..Default::default()
                },
            )]
            .into(),
            ..Default::default()
        },
    );
    let o = open_with(cx, &col, &state, start, "");
    o.model.read_with(cx, |m, _| {
        assert_eq!(m.data_file(), Some(gone.as_path()));
        assert!(m.data_file_missing());
    });
    assert_eq!(data_label(cx, &o).as_deref(), Some("gone.csv (missing)"));
}

#[gpui_kit::test]
fn a_folder_runner_and_the_collection_runner_are_saved_apart(cx: &mut TestAppContext) {
    let col = collection_with_folder();
    let (s1, s2) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let whole = open_with(cx, &col, &s1, Session::default(), "");
    whole.model.update(cx, |m, cx| m.set_retries(2, cx));
    let saved = session_of(cx, &whole);
    let folder = open_with(cx, &col, &s2, saved, "sub");
    folder
        .model
        .read_with(cx, |m, _| assert_eq!(m.retries(), 0));
    folder.model.update(cx, |m, cx| m.set_delay_ms(50, cx));
    let both = session_of(cx, &folder);
    let runners = &both.collections[col.path()].runners;
    assert_eq!(runners.keys().collect::<Vec<_>>(), ["", "sub"]);
    assert_eq!((runners[""].retries, runners[""].delay_ms), (2, 0));
    assert_eq!((runners["sub"].retries, runners["sub"].delay_ms), (0, 50));
}

#[gpui_kit::test]
fn returning_every_option_to_its_default_removes_the_entry(cx: &mut TestAppContext) {
    let col = collection_with_folder();
    let state = tempfile::tempdir().unwrap();
    let o = open_with(cx, &col, &state, Session::default(), "");
    o.model.update(cx, |m, cx| m.set_bail(true, cx));
    assert_eq!(session_of(cx, &o).collections[col.path()].runners.len(), 1);
    o.model.update(cx, |m, cx| m.set_bail(false, cx));
    assert!(
        session_of(cx, &o).collections[col.path()]
            .runners
            .is_empty()
    );
}

#[gpui_kit::test]
fn a_runner_of_a_selection_neither_saves_nor_restores(cx: &mut TestAppContext) {
    let col = collection_with_folder();
    let state = tempfile::tempdir().unwrap();
    let mut start = Session::default();
    start.collections.insert(
        col.path().to_path_buf(),
        CollectionSession {
            runners: [(
                String::new(),
                SavedRunnerOptions {
                    retries: 5,
                    ..Default::default()
                },
            )]
            .into(),
            ..Default::default()
        },
    );
    support::init(cx);
    let session = session_at(cx, &state, start);
    let ws = workspace(cx, &session);
    let root = col.path().to_path_buf();
    ws.update(cx, |w, cx| w.open(root.clone(), cx));
    let shared = ws.clone();
    let (_win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |_, cx| {
                let items = vec![ItemPath::new("a.yml").unwrap()];
                cx.new(|cx| RunnerView::for_items(shared, root, items, cx))
            })
        })
        .unwrap();
    let model = view.read_with(cx, |v, _| v.model().clone());
    model.read_with(cx, |m, _| assert_eq!(m.retries(), 0));
    model.update(cx, |m, cx| m.set_delay_ms(9, cx));
    let after = session_value(cx, &session);
    assert_eq!(after.collections[col.path()].runners[""].delay_ms, 0);
    assert_eq!(after.collections[col.path()].runners.len(), 1);
}

#[test]
fn an_older_session_file_without_runners_loads_and_writes_none() {
    let old = r#"{"collections":{"/c":{"tabs":["a.yml"],"environment":null}}}"#;
    let s: Session = serde_json::from_str(old).unwrap();
    assert!(s.collections[Path::new("/c")].runners.is_empty());
    assert!(!serde_json::to_string(&s).unwrap().contains("runners"));
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_data_file_that_is_not_utf8_never_reaches_the_session(cx: &mut TestAppContext) {
    use std::os::unix::ffi::OsStrExt as _;
    let col = collection_with_folder();
    let state = tempfile::tempdir().unwrap();
    let o = open_with(cx, &col, &state, Session::default(), "");
    let odd = PathBuf::from(std::ffi::OsStr::from_bytes(b"/tmp/rows-\xff.csv"));
    o.model.update(cx, |m, cx| {
        m.set_retries(2, cx);
        m.set_data_file(Some(odd), cx);
    });
    let session = session_of(cx, &o);
    let saved = &session.collections[col.path()].runners[""];
    assert_eq!((saved.retries, saved.data_file.as_ref()), (2, None));
    assert!(serde_json::to_string(&session).is_ok());
}

#[gpui_kit::test]
fn options_written_to_session_json_come_back_from_the_file(cx: &mut TestAppContext) {
    let col = collection_with_folder();
    let state = tempfile::tempdir().unwrap();
    let first = open_with(cx, &col, &state, Session::default(), "");
    first.model.update(cx, |m, cx| {
        m.set_retries(4, cx);
        m.set_bail(true, cx);
    });
    let persisted = first.ws.read_with(cx, |w, _| w.session().clone());
    persisted.update(cx, |p, _| p.flush_now()).unwrap();
    let loaded: Session = load_json(&state.path().join("session.json")).value;
    let other = tempfile::tempdir().unwrap();
    let second = open_with(cx, &col, &other, loaded, "");
    second.model.read_with(cx, |m, _| {
        assert_eq!((m.retries(), m.bail()), (4, true));
    });
}

#[gpui_kit::test]
fn the_fail_on_status_choice_comes_back_after_a_restart(cx: &mut TestAppContext) {
    let col = collection_with_folder();
    let (s1, s2) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let first = open_with(cx, &col, &s1, Session::default(), "");
    first.model.update(cx, |m, cx| {
        m.set_fail_on_status(Some(apinata_core::StatusRule::Server), cx)
    });
    let saved = session_of(cx, &first);
    let second = open_with(cx, &col, &s2, saved, "");
    second.model.read_with(cx, |m, _| {
        assert_eq!(m.fail_on_status(), Some(apinata_core::StatusRule::Server));
    });
}
