use crate::support;

use apinata_app::keymap::ToggleConsole;
use apinata_app::runner_model::RunState;
use apinata_app::send_model::SendState;
use apinata_app::sidebar::menu::MenuEntry;
use apinata_app::sidebar::model::NodeKind;
use apinata_app::tabs::{TabId, TabKind};
use apinata_core::{ItemPath, Outcome};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext, px, size};
use std::time::Duration;
use support::*;
use wiremock::ResponseTemplate;

fn view(
    cx: &mut TestAppContext,
    s: &Started,
    id: TabId,
) -> Entity<apinata_app::runner_view::RunnerView> {
    s.shell
        .read_with(cx, |v, cx| v.tab_area().read(cx).runner(id).cloned())
        .expect("a runner view")
}

fn click(cx: &mut TestAppContext, s: &Started, id: impl Into<gpui_kit::ElementId>) {
    let id = id.into();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click(id, cx)).unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_menu_opens_one_runner_per_target(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("Users")),
        ("users/list.yml", &request("List users", "http://x/users")),
    ]);
    let s = started(cx, &col);
    choose_menu(cx, s.win, "users", NodeKind::Folder, MenuEntry::RunFolder);
    choose_menu(cx, s.win, "users", NodeKind::Folder, MenuEntry::RunFolder);
    choose_menu(
        cx,
        s.win,
        "",
        NodeKind::Collection,
        MenuEntry::RunCollection,
    );
    let kinds: Vec<TabKind> = s.ws.read_with(cx, |w, _| {
        w.tabs().entries().iter().map(|e| e.kind.clone()).collect()
    });
    assert_eq!(
        kinds,
        [
            TabKind::Runner(ip("users")),
            TabKind::Runner(ItemPath::root())
        ]
    );
    assert_eq!(
        s.ws.update(cx, |w, cx| w.open_runner(&s.root, ip("users/list.yml"), cx)),
        None
    );
}

#[gpui_kit::test]
fn clicking_a_row_opens_its_request_with_the_runs_result(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200).set_body_string("hello"));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    let (_, model) = run_to_end(cx, &s, ItemPath::root());
    assert!(present(cx, s.win, ("run-outcome", 0usize)) && present(cx, s.win, "run-summary"));
    let (ix, result) = done_row(cx, &model, "a.yml");
    click(cx, &s, ("run-row", ix));
    let active =
        s.ws.read_with(cx, |w, _| w.tabs().active().map(|t| (t.id, t.kind.clone())));
    let (id, kind) = active.expect("a tab is active");
    assert_eq!(kind, TabKind::Request(ip("a.yml")));
    let editor = s
        .shell
        .read_with(cx, |v, cx| v.tab_area().read(cx).editor(id).cloned())
        .expect("the request's editor");
    let send = editor.read_with(cx, |e, _| e.send_model().clone());
    assert!(send.read_with(
        cx,
        |m, _| matches!(m.state(), SendState::Done(r) if std::sync::Arc::ptr_eq(r, &result))
    ));
    assert!(present(cx, s.win, "response-status"));
    // Back on the runner, no result pane sits below the rows.
    s.ws.update(cx, |w, cx| w.open_runner(&s.root, ItemPath::root(), cx));
    assert!(present(cx, s.win, ("run-row", ix)));
    assert!(!present(cx, s.win, "run-response"));
}

