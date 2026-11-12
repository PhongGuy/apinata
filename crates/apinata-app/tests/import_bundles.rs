//! The app's Import collection and Import environment over files holding several of each.

use crate::support;

use apinata_app::env_selector::IMPORT_ENVIRONMENT;
use apinata_app::session::Session;
use apinata_app::shell::Shell;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, WindowOptions};
use std::fs;
use std::path::{Path, PathBuf};
use support::*;

const TWO_WORKSPACES: &str = r#"{"_type":"export","__export_format":4,"resources":[
    {"_id":"wrk_a","_type":"workspace","parentId":null,"name":"Alpha"},
    {"_id":"env_a","_type":"environment","parentId":"wrk_a","name":"A","data":{"host":"a.test"}},
    {"_id":"wrk_b","_type":"workspace","parentId":null,"name":"Beta"},
    {"_id":"env_b","_type":"environment","parentId":"wrk_b","name":"B","data":{"host":"b.test"}}]}"#;
const HAR: &str = r#"{"log":{"version":"1.2","entries":[{"request":{"method":"GET","url":"https://x.test/a","headers":[]}}]}}"#;
const THREE: &str = r#"[{"v":12,"name":"Alpha","folders":[],"requests":[]},{"v":12,"name":"CON","folders":[],"requests":[]},{"v":12,"name":"Gamma","folders":[],"requests":[]}]"#;
const ENVS: &str = r#"[{"v":0,"name":"Env-I","variables":[{"key":"a","value":"1"}]},
    {"v":1,"name":"Env-II","variables":[{"key":"b","value":"2","secret":false}]}]"#;

fn shell(
    cx: &mut TestAppContext,
    col: Option<&tempfile::TempDir>,
) -> (tempfile::TempDir, AnyWindowHandle, Entity<Shell>) {
    support::init(cx);
    let files = tempfile::tempdir().unwrap();
    let session = session_at(cx, &files, Session::default());
    let ws = workspace(cx, &session);
    if let Some(col) = col {
        ws.update(cx, |w, cx| w.open(col.path().to_path_buf(), cx));
    }
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| Shell::new(ws, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    (files, win, view)
}

fn opened(cx: &mut TestAppContext, view: &Entity<Shell>) -> Vec<PathBuf> {
    view.read_with(cx, |v, cx| {
        v.workspace()
            .read(cx)
            .collections()
            .iter()
            .map(|c| c.root().to_path_buf())
            .collect()
    })
}

fn dialog(cx: &mut TestAppContext, win: AnyWindowHandle) -> bool {
    cx.update_window(win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap()
}

/// Chooses Import collection in the collection switcher with `name` holding `text`, creating in
/// `into`, and answers the file prompt.
fn import(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    dir: &Path,
    name: &str,
    text: &str,
    into: &Path,
) {
    let file = dir.join(name);
    fs::write(&file, text).unwrap();
    switcher_click(cx, win, "switcher-import-collection");
    cx.update_window(win, |_, w, cx| w.click("import-link-browse", cx))
        .unwrap();
    cx.run_until_parked();
    crate::import_collection::create_in(cx, into);
    cx.simulate_path_prompt_response(move |_| Some(vec![file]));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn every_collection_in_a_file_is_written_and_opened(cx: &mut TestAppContext) {
    let (files, win, view) = shell(cx, None);
    let into = tempfile::tempdir().unwrap();
    import(
        cx,
        win,
        files.path(),
        "two.json",
        TWO_WORKSPACES,
        into.path(),
    );
    assert_eq!(
        opened(cx, &view),
        [into.path().join("Alpha"), into.path().join("Beta")]
    );
    let report = view
        .read_with(cx, |v, _| v.last_import().cloned())
        .expect("imported");
    assert_eq!(report.environments, ["A", "B"]);
    assert!(into.path().join("Beta/environments/B.yml").is_file());
}

#[gpui_kit::test]
fn a_har_capture_is_named_after_its_file(cx: &mut TestAppContext) {
    let (files, win, view) = shell(cx, None);
    let into = tempfile::tempdir().unwrap();
    import(cx, win, files.path(), "checkout.har", HAR, into.path());
    assert_eq!(opened(cx, &view), [into.path().join("checkout")]);
}

#[gpui_kit::test]
fn a_failed_write_opens_what_was_written_and_reports_the_rest(cx: &mut TestAppContext) {
    let (files, win, view) = shell(cx, None);
    let into = tempfile::tempdir().unwrap();
    import(cx, win, files.path(), "ws.json", THREE, into.path());
    assert_eq!(opened(cx, &view), [into.path().join("Alpha")]);
    assert!(dialog(cx, win));
    let left: Vec<_> = fs::read_dir(into.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(left, ["Alpha"]);
}

#[gpui_kit::test]
fn an_environment_file_with_several_environments_imports_each(cx: &mut TestAppContext) {
    let col = collection();
    let (files, win, view) = shell(cx, Some(&col));
    let bulk = files.path().join("bulk.json");
    fs::write(&bulk, ENVS).unwrap();
    let sel = view.read_with(cx, |v, _| v.env_selector().clone());
    cx.update_window(win, |_, w, cx| {
        sel.update(cx, |s, cx| s.choose(IMPORT_ENVIRONMENT, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    cx.simulate_path_prompt_response(move |_| Some(vec![bulk]));
    cx.run_until_parked();
    for name in ["Env-I", "Env-II"] {
        assert!(
            col.path()
                .join(format!("environments/{name}.yml"))
                .is_file(),
            "{name}"
        );
    }
    let report = sel
        .read_with(cx, |s, _| s.last_import().cloned())
        .expect("imported");
    assert_eq!(report.environments, ["Env-I", "Env-II"]);
}
