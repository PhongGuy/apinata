use crate::support;

use apinata_app::runner_model::{RunState, RunnerModel};
use apinata_core::ItemPath;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext};
use std::path::PathBuf;
use support::*;
use wiremock::ResponseTemplate;

struct Open {
    s: Started,
    model: Entity<RunnerModel>,
    _col: tempfile::TempDir,
    _data: tempfile::TempDir,
    file: PathBuf,
}

// A runner tab for a one-request collection, with `text` written as the data file `name`.
fn open(cx: &mut TestAppContext, uri: &str, name: &str, text: &str) -> Open {
    let col = collection_for(uri, "");
    let data = write_tree(&[(name, text)]);
    let file = data.path().join(name);
    let s = started(cx, &col);
    let tab =
        s.ws.update(cx, |w, cx| w.open_runner(&s.root, ItemPath::root(), cx))
            .unwrap();
    let view = s
        .shell
        .read_with(cx, |v, cx| v.tab_area().read(cx).runner(tab).cloned())
        .expect("a runner view");
    let model = view.read_with(cx, |v, _| v.model().clone());
    frame(cx, s.win);
    Open {
        s,
        model,
        _col: col,
        _data: data,
        file,
    }
}

fn choose(cx: &mut TestAppContext, o: &Open) {
    let file = o.file.clone();
    o.model.update(cx, |m, cx| m.set_data_file(Some(file), cx));
    cx.run_until_parked();
}

fn label(cx: &mut TestAppContext, o: &Open, id: impl Into<gpui_kit::ElementId>) -> Option<String> {
    let id = id.into();
    frame(cx, o.s.win);
    cx.update_window(o.s.win, |_, w, _| {
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

fn placeholder(cx: &mut TestAppContext, o: &Open, id: &'static str) -> Option<String> {
    frame(cx, o.s.win);
    cx.update_window(o.s.win, |_, w, _| {
        w.try_find(id)
            .and_then(|e| e.placeholder().map(str::to_owned))
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_good_csv_shows_its_row_count_and_columns(cx: &mut TestAppContext) {
    let o = open(
        cx,
        "http://127.0.0.1:9",
        "data.csv",
        "id,name\n1,a\n2,b\n3,c\n",
    );
    assert_eq!(placeholder(cx, &o, "run-iterations").as_deref(), Some("1"));
    choose(cx, &o);
    assert_eq!(
        label(cx, &o, "run-data-summary").as_deref(),
        Some("3 rows: id, name")
    );
    assert_eq!(
        placeholder(cx, &o, "run-iterations").as_deref(),
        Some("All 3 rows")
    );
    assert!(!present(cx, o.s.win, "run-data-error"));
    assert!(!present(cx, o.s.win, "run-data-ring"));
}

#[gpui_kit::test]
fn an_unterminated_quote_shows_the_error_before_run(cx: &mut TestAppContext) {
    let o = open(cx, "http://127.0.0.1:9", "bad.csv", "id,name\n1,\"beta\n");
    choose(cx, &o);
    assert!(matches!(
        o.model.read_with(cx, |m, _| m.state().clone()),
        RunState::Idle
    ));
    let error = label(cx, &o, "run-data-error").expect("an error under the field");
    assert!(
        error.starts_with("Cannot read data file bad.csv"),
        "{error}"
    );
    assert!(error.contains("unterminated quoted field"), "{error}");
    assert!(
        present(cx, o.s.win, "run-data-ring"),
        "the field is outlined"
    );
    assert!(!present(cx, o.s.win, "run-data-summary"));
    assert_eq!(placeholder(cx, &o, "run-iterations").as_deref(), Some("1"));
}

#[gpui_kit::test]
fn json_that_is_not_an_array_has_its_own_error(cx: &mut TestAppContext) {
    let o = open(cx, "http://127.0.0.1:9", "rows.json", "{\"id\": 1}");
    choose(cx, &o);
    let error = label(cx, &o, "run-data-error").expect("an error under the field");
    assert!(error.contains("must be an array of objects"), "{error}");
    assert!(present(cx, o.s.win, "run-data-ring"));
}

#[gpui_kit::test]
fn a_typed_count_caps_the_rows_and_the_plan_count(cx: &mut TestAppContext) {
    http_ready(cx);
    let o = open(
        cx,
        "http://127.0.0.1:9",
        "data.csv",
        "id,name\n1,a\n2,b\n3,c\n",
    );
    choose(cx, &o);
    o.model.update(cx, |m, cx| m.refresh_preview(cx));
    wait_real(cx, |cx| {
        o.model.read_with(cx, |m, _| m.preview().len() == 1)
    });
    assert_eq!(o.model.read_with(cx, |m, _| m.preview_rounds()), Some(3));
    assert!(!present(cx, o.s.win, "run-iterations-hint"));
    replace_input(cx, o.s.win, None, "run-iterations", "2");
    cx.run_until_parked();
    assert_eq!(o.model.read_with(cx, |m, _| m.preview_rounds()), Some(2));
    assert_eq!(
        label(cx, &o, "run-iterations-hint").as_deref(),
        Some("Runs the first 2 of 3 rows")
    );
    assert_eq!(
        label(cx, &o, "run-plan-count").as_deref(),
        Some("1 request for each of 2 data rows")
    );
    replace_input(cx, o.s.win, None, "run-iterations", "7");
    cx.run_until_parked();
    assert_eq!(o.model.read_with(cx, |m, _| m.preview_rounds()), Some(3));
    assert_eq!(
        label(cx, &o, "run-iterations-hint").as_deref(),
        Some("Runs all 3 rows")
    );
}

#[gpui_kit::test]
fn the_iteration_header_shows_the_rows_values(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let long = "abcdefghijklmnopqrstuvwxyz0123";
    let csv = format!("id,name,token,extra\n1,alpha,{long},x\n2,beta,short,y\n");
    let o = open(cx, &server.uri(), "data.csv", &csv);
    choose(cx, &o);
    assert!(o.model.update(cx, |m, cx| m.start(cx)));
    wait_real(cx, |cx| {
        o.model
            .read_with(cx, |m, _| matches!(m.state(), RunState::Finished(_)))
    });
    // At most three columns, and a long value is cut short.
    assert_eq!(
        label(cx, &o, ("run-iteration", 0usize)).as_deref(),
        Some("Iteration 1: id=1, name=alpha, token=abcdefghijklmnopqrstuvw\u{2026}")
    );
    assert_eq!(
        label(cx, &o, ("run-iteration", 1usize)).as_deref(),
        Some("Iteration 2: id=2, name=beta, token=short")
    );
}