#[gpui_kit::test]
fn bail_and_cancel_are_wired(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(
        cx,
        ResponseTemplate::new(200).set_delay(Duration::from_secs(30)),
    );
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    let tab =
        s.ws.update(cx, |w, cx| w.open_runner(&s.root, ItemPath::root(), cx))
            .unwrap();
    let model = view(cx, &s, tab).read_with(cx, |v, _| v.model().clone());
    click(cx, &s, "run-bail");
    assert!(model.read_with(cx, |m, _| m.bail()));
    click(cx, &s, "run-start");
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| matches!(m.state(), RunState::Running))
    });
    click(cx, &s, "run-cancel");
    wait_real(cx, |cx| model.read_with(cx, |m, _| !m.is_running()));
    let first = model.read_with(cx, |m, _| match &m.rows()[0].state {
        apinata_app::runner_model::RowState::Done(r) => Some(r.outcome),
        _ => None,
    });
    assert_eq!(first, Some(Outcome::Cancelled));
    // The cancelled row opens its request, showing the cancellation.
    click(cx, &s, ("run-row", 0usize));
    let kind =
        s.ws.read_with(cx, |w, _| w.tabs().active().map(|t| t.kind.clone()));
    assert_eq!(kind, Some(TabKind::Request(ip("a.yml"))));
    assert!(present(cx, s.win, "response-error"));
}

// Opens the runner of `target`, runs it and waits for the run to finish.
fn run_to_end(
    cx: &mut TestAppContext,
    s: &Started,
    target: ItemPath,
) -> (TabId, Entity<apinata_app::runner_model::RunnerModel>) {
    let tab =
        s.ws.update(cx, |w, cx| w.open_runner(&s.root, target, cx))
            .expect("a runner tab");
    click(cx, s, "run-start");
    let model = view(cx, s, tab).read_with(cx, |v, _| v.model().clone());
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| matches!(m.state(), RunState::Finished(_)))
    });
    (tab, model)
}

// The accessibility label of element `id`, which carries a text element's words.
fn label(
    cx: &mut TestAppContext,
    s: &Started,
    id: impl Into<gpui_kit::ElementId>,
) -> Option<String> {
    let id = id.into();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

// The finished result of the row for request `rel`, with that row's index.
fn done_row(
    cx: &mut TestAppContext,
    model: &Entity<apinata_app::runner_model::RunnerModel>,
    rel: &str,
) -> (usize, std::sync::Arc<apinata_core::RequestResult>) {
    model.read_with(cx, |m, _| {
        let ix = m
            .rows()
            .iter()
            .position(|r| r.path == ip(rel))
            .expect("a row");
        match &m.rows()[ix].state {
            apinata_app::runner_model::RowState::Done(r) => (ix, r.clone()),
            _ => panic!("row {rel} has not finished"),
        }
    })
}

#[gpui_kit::test]
fn a_run_sends_the_saved_files_and_names_unsaved_requests(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let saved = request("List users", &format!("{}/ok", server.uri()));
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &saved)]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    // An unsaved draft that would error the run if it were sent.
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, edit_url("a.yml", "{{missing}}/draft"), cx)
    })
    .expect("the collection is open");
    cx.run_until_parked();
    let (_, model) = run_to_end(cx, &s, ItemPath::root());
    let (_, result) = done_row(cx, &model, "a.yml");
    assert_eq!(result.outcome, Outcome::Passed, "the run sent the draft");
    assert_eq!(served(cx, &server), ["/ok"]);
    let notice = label(cx, &s, "runner-unsaved-notice").expect("an unsaved-edits notice");
    assert!(notice.contains("List users"), "{notice}");
}

#[gpui_kit::test]
fn the_unsaved_notice_names_only_requests_in_the_runs_folder(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let url = format!("{}/ok", server.uri());
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("Users")),
        ("users/list.yml", &request("List users", &url)),
        ("users2/folder.yml", &folder("Users 2")),
        ("users2/other.yml", &request("Other", &url)),
    ]);
    let s = started(cx, &col);
    for rel in ["users/list.yml", "users2/other.yml"] {
        open_tab(cx, &s.ws, &s.root, rel);
        s.ws.update(cx, |w, cx| {
            w.send(&s.root, edit_url(rel, &format!("{url}/draft")), cx)
        })
        .expect("the collection is open");
    }
    cx.run_until_parked();
    run_to_end(cx, &s, ip("users"));
    let notice = label(cx, &s, "runner-unsaved-notice").expect("an unsaved-edits notice");
    assert!(
        notice.contains("List users") && !notice.contains("Other"),
        "{notice}"
    );
}

