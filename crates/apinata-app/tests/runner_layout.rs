//! The runner's layout in short and tall panes: Run never scrolls out of the tab area, the
//! options scroll with a visible bar when capped, and status lines sit above the results.

use crate::support;

use apinata_app::runner_model::{RunState, RunnerModel};
use apinata_core::ItemPath;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, Entity, Pixels, ScrollDelta, TestAppContext, point, px, size,
};
use support::*;
use wiremock::ResponseTemplate;

const SLACK: Pixels = px(0.5);

// A collection of `n` GET requests against `uri`.
fn many(uri: &str, n: usize) -> tempfile::TempDir {
    let files: Vec<(String, String)> = (0..n)
        .map(|i| {
            (
                format!("r{i:02}.yml"),
                request(&format!("r{i:02}"), &format!("{uri}/ok")),
            )
        })
        .collect();
    let mut tree: Vec<(&str, &str)> = vec![("opencollection.yml", ROOT)];
    tree.extend(files.iter().map(|(a, b)| (a.as_str(), b.as_str())));
    write_tree(&tree)
}

// Opens a runner tab on the collection root in a window of the given size.
fn open_sized(
    cx: &mut TestAppContext,
    s: &Started,
    width: f32,
    height: f32,
) -> Entity<RunnerModel> {
    let tab =
        s.ws.update(cx, |w, cx| w.open_runner(&s.root, ItemPath::root(), cx))
            .expect("a runner tab");
    let model = s
        .shell
        .read_with(cx, |v, cx| v.tab_area().read(cx).runner(tab).cloned())
        .expect("a runner view")
        .read_with(cx, |v, _| v.model().clone());
    gpui_kit::VisualTestContext::from_window(s.win, cx)
        .simulate_resize(size(px(width), px(height)));
    cx.run_until_parked();
    frame(cx, s.win);
    frame(cx, s.win);
    model
}

// Runs an `n`-request collection to the end in a runner tab of a `width` x `height` window.
fn ran(
    cx: &mut TestAppContext,
    n: usize,
    width: f32,
    height: f32,
) -> (Started, Entity<RunnerModel>) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = many(&server.uri(), n);
    let s = started(cx, &col);
    let model = open_sized(cx, &s, width, height);
    cx.update_window(s.win, |_, w, cx| w.click("run-start", cx))
        .unwrap();
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| matches!(m.state(), RunState::Finished(_)))
    });
    frame(cx, s.win);
    frame(cx, s.win);
    // The temp dir and server must outlive the assertions.
    std::mem::forget(col);
    std::mem::forget(server);
    (s, model)
}

fn find(cx: &mut TestAppContext, s: &Started, id: &'static str) -> Bounds<Pixels> {
    cx.update_window(s.win, |_, w, _| w.find(id).bounds())
        .unwrap()
}

// Run lies wholly below the tab strip and inside the tab area and the window.
fn assert_run_in_tab_area(cx: &mut TestAppContext, s: &Started) -> Bounds<Pixels> {
    let run = find(cx, s, "run-start");
    let strip = find(cx, s, "tab-strip");
    let area = find(cx, s, "tab-area");
    let window = cx
        .update_window(s.win, |_, w, _| w.viewport_size())
        .unwrap();
    assert!(
        run.top() >= strip.bottom() - SLACK
            && run.bottom() <= area.bottom() + SLACK
            && run.bottom() <= window.height,
        "Run {run:?} outside the tab area {area:?} below the strip {strip:?}"
    );
    // A modal prompt keeps a wheel from reaching a whole-view scroll, so check there is none.
    assert!(
        !present(cx, s.win, "tab-view-scroll"),
        "the runner scrolls as a whole, so Run can leave view"
    );
    run
}

