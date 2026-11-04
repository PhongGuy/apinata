//! Exporting a finished run from the runner tab as JUnit XML, JSON or HTML.

use crate::support;

use apinata_app::runner_export::ReportFormat;
use apinata_app::runner_model::{RowState, RunState, RunnerModel};
use apinata_core::{ItemPath, ResultsReport};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use std::time::Duration;
use support::*;
use wiremock::ResponseTemplate;

fn click(cx: &mut TestAppContext, s: &Started, id: &'static str) {
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click(id, cx)).unwrap();
    cx.run_until_parked();
}

fn open(cx: &mut TestAppContext, s: &Started) -> Entity<RunnerModel> {
    let tab =
        s.ws.update(cx, |w, cx| w.open_runner(&s.root, ItemPath::root(), cx))
            .expect("a runner tab");
    s.shell
        .read_with(cx, |v, cx| v.tab_area().read(cx).runner(tab).cloned())
        .expect("a runner view")
        .read_with(cx, |v, _| v.model().clone())
}

fn finish(cx: &mut TestAppContext, s: &Started, model: &Entity<RunnerModel>) {
    click(cx, s, "run-start");
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| matches!(m.state(), RunState::Finished(_)))
    });
}

fn toasts(cx: &mut TestAppContext, s: &Started) -> usize {
    cx.update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

// The report core builds from the finished rows, as the CLI builds it from the run's results.
fn expected(cx: &mut TestAppContext, model: &Entity<RunnerModel>) -> ResultsReport {
    model.read_with(cx, |m, _| {
        let done: Vec<_> = m
            .rows()
            .iter()
            .filter_map(|r| match &r.state {
                RowState::Done(result) => Some(result.clone()),
                _ => None,
            })
            .collect();
        ResultsReport::new(Vec::new(), done.iter().map(|r| r.as_ref()))
    })
}

// Picks `file` in the open save dialog and returns what was written, if anything.
fn answer(cx: &mut TestAppContext, file: &std::path::Path) -> Option<String> {
    assert!(cx.did_prompt_for_new_path());
    let chosen = file.to_path_buf();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    cx.run_until_parked();
    std::fs::read_to_string(file).ok()
}

#[gpui_kit::test]
fn export_is_absent_before_a_run_and_has_no_report(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    let model = open(cx, &s);
    frame(cx, s.win);
    for id in ["run-export-junit", "run-export-json", "run-export-html"] {
        assert!(!present(cx, s.win, id), "{id}");
    }
    assert!(model.read_with(cx, |m, _| m.export_report().is_none()));
}

#[gpui_kit::test]
fn export_does_nothing_while_the_run_is_going(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(
        cx,
        ResponseTemplate::new(200).set_delay(Duration::from_secs(30)),
    );
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    let model = open(cx, &s);
    click(cx, &s, "run-start");
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| matches!(m.state(), RunState::Running))
    });
    assert!(model.read_with(cx, |m, _| m.export_report().is_none()));
    frame(cx, s.win);
    assert!(present(cx, s.win, "run-export-json"));
    click(cx, &s, "run-export-json");
    assert!(!cx.did_prompt_for_new_path());
    click(cx, &s, "run-cancel");
    wait_real(cx, |cx| model.read_with(cx, |m, _| !m.is_running()));
}

#[gpui_kit::test]
fn json_export_writes_the_core_json_through_the_dialog(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    let model = open(cx, &s);
    finish(cx, &s, &model);
    let want = expected(cx, &model).to_json();
    let before = toasts(cx, &s);
    click(cx, &s, "run-export-json");
    let out = tempfile::tempdir().unwrap();
    let file = out.path().join("results.json");
    assert_eq!(answer(cx, &file), Some(want));
    assert_eq!(toasts(cx, &s), before + 1);
}

#[gpui_kit::test]
fn junit_export_equals_the_core_renderer_for_its_own_stamp(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    let model = open(cx, &s);
    finish(cx, &s, &model);
    click(cx, &s, "run-export-junit");
    let out = tempfile::tempdir().unwrap();
    let file = out.path().join("results.xml");
    let xml = answer(cx, &file).expect("written");
    let stamp = xml
        .split("timestamp=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("a timestamp")
        .to_owned();
    let host = gethostname::gethostname();
    let want = expected(cx, &model).to_junit(&stamp, &host.to_string_lossy(), false);
    assert_eq!(xml, want);
}

#[gpui_kit::test]
fn html_export_equals_the_core_renderer_for_its_own_stamp(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    let model = open(cx, &s);
    finish(cx, &s, &model);
    click(cx, &s, "run-export-html");
    let out = tempfile::tempdir().unwrap();
    let file = out.path().join("results.html");
    let html = answer(cx, &file).expect("written");
    let end = html.find(" UTC").expect("a stamp") + " UTC".len();
    let stamp = html[end - "2026-10-12 10:00:00 UTC".len()..end].to_owned();
    assert_eq!(html, expected(cx, &model).to_html(&stamp));
}

#[gpui_kit::test]
fn the_bail_flag_of_the_run_words_the_skipped_message(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(500));
    let a = failing_request("a", &server.uri());
    let b = failing_request("b", &server.uri());
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a), ("b.yml", &b)]);
    let s = started(cx, &col);
    let model = open(cx, &s);
    click(cx, &s, "run-bail");
    finish(cx, &s, &model);
    click(cx, &s, "run-export-junit");
    let out = tempfile::tempdir().unwrap();
    let xml = answer(cx, &out.path().join("r.xml")).expect("written");
    assert!(xml.contains("Request skipped due to bail"), "{xml}");
}

#[gpui_kit::test]
fn a_write_failure_is_toasted_and_leaves_no_file(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    let model = open(cx, &s);
    finish(cx, &s, &model);
    let before = toasts(cx, &s);
    click(cx, &s, "run-export-json");
    // A path beneath a regular file cannot be created.
    let out = tempfile::tempdir().unwrap();
    let blocker = out.path().join("file");
    std::fs::write(&blocker, "x").unwrap();
    let target = blocker.join("results.json");
    assert_eq!(answer(cx, &target), None);
    assert_eq!(toasts(cx, &s), before + 1);
}

#[test]
fn default_names_carry_the_format_extension() {
    assert_eq!(ReportFormat::Junit.file_name(), "results.xml");
    assert_eq!(ReportFormat::Json.file_name(), "results.json");
    assert_eq!(ReportFormat::Html.file_name(), "results.html");
}

// A request file whose one assertion fails against a 500.
fn failing_request(name: &str, uri: &str) -> String {
    let fails = "runtime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n";
    format!("{}{fails}", request(name, &format!("{uri}/ok")))
}