#[gpui_kit::test]
fn rows_name_their_request_and_a_failed_row_says_why(cx: &mut TestAppContext) {
    use apinata_app::style::{PANE_INSET, at_zoom};
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(500));
    let check = "runtime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n";
    let failing = format!(
        "{}{check}",
        request("List users", &format!("{}/ok", server.uri()))
    );
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &failing),
        ("b.yml", &request("Broken", "{{missing}}/x")),
    ]);
    let s = started(cx, &col);
    let (_, model) = run_to_end(cx, &s, ItemPath::root());
    let (a, failed) = done_row(cx, &model, "a.yml");
    let (b, errored) = done_row(cx, &model, "b.yml");
    assert_eq!(
        (failed.outcome, errored.outcome),
        (Outcome::Failed, Outcome::Errored)
    );
    assert_eq!(
        label(cx, &s, ("run-name", a)).as_deref(),
        Some("GET List users")
    );
    assert_eq!(
        label(cx, &s, ("run-name", b)).as_deref(),
        Some("GET Broken")
    );
    // The Status column already shows the code, so the reason does not repeat it.
    let why = label(cx, &s, ("run-reason", a)).expect("a failed row's reason");
    assert_eq!(why, "res.status eq 200: actual 500");
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let (reason, name) = (
            w.find(("run-reason", a)).bounds(),
            w.find(("run-name", a)).bounds(),
        );
        assert_eq!(reason.left(), name.left(), "the reason sits under the Name");
    })
    .unwrap();
    let why = label(cx, &s, ("run-reason", b)).expect("an errored row's reason");
    assert!(why.contains("missing"), "{why}");
    assert!(
        !present(cx, s.win, "runner-unsaved-notice"),
        "nothing is unsaved"
    );
    cx.update_window(s.win, |_, w, _| {
        let area = w.find("tab-area").bounds();
        let bail = w.find("run-bail").bounds();
        assert!(
            bail.left() >= area.left() + at_zoom(PANE_INSET, 1.0),
            "run-bail {bail:?}, area {area:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn typed_letters_do_not_stay_in_the_number_fields(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = collection_for("http://127.0.0.1:1", "");
    let s = started(cx, &col);
    let tab =
        s.ws.update(cx, |w, cx| w.open_runner(&s.root, ItemPath::root(), cx))
            .unwrap();
    let model = view(cx, &s, tab).read_with(cx, |v, _| v.model().clone());
    frame(cx, s.win);
    let v = view(cx, &s, tab);
    for id in ["run-iterations", "run-delay"] {
        replace_input(cx, s.win, None, id, "a1b2x");
        cx.run_until_parked();
        frame(cx, s.win);
        let texts = v.read_with(cx, |v, cx| v.number_field_texts(cx)).unwrap();
        let ix = usize::from(id == "run-iterations");
        assert_eq!(texts[ix], "12", "{id}: {texts:?}");
    }
    model.read_with(cx, |m, _| {
        assert_eq!((m.delay_ms(), m.iteration_count()), (12, Some(12)));
    });
}

#[gpui_kit::test]
fn run_options_are_wired_and_iterations_get_headers(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let rows = write_tree(&[("rows.json", "[{\"id\":1},{\"id\":2}]")]);
    let s = started(cx, &col);
    let tab =
        s.ws.update(cx, |w, cx| w.open_runner(&s.root, ItemPath::root(), cx))
            .unwrap();
    let model = view(cx, &s, tab).read_with(cx, |v, _| v.model().clone());
    frame(cx, s.win);
    replace_input(cx, s.win, None, "run-delay", "250");
    replace_input(cx, s.win, None, "run-tags", "a, b");
    replace_input(cx, s.win, None, "run-exclude-tags", "c");
    cx.run_until_parked();
    model.read_with(cx, |m, _| {
        assert_eq!(
            (m.delay_ms(), m.tags(), m.exclude_tags()),
            (250, "a, b", "c")
        );
    });
    replace_input(cx, s.win, None, "run-delay", "2x5");
    cx.run_until_parked();
    assert_eq!(
        model.read_with(cx, |m, _| m.delay_ms()),
        25,
        "non-digits are stripped"
    );
    replace_input(cx, s.win, None, "run-delay", "");
    cx.run_until_parked();
    assert_eq!(
        model.read_with(cx, |m, _| m.delay_ms()),
        0,
        "a cleared delay means no delay"
    );
    replace_input(cx, s.win, None, "run-tags", "");
    replace_input(cx, s.win, None, "run-exclude-tags", "");
    replace_input(cx, s.win, None, "run-delay", "0");
    model.update(cx, |m, cx| {
        m.set_data_file(Some(rows.path().join("rows.json")), cx)
    });
    assert_eq!(label(cx, &s, "run-data-name").as_deref(), Some("rows.json"));
    click(cx, &s, "run-start");
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| matches!(m.state(), RunState::Finished(_)))
    });
    assert_eq!(
        label(cx, &s, ("run-iteration", 0usize)).as_deref(),
        Some("Iteration 1")
    );
    assert_eq!(
        label(cx, &s, ("run-iteration", 1usize)).as_deref(),
        Some("Iteration 2")
    );
    click(cx, &s, "run-data-clear");
    assert!(model.read_with(cx, |m, _| m.data_file().is_none()));
}

