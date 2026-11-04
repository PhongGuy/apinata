use crate::support;

use apinata_app::runner_model::{PLAN_UNANSWERED, RowState, RunState, RunnerModel};
use apinata_app::workspace::Workspace;
use apinata_core::Outcome;
use gpui_kit::{AppContext, Entity, TestAppContext};
use std::path::Path;
use std::time::Duration;
use support::*;
use wiremock::ResponseTemplate;

const FAILS: &str = "runtime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n";

/// `api/one`, `api/two` and the root's `other`, each calling `<uri>/<name>`, with `extra` appended.
fn suite(uri: &str, extra: &str) -> tempfile::TempDir {
    let req = |name: &str, seq: u32| {
        format!(
            "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"{uri}/{name}\"\n{extra}"
        )
    };
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("api/folder.yml", &folder("api")),
        ("api/one.yml", &req("one", 1)),
        ("api/two.yml", &req("two", 2)),
        ("other.yml", &req("other", 3)),
    ])
}

fn runner(
    cx: &mut TestAppContext,
    ws: &Entity<Workspace>,
    root: &Path,
    target: &str,
) -> Entity<RunnerModel> {
    let (ws, root) = (ws.clone(), root.to_path_buf());
    cx.new(|_| RunnerModel::new(ws, root, ip(target)))
}

fn outcomes(cx: &mut TestAppContext, m: &Entity<RunnerModel>) -> Vec<Option<Outcome>> {
    m.read_with(cx, |m, _| {
        m.rows()
            .iter()
            .map(|r| match &r.state {
                RowState::Done(x) => Some(x.outcome),
                _ => None,
            })
            .collect()
    })
}

fn run_to_end(cx: &mut TestAppContext, m: &Entity<RunnerModel>) -> apinata_core::Summary {
    assert!(m.update(cx, |m, cx| m.start(cx)));
    wait_real(cx, |cx| m.read_with(cx, |m, _| !m.is_running()));
    m.read_with(cx, |m, _| match m.state() {
        RunState::Finished(s) => s.clone(),
        other => panic!("{other:?}"),
    })
}

#[gpui_kit::test]
fn a_folder_run_lists_its_queue_then_each_outcome_in_order(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = suite(&server.uri(), "");
    let (_state, ws, root) = open_dir(cx, &col);
    let m = runner(cx, &ws, &root, "api");
    let summary = run_to_end(cx, &m);
    let paths: Vec<String> = m.read_with(cx, |m, _| {
        m.rows()
            .iter()
            .map(|r| r.path.as_str().to_owned())
            .collect()
    });
    assert_eq!(paths, ["api/one.yml", "api/two.yml"]);
    assert_eq!(
        outcomes(cx, &m),
        [Some(Outcome::Passed), Some(Outcome::Passed)]
    );
    assert_eq!(served(cx, &server), ["/one", "/two"]);
    assert_eq!((summary.requests, summary.passed), (2, 2));
    assert!(
        !m.update(cx, |m, cx| {
            m.start(cx);
            m.start(cx)
        }),
        "a second start while running is refused"
    );
}

// A cancelled run can leave its request in flight after its test ends; that request must
// never be counted by a later test's server, or `served` assertions flake.
#[gpui_kit::test]
fn a_late_request_to_an_earlier_server_is_not_served_to_the_next(cx: &mut TestAppContext) {
    use std::io::{Read, Write};
    http_ready(cx);
    let earlier = mock_server(cx, ResponseTemplate::new(200));
    let stale = *earlier.address();
    drop(earlier);
    let next = mock_server(cx, ResponseTemplate::new(200));
    // The earlier server may refuse the connection outright; only a delivery matters.
    if let Ok(mut socket) = std::net::TcpStream::connect(stale) {
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let _ = socket.write_all(b"GET /stale HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
        let _ = socket.read_to_end(&mut Vec::new());
    }
    assert!(served(cx, &next).is_empty(), "{:?}", served(cx, &next));
}

#[gpui_kit::test]
fn bail_stops_a_collection_run_after_the_first_failure(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(500));
    let col = suite(&server.uri(), FAILS);
    let (_state, ws, root) = open_dir(cx, &col);
    let m = runner(cx, &ws, &root, "");
    m.update(cx, |m, cx| m.set_bail(true, cx));
    run_to_end(cx, &m);
    let got = outcomes(cx, &m);
    assert_eq!(got.len(), 3);
    assert_eq!(got[0], Some(Outcome::Failed));
    assert!(
        got[1..].iter().all(|o| *o == Some(Outcome::Skipped)),
        "{got:?}"
    );
    assert_eq!(served(cx, &server).len(), 1);
    m.update(cx, |m, cx| m.set_bail(false, cx));
    run_to_end(cx, &m);
    assert!(outcomes(cx, &m).iter().all(|o| *o == Some(Outcome::Failed)));
}

