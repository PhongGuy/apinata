use crate::support::body_server::{Reply, Tail, pattern};
use crate::support::*;
use apinata_app::download_model::{DownloadModel, DownloadState};
use apinata_app::send_model::SendState;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use std::path::Path;
use std::time::Duration;

const MIB: usize = 1024 * 1024;

fn model(
    cx: &mut TestAppContext,
    ws: &Entity<apinata_app::workspace::Workspace>,
) -> Entity<DownloadModel> {
    let ws = ws.clone();
    cx.new(|_| DownloadModel::new(ws))
}

fn running(cx: &mut TestAppContext, m: &Entity<DownloadModel>) -> bool {
    m.read_with(cx, |m, _| m.is_running())
}

// The 10 MiB kept body is post-processed by core, which takes seconds in a debug build, so
// this waits longer than `wait_real`.
fn finish(cx: &mut TestAppContext, m: &Entity<DownloadModel>) {
    let deadline = std::time::Instant::now() + Duration::from_secs(90);
    while running(cx, m) {
        assert!(std::time::Instant::now() < deadline, "timed out");
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn names(dir: &Path) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    found.sort();
    found
}

#[gpui_kit::test]
fn a_large_body_is_saved_byte_for_byte(cx: &mut TestAppContext) {
    http_ready(cx);
    // Past the 10 MiB body cap, so the saved file must hold more than memory does.
    let body = pattern(12 * MIB);
    let server = bodies(cx, vec![("/ok", Reply::plain(body.clone()))]);
    let col = collection_for(server.url("").trim_end_matches('/'), "");
    let (_state, ws, root) = open_dir(cx, &col);
    let out = tempfile::tempdir().unwrap();
    let file = out.path().join("big.bin");
    let dl = model(cx, &ws);
    assert!(dl.update(cx, |m, cx| m.start(&root, ip("a.yml"), file.clone(), cx)));
    finish(cx, &dl);
    dl.read_with(cx, |m, _| match m.state() {
        DownloadState::Done { status, bytes } => {
            assert_eq!((*status, *bytes), (200, body.len() as u64));
        }
        _ => panic!("not done: {}", m.describe()),
    });
    assert!(std::fs::read(&file).unwrap() == body, "file differs");
    assert_eq!(names(out.path()), ["big.bin"]);
}

#[gpui_kit::test]
fn a_download_leaves_the_send_model_and_its_state_alone(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = bodies(cx, vec![("/ok", Reply::plain(pattern(MIB)))]);
    let col = collection_for(server.url("").trim_end_matches('/'), "");
    let (_state, ws, root) = open_dir(cx, &col);
    let sender = sender(cx, &ws);
    let out = tempfile::tempdir().unwrap();
    let dl = model(cx, &ws);
    dl.update(cx, |m, cx| {
        m.start(&root, ip("a.yml"), out.path().join("x.bin"), cx)
    });
    wait_real(cx, |cx| !running(cx, &dl));
    sender.read_with(cx, |m, _| assert!(matches!(m.state(), SendState::Idle)));
    assert_eq!(names(out.path()), ["x.bin"]);
}

#[gpui_kit::test]
fn cancel_removes_the_partial_and_is_not_an_error(cx: &mut TestAppContext) {
    http_ready(cx);
    let reply = Reply::plain(pattern(30 * MIB)).paced(64 * 1024, Duration::from_millis(5));
    let server = bodies(cx, vec![("/ok", reply)]);
    let col = collection_for(server.url("").trim_end_matches('/'), "");
    let (_state, ws, root) = open_dir(cx, &col);
    let out = tempfile::tempdir().unwrap();
    let file = out.path().join("big.bin");
    let dl = model(cx, &ws);
    dl.update(cx, |m, cx| m.start(&root, ip("a.yml"), file.clone(), cx));
    wait_real(cx, |cx| {
        dl.read_with(
            cx,
            |m, _| matches!(m.state(), DownloadState::Running { received, .. } if *received > 0),
        )
    });
    dl.update(cx, |m, cx| m.cancel(cx));
    wait_real(cx, |cx| !running(cx, &dl));
    dl.read_with(cx, |m, _| {
        assert!(
            matches!(m.state(), DownloadState::Cancelled),
            "{}",
            m.describe()
        );
    });
    wait_real(cx, |_| names(out.path()).is_empty());
}

#[gpui_kit::test]
fn a_server_that_drops_the_connection_fails_the_download(cx: &mut TestAppContext) {
    http_ready(cx);
    let reply = Reply::short(pattern(MIB), 4 * MIB, Tail::Close);
    let server = bodies(cx, vec![("/ok", reply)]);
    let col = collection_for(server.url("").trim_end_matches('/'), "");
    let (_state, ws, root) = open_dir(cx, &col);
    let out = tempfile::tempdir().unwrap();
    let dl = model(cx, &ws);
    dl.update(cx, |m, cx| {
        m.start(&root, ip("a.yml"), out.path().join("big.bin"), cx)
    });
    wait_real(cx, |cx| !running(cx, &dl));
    dl.read_with(cx, |m, _| match m.state() {
        DownloadState::Failed(msg) => assert!(!msg.is_empty()),
        _ => panic!("not failed: {}", m.describe()),
    });
    assert!(names(out.path()).is_empty());
}

#[gpui_kit::test]
fn a_download_adds_nothing_to_the_session_log(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = bodies(cx, vec![("/ok", Reply::plain(pattern(MIB)))]);
    let col = collection_for(server.url("").trim_end_matches('/'), "");
    let (_state, ws, root) = open_dir(cx, &col);
    let log = cx
        .update(|cx| apinata_app::log_feed::shared(cx))
        .expect("install sets the log");
    let out = tempfile::tempdir().unwrap();
    let dl = model(cx, &ws);
    dl.update(cx, |m, cx| {
        m.start(&root, ip("a.yml"), out.path().join("x.bin"), cx)
    });
    finish(cx, &dl);
    assert!(matches!(
        dl.read_with(cx, |m, _| m.state().clone()),
        DownloadState::Done { .. }
    ));
    assert!(log.network_since(0).entries.is_empty());
}

#[gpui_kit::test]
fn cancel_while_planning_ends_cancelled_with_no_file(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = bodies(cx, vec![("/ok", Reply::plain(pattern(MIB)))]);
    let col = collection_for(server.url("").trim_end_matches('/'), "");
    let (_state, ws, root) = open_dir(cx, &col);
    let out = tempfile::tempdir().unwrap();
    let dl = model(cx, &ws);
    dl.update(cx, |m, cx| {
        m.start(&root, ip("a.yml"), out.path().join("x.bin"), cx);
        m.cancel(cx);
    });
    cx.run_until_parked();
    std::thread::sleep(Duration::from_millis(200));
    cx.run_until_parked();
    dl.read_with(cx, |m, _| {
        assert!(
            matches!(m.state(), DownloadState::Cancelled),
            "{}",
            m.describe()
        )
    });
    assert!(names(out.path()).is_empty());
}

fn prompted(uri: &str) -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", &format!("{uri}/{{{{?code}}}}"))),
    ])
}