#[gpui_kit::test]
fn the_delay_input_is_labelled(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/a")),
    ]);
    let s = started(cx, &col);
    let _tab =
        s.ws.update(cx, |w, cx| w.open_runner(&s.root, ItemPath::root(), cx))
            .unwrap();
    frame(cx, s.win);
    assert_eq!(
        label(cx, &s, "run-delay-label").as_deref(),
        Some("Delay (ms)")
    );
    let (lbl, input) = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            (
                w.find("run-delay-label").bounds(),
                w.find("run-delay").bounds(),
            )
        })
        .unwrap();
    assert!(
        lbl.right() <= input.left(),
        "the label {lbl:?} precedes the input {input:?}"
    );
}

const CODE: &str = "bru.setVar('a', 1)";

fn scripted() -> tempfile::TempDir {
    let a = format!(
        "{}runtime:\n  scripts:\n    - type: before-request\n      code: \"{CODE}\"\n",
        request("a", "http://127.0.0.1:9/ok")
    );
    write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)])
}

#[gpui_kit::test]
fn an_idle_runner_lists_its_plan_with_run_on_the_right(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://127.0.0.1:9/a")),
        ("b.yml", &request("b", "http://127.0.0.1:9/b")),
    ]);
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| w.open_runner(&s.root, ItemPath::root(), cx))
        .expect("a runner tab");
    wait_real(cx, |cx| present(cx, s.win, ("run-plan-row", 1usize)));
    let mut names: Vec<String> = (0..2usize)
        .filter_map(|ix| label(cx, &s, ("run-plan-row", ix)))
        .collect();
    names.sort();
    assert_eq!(names, ["GET a", "GET b"]);
    assert_eq!(
        label(cx, &s, "run-plan-count").as_deref(),
        Some("2 requests")
    );
    for (id, text) in [
        ("run-delay-label", "Delay (ms)"),
        ("run-tags-label", "Tags"),
        ("run-exclude-tags-label", "Exclude tags"),
    ] {
        assert_eq!(label(cx, &s, id).as_deref(), Some(text), "{id}");
    }
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let run = w.find("run-start").bounds();
        let options = w.find("run-options").bounds();
        // Run holds the right edge in its own row above the options, so it never scrolls.
        assert!(
            run.right() >= options.right() - px(0.5) && run.bottom() <= options.top() + px(0.5),
            "run {run:?}, options {options:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_run_waiting_on_the_script_question_says_so(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = scripted();
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| w.open_runner(&s.root, ItemPath::root(), cx))
        .expect("a runner tab");
    click(cx, &s, "run-start");
    wait_real(cx, |cx| present(cx, s.win, "script-trust-run"));
    assert_eq!(
        label(cx, &s, "run-waiting").as_deref(),
        Some("Waiting for your answer")
    );
    cx.simulate_keystrokes(s.win, "enter");
    wait_real(cx, |cx| {
        !present(cx, s.win, "run-waiting") && present(cx, s.win, "run-summary")
    });
}