#[gpui_kit::test]
fn cancel_ends_the_run_with_a_summary(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(
        cx,
        ResponseTemplate::new(200).set_delay(Duration::from_secs(30)),
    );
    let col = suite(&server.uri(), "");
    let (_state, ws, root) = open_dir(cx, &col);
    let m = runner(cx, &ws, &root, "api");
    assert!(m.update(cx, |m, cx| m.start(cx)));
    wait_real(cx, |cx| {
        m.read_with(cx, |m, _| {
            m.rows()
                .first()
                .is_some_and(|r| matches!(r.state, RowState::Running))
        })
    });
    m.update(cx, |m, cx| m.cancel(cx));
    wait_real(cx, |cx| m.read_with(cx, |m, _| !m.is_running()));
    assert_eq!(
        outcomes(cx, &m),
        [Some(Outcome::Cancelled), Some(Outcome::Skipped)]
    );
    assert!(m.read_with(
        cx,
        |m, _| matches!(m.state(), RunState::Finished(s) if s.cancelled == 1)
    ));
}

#[gpui_kit::test]
fn load_errors_come_first_and_a_missing_target_fails(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = suite(&server.uri(), "");
    std::fs::write(col.path().join("api/bad.yml"), "info: [").unwrap();
    let (_state, ws, root) = open_dir(cx, &col);
    let m = runner(cx, &ws, &root, "api");
    run_to_end(cx, &m);
    let errors = m.read_with(cx, |m, _| m.load_errors().to_vec());
    assert!(
        matches!(errors.as_slice(), [e] if e.starts_with("api/bad.yml: ")),
        "{errors:?}"
    );
    let missing = runner(cx, &ws, &root, "nope");
    assert!(missing.update(cx, |m, cx| m.start(cx)));
    cx.run_until_parked();
    assert!(missing.read_with(
        cx,
        |m, _| matches!(m.state(), RunState::Failed(e) if e.contains("nope"))
    ));
}

#[gpui_kit::test]
fn a_plan_that_is_never_answered_fails(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let m = runner(cx, &ws, &root, "");
    assert!(m.update(cx, |m, cx| m.start(cx)));
    // Closing drops the actor with the Plan still queued, so its reply channel closes.
    ws.update(cx, |w, cx| w.close(&root, cx));
    cx.run_until_parked();
    assert!(m.read_with(
        cx,
        |m, _| matches!(m.state(), RunState::Failed(e) if e == PLAN_UNANSWERED)
    ));
    assert!(
        m.update(cx, |m, cx| m.start(cx)),
        "a failed run can start again"
    );
    cx.run_until_parked();
    assert!(
        m.read_with(
            cx,
            |m, _| matches!(m.state(), RunState::Failed(e) if e == PLAN_UNANSWERED)
        ),
        "the root is not open"
    );
}