// A wheel over each of `over` scrolls only what lies inside the runner, so Run stays at `run`.
fn assert_wheel_keeps_run(
    cx: &mut TestAppContext,
    s: &Started,
    run: Bounds<Pixels>,
    over: &[&'static str],
) {
    for &id in over {
        cx.update_window(s.win, |_, w, cx| {
            w.scroll(id, ScrollDelta::Pixels(point(px(0.), px(-200.))), cx)
        })
        .unwrap();
        frame(cx, s.win);
        frame(cx, s.win);
        assert_eq!(
            find(cx, s, "run-start"),
            run,
            "Run moved under a wheel over {id}"
        );
    }
}

#[gpui_kit::test]
fn run_stays_in_the_tab_area_of_a_230px_window_with_forty_rows(cx: &mut TestAppContext) {
    let (s, _) = ran(cx, 40, 1280., 230.);
    let run = assert_run_in_tab_area(cx, &s);
    let header = find(cx, &s, "run-table-header");
    assert!(
        run.bottom() <= header.top() + SLACK,
        "Run {run:?} not above the table header {header:?}"
    );
    assert!(
        present(cx, s.win, "run-options-scrollbar"),
        "the capped options show no scrollbar"
    );
    assert_wheel_keeps_run(cx, &s, run, &["run-options", "run-start", "runner"]);
}

#[gpui_kit::test]
fn run_stays_in_the_tab_area_of_a_230px_window_before_a_run(cx: &mut TestAppContext) {
    let col = many("http://127.0.0.1:9", 40);
    let s = started(cx, &col);
    open_sized(cx, &s, 1280., 230.);
    let run = assert_run_in_tab_area(cx, &s);
    let options = find(cx, &s, "run-options");
    let plan = find(cx, &s, "run-plan");
    assert!(
        options.bottom() <= plan.top() + SLACK,
        "options {options:?} over the plan {plan:?}"
    );
    assert!(
        present(cx, s.win, "run-options-scrollbar"),
        "the capped options show no scrollbar"
    );
    assert_wheel_keeps_run(cx, &s, run, &["run-start", "runner"]);
}

#[gpui_kit::test]
fn run_stays_in_the_tab_area_of_a_230px_window_while_waiting(cx: &mut TestAppContext) {
    let req = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://127.0.0.1:9/{{?code}}\"\n";
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", req)]);
    let s = started(cx, &col);
    let model = open_sized(cx, &s, 1280., 230.);
    assert!(model.update(cx, |m, cx| m.start(cx)));
    wait_real(cx, |cx| {
        cx.run_until_parked();
        frame(cx, s.win);
        present(cx, s.win, "run-waiting")
    });
    frame(cx, s.win);
    let run = assert_run_in_tab_area(cx, &s);
    let options = find(cx, &s, "run-options");
    let waiting = find(cx, &s, "run-waiting");
    assert!(
        options.bottom() <= waiting.top() + SLACK,
        "options {options:?} over the waiting line {waiting:?}"
    );
    assert!(
        present(cx, s.win, "run-options-scrollbar"),
        "the capped options show no scrollbar"
    );
    assert_wheel_keeps_run(cx, &s, run, &["run-start", "runner"]);
    model.update(cx, |m, cx| m.cancel(cx));
}

#[gpui_kit::test]
fn the_stale_line_sits_above_the_table_at_1280x800(cx: &mut TestAppContext) {
    let (s, model) = ran(cx, 40, 1280., 800.);
    model.update(cx, |m, cx| m.set_bail(true, cx));
    frame(cx, s.win);
    frame(cx, s.win);
    assert!(
        !present(cx, s.win, "run-options-scrollbar"),
        "options above a shrinking table show a scrollbar"
    );
    let stale = find(cx, &s, "run-stale");
    let header = find(cx, &s, "run-table-header");
    let runner = find(cx, &s, "runner");
    assert!(
        stale.bottom() <= header.top() + SLACK,
        "stale line {stale:?} under the table header {header:?}"
    );
    assert!(
        stale.top() >= runner.top() - SLACK && stale.bottom() <= runner.bottom() + SLACK,
        "stale line {stale:?} outside the runner {runner:?}"
    );
}

#[gpui_kit::test]
fn few_rows_leave_the_options_at_their_content_height(cx: &mut TestAppContext) {
    let (s, _) = ran(cx, 2, 1280., 800.);
    assert!(
        !present(cx, s.win, "run-options-scrollbar"),
        "options that fit show a scrollbar"
    );
    let options = find(cx, &s, "run-options");
    for id in [
        "run-bail",
        "run-delay",
        "run-tags",
        "run-exclude-tags",
        "run-iterations",
        "run-retries",
        "run-retry-delay",
        "run-retry-unsafe",
        "run-retry-unsafe-note",
        "run-data-file",
    ] {
        let b = find(cx, &s, id);
        assert!(
            b.top() >= options.top() - SLACK && b.bottom() <= options.bottom() + SLACK,
            "{id} {b:?} outside the options {options:?}"
        );
    }
}

// Each option label and the control it names, in reading order.
const OPTION_PAIRS: [(&str, &str); 7] = [
    ("run-tags-label", "run-tags"),
    ("run-exclude-tags-label", "run-exclude-tags"),
    ("run-iterations-label", "run-iterations"),
    ("run-delay-label", "run-delay"),
    ("run-fail-on-status-label", "run-fail-on-status"),
    ("run-retries-label", "run-retries"),
    ("run-retry-delay-label", "run-retry-delay"),
];

// Every label shares a line with its control at one fixed offset, and the labels fall into
// a few shared columns, so the options read as a grid rather than a wrapped flow.
fn assert_options_in_columns(cx: &mut TestAppContext, s: &Started, width: f32) {
    let mut offsets = Vec::new();
    let mut columns: Vec<Pixels> = Vec::new();
    for (label, control) in OPTION_PAIRS {
        let (l, c) = (find(cx, s, label), find(cx, s, control));
        let middle = l.top() + l.size.height / 2.;
        assert!(
            middle >= c.top() && middle <= c.bottom(),
            "{width}px: {label} {l:?} not on the line of {control} {c:?}"
        );
        offsets.push(c.left() - l.left());
        if !columns.iter().any(|x| (*x - l.left()).abs() <= SLACK) {
            columns.push(l.left());
        }
    }
    assert!(
        offsets.iter().all(|o| (*o - offsets[0]).abs() <= SLACK),
        "{width}px: controls sit at different distances from their labels: {offsets:?}"
    );
    assert!(
        columns.len() <= 3,
        "{width}px: labels start at {} different places: {columns:?}",
        columns.len()
    );
    // The checkboxes line up with the controls' column, not with the labels.
    for id in ["run-bail", "run-retry-unsafe"] {
        let b = find(cx, s, id);
        assert!(
            columns
                .iter()
                .any(|x| (*x + offsets[0] - b.left()).abs() <= SLACK),
            "{width}px: {id} {b:?} off the control columns"
        );
    }
}

#[gpui_kit::test]
fn the_options_line_up_in_columns_at_1280x800(cx: &mut TestAppContext) {
    let col = many("http://127.0.0.1:9", 3);
    let s = started(cx, &col);
    open_sized(cx, &s, 1280., 800.);
    assert_options_in_columns(cx, &s, 1280.);
    // At the default size each group keeps to one line.
    for group in [
        &["run-tags-label", "run-exclude-tags-label"][..],
        &["run-iterations-label", "run-delay-label", "run-data-label"],
        &[
            "run-fail-on-status-label",
            "run-retries-label",
            "run-retry-delay-label",
        ],
    ] {
        let first = find(cx, &s, group[0]);
        for &id in &group[1..] {
            let b = find(cx, &s, id);
            assert!(
                b.top() < first.bottom() && b.bottom() > first.top(),
                "{id} {b:?} not on the line of {} {first:?}",
                group[0]
            );
        }
    }
}

// Every text field and the status choice share one width, so no column mixes a wide control
// with a narrow one.
#[gpui_kit::test]
fn every_option_control_has_one_width(cx: &mut TestAppContext) {
    let col = many("http://127.0.0.1:9", 3);
    let s = started(cx, &col);
    open_sized(cx, &s, 1280., 800.);
    let widths: Vec<(&str, Pixels)> = OPTION_PAIRS
        .iter()
        .map(|(_, id)| (*id, find(cx, &s, id).size.width))
        .collect();
    assert!(
        widths
            .iter()
            .all(|(_, w)| (*w - widths[0].1).abs() <= SLACK),
        "option controls differ in width: {widths:?}"
    );
}

// The stop-after-failure checkbox takes a row of its own under the options it would otherwise
// share a line with, not an option slot with an empty label.
#[gpui_kit::test]
fn stop_after_the_first_failure_sits_on_its_own_row(cx: &mut TestAppContext) {
    let col = many("http://127.0.0.1:9", 3);
    let s = started(cx, &col);
    open_sized(cx, &s, 1280., 800.);
    let bail = find(cx, &s, "run-bail");
    let last = find(cx, &s, "run-fail-on-status");
    assert!(
        bail.top() >= last.bottom() - SLACK,
        "run-bail {bail:?} shares a line with the option rows above {last:?}"
    );
}

#[gpui_kit::test]
fn the_options_line_up_in_columns_at_800x800(cx: &mut TestAppContext) {
    let col = many("http://127.0.0.1:9", 3);
    let s = started(cx, &col);
    open_sized(cx, &s, 800., 800.);
    assert_options_in_columns(cx, &s, 800.);
}

fn click(cx: &mut TestAppContext, s: &Started, id: impl Into<gpui_kit::ElementId>) {
    let id = id.into();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click(id, cx)).unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
}