#[gpui_kit::test]
fn a_run_without_scripts_states_the_skipped_tests(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let ping = format!(
        "{}runtime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n  scripts:\n    - type: tests\n      code: \"test('status is 200', () => {{}})\"\n",
        request("Ping", &format!("{}/ping", server.uri()))
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &ping)]);
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| w.open_runner(&s.root, ItemPath::root(), cx))
        .expect("a runner tab");
    click(cx, &s, "run-start");
    wait_real(cx, |cx| present(cx, s.win, "script-trust-run"));
    // Enter answers Don't run.
    cx.simulate_keystrokes(s.win, "enter");
    wait_real(cx, |cx| present(cx, s.win, "run-summary"));
    assert_eq!(
        label(cx, &s, "run-summary").as_deref(),
        Some("1 request: 1 passed, 0 failed; 1 check skipped")
    );
    assert_eq!(
        label(cx, &s, ("run-checks", 0usize)).as_deref(),
        Some("1 passed, 0 failed, 1 skipped")
    );
}

#[gpui_kit::test]
fn the_results_table_names_each_requests_result(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let check = "runtime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n";
    let checked = format!(
        "{}{check}",
        request("Checked", &format!("{}/a", server.uri()))
    );
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &checked),
        (
            "b.yml",
            &request("Unchecked", &format!("{}/b", server.uri())),
        ),
    ]);
    let s = started(cx, &col);
    let (_, model) = run_to_end(cx, &s, ItemPath::root());
    let (a, _) = done_row(cx, &model, "a.yml");
    let (b, _) = done_row(cx, &model, "b.yml");
    assert_eq!(label(cx, &s, ("run-outcome", a)).as_deref(), Some("Passed"));
    assert_eq!(
        label(cx, &s, ("run-outcome", b)).as_deref(),
        Some("No tests")
    );
    assert_eq!(label(cx, &s, ("run-status", a)).as_deref(), Some("200"));
    assert!(label(cx, &s, ("run-time", b)).is_some_and(|t| !t.is_empty()));
    assert!(present(cx, s.win, ("run-method", a)));
    assert!(present(cx, s.win, "run-table-header"));
    assert!(
        !present(cx, s.win, ("run-plan-row", 0usize)),
        "the plan gives way to results"
    );
    assert_eq!(
        label(cx, &s, "run-summary").as_deref(),
        Some("2 requests: 2 passed (1 no tests), 0 failed")
    );
}