// A run keeps the environment it started under: a value its script set there survives a
// switch mid-run, and the newly selected environment never sees it.
#[gpui_kit::test]
fn a_run_keeps_its_script_env_values_across_an_environment_switch(cx: &mut TestAppContext) {
    use apinata_app::command::Command;
    use apinata_core::OverlayLayer;
    http_ready(cx);
    let slow = ResponseTemplate::new(200).set_delay(Duration::from_millis(500));
    let server = mock_server(cx, slow);
    let one = format!(
        "info:\n  name: one\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{}/one\"\nruntime:\n  scripts:\n    - type: before-request\n      code: \"bru.setEnvVar('region', 'from-script')\"\n",
        server.uri()
    );
    let two = format!(
        "info:\n  name: two\n  type: http\n  seq: 2\nhttp:\n  method: GET\n  url: \"{}/{{{{region}}}}\"\n",
        server.uri()
    );
    let env =
        |name: &str| format!("name: {name}\nvariables:\n  - name: region\n    value: {name}\n");
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("one.yml", &one),
        ("two.yml", &two),
        ("environments/staging.yml", &env("staging")),
        ("environments/prod.yml", &env("prod")),
    ]);
    let root = col.path().to_path_buf();
    let s = started_with(cx, &col, |p| allow_scripts(p, &root));
    let select = |cx: &mut TestAppContext, name: &str| {
        let select = Command::SelectEnvironment(Some(name.into()));
        s.ws.update(cx, |w, cx| w.send(&s.root, select, cx));
        cx.run_until_parked();
    };
    select(cx, "staging");
    let m = runner(cx, &s.ws, &s.root, "");
    assert!(m.update(cx, |m, cx| m.start(cx)));
    // Switch while the first request waits on its response, after its script set `region`.
    wait_real(cx, |cx| !served(cx, &server).is_empty());
    select(cx, "prod");
    wait_real(cx, |cx| m.read_with(cx, |m, _| !m.is_running()));
    assert_eq!(served(cx, &server), ["/one", "/from-script"]);
    let overlay =
        s.ws.read_with(cx, |w, cx| w.env_overlay(&s.root, cx))
            .expect("open");
    let rows = |o: &apinata_core::EnvOverlay| {
        (o.entries().iter())
            .map(|v| (v.layer(), v.name().to_owned(), v.value().map(str::to_owned)))
            .collect::<Vec<_>>()
    };
    assert!(rows(&overlay).is_empty(), "{:?}", rows(&overlay));
    // Switching back finds staging's script value where the run left it.
    select(cx, "staging");
    assert_eq!(
        rows(&overlay),
        [(
            OverlayLayer::Environment,
            "region".to_owned(),
            Some("from-script".to_owned())
        )]
    );
}

// A run in flight when its environment is deleted writes nothing back: the script it runs
// after the delete is dropped, and an environment recreated under that name starts empty.
#[gpui_kit::test]
fn a_run_never_resurrects_a_deleted_environments_script_values(cx: &mut TestAppContext) {
    use apinata_app::command::{Command, EnvOp};
    http_ready(cx);
    let slow = ResponseTemplate::new(200).set_delay(Duration::from_millis(500));
    let server = mock_server(cx, slow);
    let request = |name: &str, seq: u32| {
        format!(
            "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"{}/{name}\"\nruntime:\n  scripts:\n    - type: before-request\n      code: \"bru.setEnvVar('{name}', 'from-script')\"\n",
            server.uri()
        )
    };
    let staging = "name: staging\nvariables:\n  - name: region\n    value: staging\n";
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("one.yml", &request("one", 1)),
        ("two.yml", &request("two", 2)),
        ("environments/staging.yml", staging),
    ]);
    let root = col.path().to_path_buf();
    let s = started_with(cx, &col, |p| allow_scripts(p, &root));
    let send = |cx: &mut TestAppContext, command: Command| {
        s.ws.update(cx, |w, cx| w.send(&s.root, command, cx));
        cx.run_until_parked();
    };
    send(cx, Command::SelectEnvironment(Some("staging".into())));
    let m = runner(cx, &s.ws, &s.root, "");
    assert!(m.update(cx, |m, cx| m.start(cx)));
    // Delete while the first request waits on its response; the second request's script runs after.
    wait_real(cx, |cx| !served(cx, &server).is_empty());
    let delete = EnvOp::Delete {
        name: "staging".into(),
    };
    send(cx, Command::Env(delete));
    wait_real(cx, |cx| m.read_with(cx, |m, _| !m.is_running()));
    assert_eq!(served(cx, &server), ["/one", "/two"]);
    let create = EnvOp::Create {
        name: "staging".into(),
    };
    send(cx, Command::Env(create));
    send(cx, Command::SelectEnvironment(Some("staging".into())));
    let overlay =
        s.ws.read_with(cx, |w, cx| w.env_overlay(&s.root, cx))
            .expect("open");
    assert!(overlay.env_names().is_empty(), "{:?}", overlay.env_names());
}

