//! The runner's retry options: retries, the pause between them and the opt-in for POST and PATCH.

use crate::support;

use apinata_app::runner_model::RunState;
use apinata_app::tabs::TabId;
use apinata_core::ItemPath;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use support::*;
use wiremock::matchers::any;
use wiremock::{Mock, ResponseTemplate};

fn open(
    cx: &mut TestAppContext,
    s: &Started,
) -> (
    TabId,
    Entity<apinata_app::runner_view::RunnerView>,
    Entity<apinata_app::runner_model::RunnerModel>,
) {
    let tab =
        s.ws.update(cx, |w, cx| w.open_runner(&s.root, ItemPath::root(), cx))
            .expect("a runner tab");
    let view = s
        .shell
        .read_with(cx, |v, cx| v.tab_area().read(cx).runner(tab).cloned())
        .expect("a runner view");
    let model = view.read_with(cx, |v, _| v.model().clone());
    (tab, view, model)
}

fn click(cx: &mut TestAppContext, s: &Started, id: &'static str) {
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        use gpui_kit::test::TestWindowExt as _;
        w.click(id, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

fn outcome_label(cx: &mut TestAppContext, s: &Started) -> Option<String> {
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        use gpui_kit::test::TestWindowExt as _;
        w.try_find(("run-outcome", 0usize))
            .and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

fn finished(cx: &mut TestAppContext, model: &Entity<apinata_app::runner_model::RunnerModel>) {
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| matches!(m.state(), RunState::Finished(_)))
    });
}

fn method_collection(method: &str, uri: &str) -> tempfile::TempDir {
    let a = format!(
        "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: {method}\n  url: \"{uri}/ok\"\n"
    );
    write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)])
}

#[gpui_kit::test]
fn a_run_with_retries_shows_a_retrying_row_then_its_attempt_count(cx: &mut TestAppContext) {
    http_ready(cx);
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(wiremock::MockServer::builder().start());
    handle.block_on(
        Mock::given(any())
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(2)
            .mount(&server),
    );
    handle.block_on(
        Mock::given(any())
            .respond_with(ResponseTemplate::new(200))
            .mount(&server),
    );
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    let (_, _, model) = open(cx, &s);
    model.update(cx, |m, cx| {
        m.set_retries(2, cx);
        m.set_retry_delay_ms(1500, cx);
    });
    click(cx, &s, "run-start");
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.retrying(0).is_some()));
    assert_eq!(
        outcome_label(cx, &s).as_deref(),
        Some("Retrying, attempt 2 of 3")
    );
    finished(cx, &model);
    let label = outcome_label(cx, &s).expect("a result label");
    assert!(label.ends_with("(3 attempts)"), "{label}");
    assert_eq!(served(cx, &server).len(), 3);
}

#[gpui_kit::test]
fn the_retry_options_default_to_off_and_follow_the_inputs(cx: &mut TestAppContext) {
    let col = method_collection("GET", "http://127.0.0.1:9");
    let s = started(cx, &col);
    let (_, view, model) = open(cx, &s);
    model.read_with(cx, |m, _| {
        assert_eq!(
            (m.retries(), m.retry_delay_ms(), m.retry_unsafe()),
            (0, 0, false)
        );
    });
    frame(cx, s.win);
    let texts = view.read_with(cx, |v, cx| v.retry_field_texts(cx)).unwrap();
    assert_eq!(texts, ["0", "0"]);
    replace_input(cx, s.win, None, "run-retries", "3x");
    replace_input(cx, s.win, None, "run-retry-delay", "250");
    click(cx, &s, "run-retry-unsafe");
    model.read_with(cx, |m, _| {
        assert_eq!(
            (m.retries(), m.retry_delay_ms(), m.retry_unsafe()),
            (3, 250, true)
        );
    });
    let texts = view.read_with(cx, |v, cx| v.retry_field_texts(cx)).unwrap();
    assert_eq!(texts, ["3", "250"], "non-digits are stripped");
}

#[gpui_kit::test]
fn the_options_survive_leaving_the_tab_and_coming_back(cx: &mut TestAppContext) {
    let col = method_collection("GET", "http://127.0.0.1:9");
    let s = started(cx, &col);
    let (tab, view, model) = open(cx, &s);
    frame(cx, s.win);
    replace_input(cx, s.win, None, "run-retries", "4");
    replace_input(cx, s.win, None, "run-retry-delay", "80");
    click(cx, &s, "run-retry-unsafe");
    // Another tab takes the focus; the runner tab is the same one when it is reopened.
    s.ws.update(cx, |w, cx| w.open_runner(&s.root, ItemPath::root(), cx));
    s.ws.update(cx, |w, cx| w.open_item(&s.root, ip("a.yml"), false, cx));
    frame(cx, s.win);
    s.ws.update(cx, |w, cx| w.open_runner(&s.root, ItemPath::root(), cx));
    frame(cx, s.win);
    let again = s
        .shell
        .read_with(cx, |v, cx| v.tab_area().read(cx).runner(tab).cloned())
        .expect("the same runner view");
    assert_eq!(again.entity_id(), view.entity_id());
    model.read_with(cx, |m, _| {
        assert_eq!(
            (m.retries(), m.retry_delay_ms(), m.retry_unsafe()),
            (4, 80, true)
        );
    });
    let texts = view.read_with(cx, |v, cx| v.retry_field_texts(cx)).unwrap();
    assert_eq!(texts, ["4", "80"]);
}