#[gpui_kit::test]
fn a_narrow_runner_keeps_names_and_labels_beside_their_inputs(cx: &mut TestAppContext) {
    use apinata_app::style::{RUN_NAME_MIN_WIDTH, at_zoom};
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    gpui_kit::VisualTestContext::from_window(s.win, cx)
        .simulate_resize(gpui_kit::size(gpui_kit::px(380.), gpui_kit::px(800.)));
    cx.run_until_parked();
    run_to_end(cx, &s, ItemPath::root());
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let area = w.find("tab-area").bounds();
        assert!(
            area.size.width < gpui_kit::px(420.),
            "a narrow pane {area:?}"
        );
        let min = at_zoom(RUN_NAME_MIN_WIDTH, 1.0);
        for id in [("run-name", 0usize).into(), "run-table-name".into()] {
            let name: gpui_kit::ElementId = id;
            let b = w.find(name.clone()).bounds();
            assert!(b.size.width >= min, "{name:?} {b:?} under {min:?}");
        }
        // Each label shares a line with its input and precedes it, however the row wraps.
        for (label, input) in [
            ("run-delay-label", "run-delay"),
            ("run-tags-label", "run-tags"),
            ("run-exclude-tags-label", "run-exclude-tags"),
        ] {
            let (l, i) = (w.find(label).bounds(), w.find(input).bounds());
            assert!(
                l.right() <= i.left() && l.center().y >= i.top() && l.center().y <= i.bottom(),
                "{label} {l:?} beside {input} {i:?}"
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_narrow_results_table_shows_a_scrollbar_for_hidden_columns(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    let size = |cx: &mut TestAppContext, w: f32| {
        gpui_kit::VisualTestContext::from_window(s.win, cx)
            .simulate_resize(gpui_kit::size(gpui_kit::px(w), gpui_kit::px(800.)));
        cx.run_until_parked();
        frame(cx, s.win);
        frame(cx, s.win);
    };
    size(cx, 1400.);
    run_to_end(cx, &s, ItemPath::root());
    size(cx, 1400.);
    assert!(
        !present(cx, s.win, "run-table-scrollbar"),
        "a table that fits"
    );
    size(cx, 420.);
    assert!(
        present(cx, s.win, "run-table-scrollbar"),
        "a table cut at the right"
    );
    // The cue must be on screen, not collapsed or outside the pane.
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    let (bar, pane) = cx
        .update_window(s.win, |_, w, _| {
            (w.find("run-table-scrollbar").bounds(), w.viewport_size())
        })
        .unwrap();
    assert!(bar.size.width > gpui_kit::px(0.) && bar.size.height > gpui_kit::px(0.));
    assert!(bar.origin.x >= gpui_kit::px(0.) && bar.right() <= pane.width + gpui_kit::px(1.));
    assert!(bar.bottom() <= pane.height, "inside the pane");
    size(cx, 1400.);
    assert!(!present(cx, s.win, "run-table-scrollbar"), "it fits again");
}

#[gpui_kit::test]
fn results_say_so_once_the_options_no_longer_match_them(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    let (_, model) = run_to_end(cx, &s, ItemPath::root());
    assert!(!present(cx, s.win, "run-stale"), "fresh results");
    model.update(cx, |m, cx| m.set_tags("smoke".into(), cx));
    assert_eq!(
        label(cx, &s, "run-stale").as_deref(),
        Some("Results from the last run. Run again to apply the changed options.")
    );
    model.update(cx, |m, cx| m.set_tags(String::new(), cx));
    assert!(!present(cx, s.win, "run-stale"), "the options match again");
    model.update(cx, |m, cx| m.set_exclude_tags("x".into(), cx));
    assert!(present(cx, s.win, "run-stale"));
}

#[gpui_kit::test]
fn changing_delay_or_bail_makes_the_results_stale(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    let (_, model) = run_to_end(cx, &s, ItemPath::root());
    assert!(!present(cx, s.win, "run-stale"), "fresh results");
    model.update(cx, |m, cx| m.set_delay_ms(5, cx));
    assert!(present(cx, s.win, "run-stale"), "a new delay");
    model.update(cx, |m, cx| m.set_delay_ms(0, cx));
    assert!(!present(cx, s.win, "run-stale"), "the delay matches again");
    model.update(cx, |m, cx| m.set_bail(true, cx));
    assert!(present(cx, s.win, "run-stale"), "a new bail");
    model.update(cx, |m, cx| m.set_bail(false, cx));
    assert!(!present(cx, s.win, "run-stale"), "bail matches again");
}

#[gpui_kit::test]
fn option_inputs_hint_with_examples_not_their_labels(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/a")),
    ]);
    let s = started(cx, &col);
    let tab =
        s.ws.update(cx, |w, cx| w.open_runner(&s.root, ItemPath::root(), cx))
            .unwrap();
    frame(cx, s.win);
    let hints = view(cx, &s, tab).read_with(cx, |v, cx| v.option_placeholders(cx));
    assert_eq!(hints, ["0", "smoke, auth", "slow", "1"]);
}

#[gpui_kit::test]
fn the_same_tags_written_differently_are_not_stale(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    let tab =
        s.ws.update(cx, |w, cx| w.open_runner(&s.root, ItemPath::root(), cx))
            .expect("a runner tab");
    let model = view(cx, &s, tab).read_with(cx, |v, _| v.model().clone());
    model.update(cx, |m, cx| m.set_exclude_tags("x, y".into(), cx));
    click(cx, &s, "run-start");
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| matches!(m.state(), RunState::Finished(_)))
    });
    model.update(cx, |m, cx| m.set_exclude_tags("x,y ".into(), cx));
    model.update(cx, |m, cx| m.set_tags(" , ".into(), cx));
    assert!(!present(cx, s.win, "run-stale"), "the same filter");
    model.update(cx, |m, cx| m.set_exclude_tags("x".into(), cx));
    assert!(present(cx, s.win, "run-stale"));
}