fn data(text: &str, name: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = write_tree(&[(name, text)]);
    let path = dir.path().join(name);
    (dir, path)
}

#[gpui_kit::test]
fn tags_filter_the_rows(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let u = server.uri();
    let tagged = |name: &str, tags: &str| {
        format!(
            "info:\n  name: {name}\n  type: http\n  seq: 1\n{tags}http:\n  method: GET\n  url: \"{u}/{name}\"\n"
        )
    };
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("one.yml", &tagged("one", "  tags:\n    - smoke\n")),
        ("two.yml", &tagged("two", "")),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let m = runner(cx, &ws, &root, "");
    m.update(cx, |m, cx| m.set_tags(" smoke ,".into(), cx));
    run_to_end(cx, &m);
    let paths: Vec<String> = m.read_with(cx, |m, _| {
        m.rows()
            .iter()
            .map(|r| r.path.as_str().to_owned())
            .collect()
    });
    assert_eq!(paths, ["one.yml"]);
    assert_eq!(served(cx, &server), ["/one"]);
}

#[gpui_kit::test]
fn a_data_file_repeats_the_rows_per_iteration(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let a = format!(
        "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{}/a/{{{{id}}}}\"\n",
        server.uri()
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let (_dir, csv) = data("id\n1\n2\n", "rows.csv");
    let (_state, ws, root) = open_dir(cx, &col);
    let m = runner(cx, &ws, &root, "");
    m.update(cx, |m, cx| m.set_data_file(Some(csv), cx));
    let summary = run_to_end(cx, &m);
    assert_eq!(summary.requests, 2);
    let marks: Vec<Option<usize>> =
        m.read_with(cx, |m, _| m.rows().iter().map(|r| r.iteration).collect());
    assert_eq!(marks, [Some(0), Some(1)]);
    assert_eq!(
        outcomes(cx, &m),
        [Some(Outcome::Passed), Some(Outcome::Passed)]
    );
    assert_eq!(served(cx, &server), ["/a/1", "/a/2"]);
}

#[gpui_kit::test]
fn a_bad_data_file_fails_before_anything_is_sent(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = suite(&server.uri(), "");
    let (_dir, csv) = data("a,b\n1\n", "bad.csv");
    let (_state, ws, root) = open_dir(cx, &col);
    let m = runner(cx, &ws, &root, "");
    m.update(cx, |m, cx| m.set_data_file(Some(csv), cx));
    assert!(m.update(cx, |m, cx| m.start(cx)));
    wait_real(cx, |cx| m.read_with(cx, |m, _| !m.is_running()));
    let failed = m.read_with(cx, |m, _| match m.state() {
        RunState::Failed(why) => why.clone(),
        other => panic!("{other:?}"),
    });
    assert!(failed.contains("line 2"), "{failed}");
    assert!(served(cx, &server).is_empty());
}

fn preview_paths(cx: &mut TestAppContext, m: &Entity<RunnerModel>) -> Vec<String> {
    m.read_with(cx, |m, _| {
        m.preview()
            .iter()
            .map(|r| r.path.as_str().to_owned())
            .collect()
    })
}

#[gpui_kit::test]
fn an_idle_runner_previews_its_plan_and_follows_the_tags(cx: &mut TestAppContext) {
    http_ready(cx);
    let tagged = |name: &str, tags: &str| {
        format!(
            "info:\n  name: {name}\n  type: http\n  seq: 1\n{tags}http:\n  method: GET\n  url: \"http://127.0.0.1:9/{name}\"\n"
        )
    };
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("one.yml", &tagged("one", "  tags:\n    - smoke\n")),
        ("two.yml", &tagged("two", "")),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let m = runner(cx, &ws, &root, "");
    m.update(cx, |m, cx| m.refresh_preview(cx));
    wait_real(cx, |cx| preview_paths(cx, &m).len() == 2);
    assert_eq!(preview_paths(cx, &m), ["one.yml", "two.yml"]);
    let first = m.read_with(cx, |m, _| m.preview()[0].clone());
    assert_eq!((first.method.as_str(), first.name.as_str()), ("GET", "one"));
    m.update(cx, |m, cx| m.set_tags("smoke".into(), cx));
    assert_eq!(preview_paths(cx, &m), ["one.yml"]);
    m.update(cx, |m, cx| {
        m.set_tags(String::new(), cx);
        m.set_exclude_tags("smoke".into(), cx);
    });
    assert_eq!(preview_paths(cx, &m), ["two.yml"]);
    assert!(matches!(
        m.read_with(cx, |m, _| m.state().clone()),
        RunState::Idle
    ));
}

#[gpui_kit::test]
fn the_preview_counts_data_rows(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = suite("http://127.0.0.1:9", "");
    let (_dir, csv) = data("id\n1\n2\n", "rows.csv");
    let (_state, ws, root) = open_dir(cx, &col);
    let m = runner(cx, &ws, &root, "api");
    m.update(cx, |m, cx| m.refresh_preview(cx));
    wait_real(cx, |cx| preview_paths(cx, &m).len() == 2);
    assert_eq!(m.read_with(cx, |m, _| m.preview_rounds()), None);
    m.update(cx, |m, cx| m.set_data_file(Some(csv), cx));
    assert_eq!(m.read_with(cx, |m, _| m.preview_rounds()), Some(2));
    m.update(cx, |m, cx| m.set_data_file(None, cx));
    assert_eq!(m.read_with(cx, |m, _| m.preview_rounds()), None);
}

fn counter_collection(uri: &str) -> tempfile::TempDir {
    let a = format!(
        "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{uri}/a/{{{{iteration}}}}\"\n"
    );
    write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)])
}