// One button swaps results and checklist, keeping its id so focus stays on it.
const TOGGLE: &str = "run-selection-toggle";

fn toggle_label(cx: &mut TestAppContext, s: &Started) -> Option<String> {
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        w.try_find(TOGGLE)
            .and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

#[gpui_kit::test]
fn change_selection_brings_the_checklist_back_after_a_run(cx: &mut TestAppContext) {
    let (s, model) = ran(cx, 3, 1280., 800.);
    assert!(
        present(cx, s.win, "run-table-header"),
        "results after the run"
    );
    assert!(
        !present(cx, s.win, "run-plan"),
        "the plan gives way to results"
    );
    assert_eq!(toggle_label(cx, &s).as_deref(), Some("Change selection"));
    click(cx, &s, TOGGLE);
    assert_eq!(toggle_label(cx, &s).as_deref(), Some("Show results"));
    assert!(
        present(cx, s.win, ("run-plan-row", 2usize)),
        "the checklist is back"
    );
    assert!(
        !present(cx, s.win, "run-table-header"),
        "the table steps aside"
    );
    assert!(present(cx, s.win, "run-summary"), "the last summary stays");
    // The results are kept and come back without running again.
    click(cx, &s, TOGGLE);
    assert!(
        present(cx, s.win, "run-table-header"),
        "results shown again"
    );
    assert_eq!(model.read_with(cx, |m, _| m.rows().len()), 3);
    // A changed selection runs only what is checked, and the results replace the checklist.
    click(cx, &s, TOGGLE);
    click(cx, &s, ("run-plan-check", 0usize));
    click(cx, &s, "run-start");
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| {
            matches!(m.state(), RunState::Finished(_)) && m.rows().len() == 2
        })
    });
    frame(cx, s.win);
    assert!(
        !present(cx, s.win, "run-plan"),
        "a new run shows its results"
    );
    assert!(present(cx, s.win, "run-table-header"));
}