#[gpui_kit::test]
fn run_is_at_least_as_wide_as_the_send_slot(cx: &mut TestAppContext) {
    use apinata_app::style::{SEND_SLOT_WIDTH, at_zoom};
    let col = two_requests();
    let s = started(cx, &col);
    choose_menu(
        cx,
        s.win,
        "",
        NodeKind::Collection,
        MenuEntry::RunCollection,
    );
    frame(cx, s.win);
    let run = cx
        .update_window(s.win, |_, w, _| w.find("run-start").bounds())
        .unwrap();
    assert!(run.size.width >= at_zoom(SEND_SLOT_WIDTH, 1.0), "{run:?}");
}

// A run longer than the pane keeps its summary inside the pane: the table scrolls and the
// summary line keeps its own row at the bottom.
#[gpui_kit::test]
fn a_long_runs_summary_stays_inside_the_pane(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let names: Vec<String> = (0..40).map(|i| format!("r{i:02}.yml")).collect();
    let texts: Vec<String> = (0..40)
        .map(|i| request(&format!("R{i:02}"), &format!("{}/r{i}", server.uri())))
        .collect();
    let mut files: Vec<(&str, &str)> = vec![("opencollection.yml", ROOT)];
    files.extend(
        names
            .iter()
            .map(String::as_str)
            .zip(texts.iter().map(String::as_str)),
    );
    let col = write_tree(&files);
    let s = started(cx, &col);
    run_to_end(cx, &s, ItemPath::root());
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        let pane = w.find("runner").bounds();
        let summary = w.find("run-summary").bounds();
        assert!(
            summary.bottom() <= pane.bottom() && summary.top() >= pane.top(),
            "summary {summary:?} outside the pane {pane:?}"
        );
    })
    .unwrap();
}

// With the console dock open the runner pane, summary included, ends at or above the dock's
// top, at the default window size and in a short window.
#[gpui_kit::test]
fn a_long_runs_summary_stays_above_the_open_dock(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let names: Vec<String> = (0..15).map(|i| format!("r{i:02}.yml")).collect();
    let texts: Vec<String> = (0..15)
        .map(|i| request(&format!("R{i:02}"), &format!("{}/r{i}", server.uri())))
        .collect();
    let mut files: Vec<(&str, &str)> = vec![("opencollection.yml", ROOT)];
    files.extend(
        names
            .iter()
            .map(String::as_str)
            .zip(texts.iter().map(String::as_str)),
    );
    let col = write_tree(&files);
    let s = started(cx, &col);
    run_to_end(cx, &s, ItemPath::root());
    VisualTestContext::from_window(s.win, cx).dispatch_action(ToggleConsole);
    let check = |cx: &mut TestAppContext, label: &str| {
        frame(cx, s.win);
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            let dock = w.find("console-dock").bounds();
            let pane = w.find("runner").bounds();
            let summary = w.find("run-summary").bounds();
            assert!(
                pane.bottom() <= dock.top() && summary.bottom() <= dock.top(),
                "{label}: runner {pane:?} summary {summary:?} past the dock {dock:?}"
            );
        })
        .unwrap();
    };
    check(cx, "default size");
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1000.), px(480.)));
    check(cx, "short window");
}