#[gpui_kit::test]
fn changing_a_retry_option_makes_the_results_stale(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    let (_, _, model) = open(cx, &s);
    click(cx, &s, "run-start");
    finished(cx, &model);
    assert!(!present(cx, s.win, "run-stale"));
    model.update(cx, |m, cx| m.set_retries(1, cx));
    assert!(present(cx, s.win, "run-stale"), "retries changed");
    model.update(cx, |m, cx| m.set_retries(0, cx));
    assert!(!present(cx, s.win, "run-stale"), "retries match again");
    model.update(cx, |m, cx| m.set_retry_delay_ms(9, cx));
    assert!(present(cx, s.win, "run-stale"), "delay changed");
    model.update(cx, |m, cx| m.set_retry_delay_ms(0, cx));
    assert!(!present(cx, s.win, "run-stale"), "delay matches again");
    model.update(cx, |m, cx| m.set_retry_unsafe(true, cx));
    assert!(present(cx, s.win, "run-stale"), "unsafe changed");
    model.update(cx, |m, cx| m.set_retry_unsafe(false, cx));
    assert!(!present(cx, s.win, "run-stale"), "unsafe matches again");
}

#[gpui_kit::test]
fn a_post_is_retried_only_with_the_opt_in(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(503));
    let col = method_collection("POST", &server.uri());
    let s = started(cx, &col);
    let (_, _, model) = open(cx, &s);
    model.update(cx, |m, cx| m.set_retries(1, cx));
    click(cx, &s, "run-start");
    finished(cx, &model);
    assert_eq!(served(cx, &server).len(), 1, "a POST is sent once");
    model.update(cx, |m, cx| m.set_retry_unsafe(true, cx));
    click(cx, &s, "run-start");
    finished(cx, &model);
    assert_eq!(served(cx, &server).len(), 3, "one more send plus its retry");
}

#[gpui_kit::test]
fn the_retry_controls_are_labelled_and_the_risk_is_explained(cx: &mut TestAppContext) {
    let col = method_collection("GET", "http://127.0.0.1:9");
    let s = started(cx, &col);
    open(cx, &s);
    frame(cx, s.win);
    let label = |cx: &mut TestAppContext, id: &'static str| {
        cx.update_window(s.win, |_, w, _| {
            use gpui_kit::test::TestWindowExt as _;
            w.try_find(id).and_then(|e| e.label().map(str::to_owned))
        })
        .unwrap()
    };
    assert_eq!(label(cx, "run-retries-label").as_deref(), Some("Retries"));
    assert_eq!(
        label(cx, "run-retry-delay-label").as_deref(),
        Some("Retry delay (ms)")
    );
    assert_eq!(
        label(cx, "run-retry-unsafe-note").as_deref(),
        Some("A retried POST or PATCH may repeat a side effect on the server. Off by default.")
    );
}

/// Width of `text` shaped in the window's default text style, as the table cells draw it.
fn text_width(w: &mut gpui_kit::Window, text: &str) -> gpui_kit::Pixels {
    let size = w.text_style().font_size.to_pixels(w.rem_size());
    let run = w.text_style().to_run(text.len());
    w.text_system()
        .shape_line(text.to_owned().into(), size, &[run], None)
        .width
}

/// The outcome cell must be wide enough for its whole label; a cut label reads as a bug.
fn assert_outcome_fits(cx: &mut TestAppContext, s: &Started, expected: &str) {
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        use gpui_kit::test::TestWindowExt as _;
        let cell = w.find(("run-outcome", 0usize)).bounds();
        let text = text_width(w, expected);
        assert!(
            text <= cell.size.width,
            "{expected:?} is {text:?} wide in a cell of {:?}",
            cell.size.width
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_result_column_fits_the_retry_labels(cx: &mut TestAppContext) {
    http_ready(cx);
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(wiremock::MockServer::builder().start());
    handle.block_on(
        Mock::given(any())
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(2)
            .mount(&server),
    );
    handle.block_on(
        Mock::given(any())
            .respond_with(ResponseTemplate::new(200))
            .mount(&server),
    );
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    let (_, _, model) = open(cx, &s);
    model.update(cx, |m, cx| {
        m.set_retries(2, cx);
        m.set_retry_delay_ms(1500, cx);
    });
    click(cx, &s, "run-start");
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.retrying(0).is_some()));
    assert_eq!(
        outcome_label(cx, &s).as_deref(),
        Some("Retrying, attempt 2 of 3")
    );
    assert_outcome_fits(cx, &s, "Retrying, attempt 2 of 3");
    finished(cx, &model);
    let done = outcome_label(cx, &s).expect("a result label");
    assert!(done.ends_with("(3 attempts)"), "{done}");
    assert_outcome_fits(cx, &s, &done);
    assert_outcome_fits(cx, &s, "Passed (3 attempts)");
}