#[gpui_kit::test]
fn the_iteration_count_is_unset_by_default_and_never_below_one(cx: &mut TestAppContext) {
    let col = counter_collection("http://127.0.0.1:9");
    let (_state, ws, root) = open_dir(cx, &col);
    let m = runner(cx, &ws, &root, "");
    assert_eq!(m.read_with(cx, |m, _| m.iteration_count()), None);
    m.update(cx, |m, cx| m.set_iteration_count(Some(4), cx));
    assert_eq!(m.read_with(cx, |m, _| m.iteration_count()), Some(4));
    m.update(cx, |m, cx| m.set_iteration_count(Some(0), cx));
    assert_eq!(m.read_with(cx, |m, _| m.iteration_count()), Some(1));
    m.update(cx, |m, cx| m.set_iteration_count(None, cx));
    assert_eq!(m.read_with(cx, |m, _| m.iteration_count()), None);
}

#[gpui_kit::test]
fn a_count_of_one_without_data_exposes_the_index(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = counter_collection(&server.uri());
    let (_state, ws, root) = open_dir(cx, &col);
    let m = runner(cx, &ws, &root, "");
    m.update(cx, |m, cx| m.set_iteration_count(Some(1), cx));
    run_to_end(cx, &m);
    let marks: Vec<Option<usize>> =
        m.read_with(cx, |m, _| m.rows().iter().map(|r| r.iteration).collect());
    assert_eq!(marks, [Some(0)]);
    assert_eq!(served(cx, &server), ["/a/0"]);
}

#[gpui_kit::test]
fn a_count_of_one_with_data_runs_exactly_one_row(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = counter_collection(&server.uri());
    let (_dir, csv) = data("id\n1\n2\n3\n", "rows.csv");
    let (_state, ws, root) = open_dir(cx, &col);
    let m = runner(cx, &ws, &root, "");
    m.update(cx, |m, cx| m.set_data_file(Some(csv), cx));
    m.update(cx, |m, cx| m.set_iteration_count(Some(1), cx));
    run_to_end(cx, &m);
    assert_eq!(served(cx, &server), ["/a/0"]);
}

#[gpui_kit::test]
fn an_iteration_count_repeats_the_run_without_a_data_file(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = counter_collection(&server.uri());
    let (_state, ws, root) = open_dir(cx, &col);
    let m = runner(cx, &ws, &root, "");
    m.update(cx, |m, cx| m.set_iteration_count(Some(3), cx));
    run_to_end(cx, &m);
    let marks: Vec<Option<usize>> =
        m.read_with(cx, |m, _| m.rows().iter().map(|r| r.iteration).collect());
    assert_eq!(marks, [Some(0), Some(1), Some(2)]);
    assert_eq!(served(cx, &server), ["/a/0", "/a/1", "/a/2"]);
}

