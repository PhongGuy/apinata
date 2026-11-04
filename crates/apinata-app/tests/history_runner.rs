//! Runner sends land in history, tagged as a run, under the same scrubbing and switch.

use crate::support;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use apinata_app::app::AppState;
use apinata_app::last_response::{ResponseStore, install};
use apinata_app::runner_model::{RunState, RunnerModel};
use apinata_app::workspace::Workspace;
use apinata_core::{MemoryStore, SecretStore as _};
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use support::*;
use wiremock::ResponseTemplate;

const SECRET: &str = "Run-S3cret-5";

/// `api/one` and `api/two`, each calling `<uri>/<name><query>`.
fn suite(uri: &str, query: &str) -> tempfile::TempDir {
    let req = |name: &str, seq: u32| {
        format!(
            "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"{uri}/{name}{query}\"\n"
        )
    };
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("api/folder.yml", &folder("api")),
        ("api/one.yml", &req("one", 1)),
        ("api/two.yml", &req("two", 2)),
    ])
}

fn run_to_end(
    cx: &mut TestAppContext,
    ws: &Entity<Workspace>,
    root: &Path,
    target: &str,
    data: Option<PathBuf>,
) {
    let (ws, root) = (ws.clone(), root.to_path_buf());
    let m = cx.new(|_| RunnerModel::new(ws, root, ip(target)));
    if data.is_some() {
        m.update(cx, |m, cx| m.set_data_file(data, cx));
    }
    assert!(m.update(cx, |m, cx| m.start(cx)));
    wait_real(cx, |cx| m.read_with(cx, |m, _| !m.is_running()));
    assert!(m.read_with(cx, |m, _| matches!(m.state(), RunState::Finished(_))));
}

#[gpui_kit::test]
fn a_runs_requests_are_kept_with_the_run_origin(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = suite(&server.uri(), "");
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    cx.update(|cx| install(store.clone(), cx));
    run_to_end(cx, &ws, &root, "api", None);
    wait_real(cx, |_| {
        store.recent(std::slice::from_ref(&root), 10).len() == 2
    });
    let mut got: Vec<(String, Option<String>)> = store
        .recent(std::slice::from_ref(&root), 10)
        .into_iter()
        .map(|r| (r.item.as_str().to_owned(), r.origin))
        .collect();
    got.sort();
    let run = Some("run".to_owned());
    assert_eq!(
        got,
        [
            ("api/one.yml".to_owned(), run.clone()),
            ("api/two.yml".to_owned(), run)
        ]
    );
    assert_eq!(store.request_json(&root, &ip("api/one.yml"), 0), None);
}

#[gpui_kit::test]
fn a_run_with_remembering_off_keeps_nothing(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = suite(&server.uri(), "");
    let s = started(cx, &col);
    cx.update(|cx| {
        let settings = cx.global::<AppState>().settings.clone();
        settings.update(cx, |p, cx| p.update(cx, |v| v.remember_responses = false));
    });
    run_to_end(cx, &s.ws, &s.root, "api", None);
    cx.run_until_parked();
    let store = cx.update(|cx| cx.global::<ResponseStore>().clone());
    assert!(!store.dir().exists());
}

#[gpui_kit::test]
fn a_run_never_keeps_a_secret_variable(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = suite(&server.uri(), "?t={{token}}");
    std::fs::create_dir_all(col.path().join("environments")).unwrap();
    std::fs::write(
        col.path().join("environments/dev.yml"),
        "name: dev\nvariables:\n  - name: token\n    secret: true\n",
    )
    .unwrap();
    let c = apinata_core::Collection::open(col.path()).unwrap();
    let secrets = MemoryStore::new();
    secrets.set(&c.secret_key("dev", "token"), SECRET).unwrap();
    let s = started_with_store(cx, &col, Arc::new(secrets));
    s.ws.update(cx, |w, cx| {
        w.select_environment(&s.root, Some("dev".into()), cx)
    });
    cx.run_until_parked();
    run_to_end(cx, &s.ws, &s.root, "api", None);
    let store = ResponseStore::new(s.files.path().join("responses"));
    wait_real(cx, |_| {
        store.recent(std::slice::from_ref(&s.root), 10).len() == 2
    });
    let leaks: Vec<String> = std::fs::read_dir(store.dir())
        .unwrap()
        .filter_map(|e| {
            let path = e.ok()?.path();
            std::fs::read_to_string(&path)
                .ok()?
                .contains(SECRET)
                .then(|| path.display().to_string())
        })
        .collect();
    assert!(leaks.is_empty(), "{leaks:?}");
}

#[gpui_kit::test]
fn a_refused_request_is_not_kept(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = suite("http://127.0.0.1:1", "");
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    cx.update(|cx| install(store.clone(), cx));
    run_to_end(cx, &ws, &root, "api", None);
    cx.run_until_parked();
    assert!(store.recent(std::slice::from_ref(&root), 10).is_empty());
}
#[gpui_kit::test]
fn a_data_driven_run_keeps_every_round_of_the_same_request(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let a = format!(
        "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{}/a/{{{{id}}}}\"\n",
        server.uri()
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let data = tempfile::tempdir().unwrap();
    let csv = data.path().join("rows.csv");
    std::fs::write(&csv, "id\n1\n2\n3\n").unwrap();
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    cx.update(|cx| install(store.clone(), cx));
    run_to_end(cx, &ws, &root, "", Some(csv));
    wait_real(cx, |_| store.history(&root, &ip("a.yml")).len() == 3);
    let urls: Vec<String> = store
        .history(&root, &ip("a.yml"))
        .into_iter()
        .map(|k| k.url)
        .collect();
    let base = server.uri();
    assert_eq!(
        urls,
        [
            format!("{base}/a/3"),
            format!("{base}/a/2"),
            format!("{base}/a/1")
        ]
    );
    let origins: Vec<Option<String>> = store
        .recent(std::slice::from_ref(&root), 10)
        .into_iter()
        .map(|r| r.origin)
        .collect();
    assert_eq!(
        origins,
        [
            Some("run".to_owned()),
            Some("run".to_owned()),
            Some("run".to_owned())
        ]
    );
}