#[gpui_kit::test]
fn the_unsafe_retry_note_wraps_inside_the_options_at_640px(cx: &mut TestAppContext) {
    let col = method_collection("GET", "http://127.0.0.1:9");
    let s = started(cx, &col);
    open(cx, &s);
    gpui_kit::VisualTestContext::from_window(s.win, cx)
        .simulate_resize(gpui_kit::size(gpui_kit::px(640.), gpui_kit::px(800.)));
    cx.run_until_parked();
    frame(cx, s.win);
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        use gpui_kit::test::TestWindowExt as _;
        let options = w.find("run-options").bounds();
        let note = w.find("run-retry-unsafe-note").bounds();
        let line = w.text_style().font_size.to_pixels(w.rem_size());
        assert!(
            note.right() <= options.right() + gpui_kit::px(1.),
            "note {note:?} past options {options:?}"
        );
        assert!(
            note.size.height > line * 1.5,
            "note {note:?} should wrap to several lines"
        );
    })
    .unwrap();
}

// Runs a 40-request collection to completion in a window of the given size, so the results
// table has far more rows than the pane can show.
fn ran_forty(cx: &mut TestAppContext, width: f32, height: f32) -> Started {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let files: Vec<(String, String)> = (0..40)
        .map(|i| {
            (
                format!("r{i:02}.yml"),
                request(&format!("r{i:02}"), &format!("{}/ok", server.uri())),
            )
        })
        .collect();
    let mut tree: Vec<(&str, &str)> = vec![("opencollection.yml", ROOT)];
    tree.extend(files.iter().map(|(a, b)| (a.as_str(), b.as_str())));
    let col = write_tree(&tree);
    let s = started(cx, &col);
    let (_, _, model) = open(cx, &s);
    gpui_kit::VisualTestContext::from_window(s.win, cx)
        .simulate_resize(gpui_kit::size(gpui_kit::px(width), gpui_kit::px(height)));
    cx.run_until_parked();
    click(cx, &s, "run-start");
    finished(cx, &model);
    frame(cx, s.win);
    frame(cx, s.win);
    // The temp dir and server must outlive the assertions.
    std::mem::forget(col);
    std::mem::forget(server);
    s
}

const OPTION_IDS: [&str; 10] = [
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
];

#[gpui_kit::test]
fn the_options_keep_their_height_above_forty_result_rows_at_1280x800(cx: &mut TestAppContext) {
    let s = ran_forty(cx, 1280., 800.);
    cx.update_window(s.win, |_, w, _| {
        use gpui_kit::test::TestWindowExt as _;
        let options = w.find("run-options").bounds();
        let header = w.find("run-table-header").bounds();
        assert!(
            header.top() >= options.bottom() - gpui_kit::px(0.5),
            "table header {header:?} starts above the options' bottom {options:?}"
        );
        for id in OPTION_IDS {
            let b = w.find(id).bounds();
            assert!(
                b.top() >= options.top() - gpui_kit::px(0.5)
                    && b.bottom() <= options.bottom() + gpui_kit::px(0.5)
                    && b.right() <= options.right() + gpui_kit::px(0.5),
                "{id} {b:?} lies outside the options {options:?}"
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_short_pane_keeps_run_visible_and_the_options_off_the_table(cx: &mut TestAppContext) {
    let s = ran_forty(cx, 1280., 300.);
    cx.update_window(s.win, |_, w, _| {
        use gpui_kit::test::TestWindowExt as _;
        let window = w.viewport_size();
        let options = w.find("run-options").bounds();
        let run = w.find("run-start").bounds();
        let header = w.find("run-table-header").bounds();
        assert!(
            run.top() >= gpui_kit::px(0.) && run.bottom() <= window.height,
            "Run {run:?} left the {window:?} window"
        );
        assert!(
            header.top() >= options.bottom() - gpui_kit::px(0.5),
            "table header {header:?} starts above the options' bottom {options:?}"
        );
    })
    .unwrap();
}