#[gpui_kit::test]
fn an_iteration_count_caps_a_data_file_and_unset_does_not(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = counter_collection(&server.uri());
    let (_dir, csv) = data("id\n1\n2\n3\n", "rows.csv");
    let (_state, ws, root) = open_dir(cx, &col);
    let m = runner(cx, &ws, &root, "");
    m.update(cx, |m, cx| m.set_data_file(Some(csv), cx));
    run_to_end(cx, &m);
    assert_eq!(served(cx, &server).len(), 3, "unset keeps every row");
    m.update(cx, |m, cx| m.set_iteration_count(Some(2), cx));
    assert!(m.read_with(cx, |m, _| m.results_stale()));
    run_to_end(cx, &m);
    assert_eq!(m.read_with(cx, |m, _| m.rows().len()), 2);
}

#[gpui_kit::test]
fn a_retrying_event_marks_its_row_until_the_next_event_of_the_run(cx: &mut TestAppContext) {
    use apinata_core::{RequestResult, RunEvent};
    use std::sync::Arc;
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = suite(&server.uri(), "");
    let (_state, ws, root) = open_dir(cx, &col);
    let m = runner(cx, &ws, &root, "api");
    let summary = run_to_end(cx, &m);
    let retrying = |cx: &mut TestAppContext, ix| m.read_with(cx, |m, _| m.retrying(ix));
    let path = ip("api/two.yml");
    let wait = |attempt| RunEvent::Retrying {
        index: 1,
        path: ip("api/two.yml"),
        attempt,
        of: 3,
        delay: Duration::from_millis(5),
    };
    assert_eq!(retrying(cx, 1), None);
    m.update(cx, |m, cx| m.apply(wait(2), cx));
    assert_eq!(retrying(cx, 1), Some((2, 3)));
    assert_eq!(retrying(cx, 0), None, "only the row that waits");
    m.update(cx, |m, cx| m.apply(wait(3), cx));
    assert_eq!(retrying(cx, 1), Some((3, 3)), "a later pause replaces it");
    m.update(cx, |m, cx| {
        m.apply(
            RunEvent::RequestStarted {
                index: 1,
                path: path.clone(),
            },
            cx,
        )
    });
    assert_eq!(retrying(cx, 1), None, "the next send clears it");
    m.update(cx, |m, cx| m.apply(wait(2), cx));
    let result = Arc::new(RequestResult::from_parts(
        path.clone(),
        "two",
        Outcome::Passed,
        None,
        Vec::new(),
    ));
    m.update(cx, |m, cx| {
        m.apply(RunEvent::RequestFinished { index: 1, result }, cx)
    });
    assert_eq!(retrying(cx, 1), None, "a finished request clears it");
    m.update(cx, |m, cx| m.apply(wait(2), cx));
    m.update(cx, |m, cx| m.apply(RunEvent::Finished { summary }, cx));
    assert_eq!(retrying(cx, 1), None, "the end of the run clears it");
}

#[gpui_kit::test]
fn a_retried_result_names_its_send_count(cx: &mut TestAppContext) {
    use apinata_app::runner_view::result_label;
    use apinata_core::{
        CancelHandle, Collection, ContextOptions, ExecOptions, Executor, RunOptions, VarContext,
        run,
    };
    http_ready(cx);
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(wiremock::MockServer::start());
    handle.block_on(
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .mount(&server),
    );
    handle.block_on(
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(ResponseTemplate::new(200))
            .mount(&server),
    );
    let col = suite(&server.uri(), "");
    let c = Collection::open(col.path()).unwrap();
    let plan = c.plan(&col.path().join("other.yml")).unwrap();
    let ctx = VarContext::new(&c, ContextOptions::new()).unwrap();
    let exec = std::sync::Arc::new(Executor::new(ExecOptions::new()).unwrap());
    let opts = RunOptions::new()
        .retries(1)
        .retry_delay(Duration::from_millis(5));
    let report = handle.block_on(run(
        plan,
        ctx,
        exec,
        opts,
        CancelHandle::new(),
        Box::new(|_| {}),
    ));
    let r = &report.results[0];
    assert_eq!(r.attempts(), 2);
    assert_eq!(result_label(r), "No tests (2 attempts)");
}