fn asked(cx: &mut TestAppContext, s: &Started) -> bool {
    cx.run_until_parked();
    frame(cx, s.win);
    present(cx, s.win, "prompt-vars-dialog")
}

#[gpui_kit::test]
fn cancelling_the_prompt_ends_cancelled_with_no_file_and_no_request(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let col = prompted(&server.uri());
    let s = started(cx, &col);
    let out = tempfile::tempdir().unwrap();
    let dl = model(cx, &s.ws);
    dl.update(cx, |m, cx| {
        m.start(&s.root, ip("a.yml"), out.path().join("x.bin"), cx)
    });
    wait_real(cx, |cx| asked(cx, &s));
    cx.update_window(s.win, |_, w, cx| w.click("prompt-vars-cancel", cx))
        .unwrap();
    wait_real(cx, |cx| !running(cx, &dl));
    dl.read_with(cx, |m, _| {
        assert!(
            matches!(m.state(), DownloadState::Cancelled),
            "{}",
            m.describe()
        )
    });
    assert!(names(out.path()).is_empty());
    assert!(served(cx, &server).is_empty());
}

#[gpui_kit::test]
fn the_cancel_button_at_the_prompt_ends_cancelled(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let col = prompted(&server.uri());
    let s = started(cx, &col);
    let out = tempfile::tempdir().unwrap();
    let dl = model(cx, &s.ws);
    dl.update(cx, |m, cx| {
        m.start(&s.root, ip("a.yml"), out.path().join("x.bin"), cx)
    });
    wait_real(cx, |cx| asked(cx, &s));
    dl.update(cx, |m, cx| m.cancel(cx));
    cx.run_until_parked();
    dl.read_with(cx, |m, _| {
        assert!(
            matches!(m.state(), DownloadState::Cancelled),
            "{}",
            m.describe()
        )
    });
    assert!(names(out.path()).is_empty());
    assert!(served(cx, &server).is_empty());
}