#[gpui_kit::test]
fn a_long_plan_scrolls_above_the_open_dock_before_a_run(cx: &mut TestAppContext) {
    use gpui_kit::{InputEvent as _, ScrollDelta, ScrollWheelEvent, point};
    http_ready(cx);
    let names: Vec<String> = (0..15).map(|i| format!("r{i:02}.yml")).collect();
    let texts: Vec<String> = (0..15)
        .map(|i| request(&format!("R{i:02}"), &format!("http://127.0.0.1:9/r{i}")))
        .collect();
    let mut files: Vec<(&str, &str)> = vec![("opencollection.yml", ROOT)];
    files.extend(
        names
            .iter()
            .map(String::as_str)
            .zip(texts.iter().map(String::as_str)),
    );
    let col = write_tree(&files);
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| w.open_runner(&s.root, ItemPath::root(), cx))
        .expect("a runner tab");
    wait_real(cx, |cx| present(cx, s.win, ("run-plan-row", 14usize)));
    VisualTestContext::from_window(s.win, cx).dispatch_action(ToggleConsole);
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1000.), px(480.)));
    frame(cx, s.win);
    // Before any scroll the list itself must end above the dock, not merely be clipped there.
    let row0 = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            let dock = w.find("console-dock").bounds();
            let pane = w.find("runner").bounds();
            let plan = w.try_find("run-plan").map(|e| e.bounds());
            assert!(
                pane.bottom() <= dock.top() && plan.is_some_and(|p| p.bottom() <= pane.bottom()),
                "runner {pane:?} plan {plan:?} past the dock {dock:?}"
            );
            w.find(("run-plan-row", 0usize)).bounds()
        })
        .unwrap();
    // The wheel then brings the last planned row into view above the dock.
    cx.update_window(s.win, |_, w, cx| {
        w.dispatch_event(
            ScrollWheelEvent {
                position: row0.center(),
                delta: ScrollDelta::Lines(point(0., -1000.)),
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        w.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let plan = w.find("run-plan").bounds();
        let last = w.find(("run-plan-row", 14usize)).bounds();
        assert!(
            last.bottom() <= plan.bottom() && last.top() >= plan.top(),
            "last planned row {last:?} outside the plan list {plan:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn many_load_errors_keep_the_summary_above_the_open_dock(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let names: Vec<String> = (0..15).map(|i| format!("bad{i:02}.yml")).collect();
    let good = request("ok", &format!("{}/ok", server.uri()));
    let mut files: Vec<(&str, &str)> = vec![("opencollection.yml", ROOT), ("ok.yml", &good)];
    files.extend(names.iter().map(|n| (n.as_str(), "info: [")));
    let col = write_tree(&files);
    let s = started(cx, &col);
    run_to_end(cx, &s, ItemPath::root());
    assert!(present(cx, s.win, ("run-load-error", 14usize)));
    VisualTestContext::from_window(s.win, cx).dispatch_action(ToggleConsole);
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1000.), px(480.)));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let dock = w.find("console-dock").bounds();
        let pane = w.find("runner").bounds();
        let notes = w.try_find("run-notes").map(|e| e.bounds());
        let summary = w.find("run-summary").bounds();
        assert!(
            pane.bottom() <= dock.top()
                && summary.bottom() <= pane.bottom()
                && notes.is_some_and(|n| n.bottom() <= summary.top()),
            "runner {pane:?} notes {notes:?} summary {summary:?} past the dock {dock:?}"
        );
    })
    .unwrap();
}