#[gpui_kit::test]
fn the_selection_toggle_is_hidden_while_a_run_is_going(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(
        cx,
        ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(30)),
    );
    let col = many(&server.uri(), 2);
    let s = started(cx, &col);
    let model = open_sized(cx, &s, 1280., 800.);
    click(cx, &s, "run-start");
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| matches!(m.state(), RunState::Running))
    });
    frame(cx, s.win);
    assert!(!present(cx, s.win, TOGGLE), "a toggle during the run");
    click(cx, &s, "run-cancel");
    wait_real(cx, |cx| model.read_with(cx, |m, _| !m.is_running()));
    frame(cx, s.win);
    assert!(present(cx, s.win, TOGGLE), "no toggle once the run stopped");
}

#[gpui_kit::test]
fn a_long_data_file_name_stays_inside_the_options_at_800px(cx: &mut TestAppContext) {
    let col = many("http://127.0.0.1:9", 2);
    let s = started(cx, &col);
    let model = open_sized(cx, &s, 800., 800.);
    let long = format!("{}.csv", "rows-from-the-nightly-export-".repeat(8));
    model.update(cx, |m, cx| m.set_data_file(Some(col.path().join(long)), cx));
    frame(cx, s.win);
    frame(cx, s.win);
    let options = find(cx, &s, "run-options");
    for id in ["run-data-name", "run-data-clear"] {
        let b = find(cx, &s, id);
        assert!(
            b.right() <= options.right() + SLACK,
            "{id} {b:?} past the options {options:?}"
        );
    }
}
