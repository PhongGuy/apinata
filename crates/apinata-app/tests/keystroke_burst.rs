//! The pilot-shaped body burst of issue 405, timed in the whole app: two tabs open, j.yml's
//! Body focused, every key in one update and then one frame, with and without a response
//! shown. A measurement, not a gate: run it with
//! `cargo test -p apinata-app --test integration keystroke_burst:: -- --ignored --nocapture`.

use crate::support;

use apinata_app::request_editor::Section;
use apinata_core::BodySpec;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Keystroke, TestAppContext};
use std::time::{Duration, Instant};
use support::*;

const KEYS: usize = 1000;

fn body_request(url: &str) -> String {
    format!(
        "info:\n  name: j\n  type: http\n  seq: 2\nhttp:\n  method: POST\n  url: \"{url}\"\n  body:\n    type: text\n    data: 'x'\n"
    )
}

// The machine is shared, so each number is printed with the load it was taken under.
fn load() -> String {
    std::fs::read_to_string("/proc/loadavg")
        .map(|l| l.split_whitespace().take(3).collect::<Vec<_>>().join(" "))
        .unwrap_or_default()
}

// The app with a.yml and j.yml open, j.yml active, optionally sent once, its Body focused.
fn body_focused(cx: &mut TestAppContext, url: &str, send: bool) -> (Started, tempfile::TempDir) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://127.0.0.1:9/a")),
        ("j.yml", &body_request(url)),
    ]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    let tab = open_tab(cx, &s.ws, &s.root, "j.yml");
    frame(cx, s.win);
    if send {
        let area = s.shell.read_with(cx, |sh, _| sh.tab_area().clone());
        let editor = area.read_with(cx, |a, _| a.editor(tab).cloned()).unwrap();
        cx.update_window(s.win, |_, w, cx| w.click("send", cx))
            .unwrap();
        let model = editor.read_with(cx, |e, _| e.send_model().clone());
        finished(cx, &model);
        frame(cx, s.win);
    }
    cx.update_window(s.win, |_, w, cx| {
        w.click(("section", Section::Body as usize), cx);
    })
    .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.within("body").click_at("body-code", code_at(), cx);
        w.press("end", cx);
    })
    .unwrap();
    frame(cx, s.win);
    (s, col)
}

fn burst(cx: &mut TestAppContext, s: &Started) -> Duration {
    let t = Instant::now();
    cx.update_window(s.win, |_, w, cx| {
        for _ in 0..KEYS {
            let mut key = Keystroke::parse("a").unwrap();
            key.key_char = Some("a".into());
            w.dispatch_keystroke(key, cx);
        }
    })
    .unwrap();
    frame(cx, s.win);
    t.elapsed()
}

fn report(cx: &mut TestAppContext, s: &Started, what: &str) {
    let before = load();
    let took = burst(cx, s);
    match &view_of(cx, &s.ws, &s.root, "j.yml").body {
        BodySpec::Text(t) => assert_eq!(t.len(), 1 + KEYS, "every key reached the body"),
        other => panic!("{other:?}"),
    }
    println!(
        "keystroke_burst {what}: {KEYS} keys in {took:.2?} (load {before} -> {})",
        load()
    );
}

#[gpui_kit::test]
#[ignore = "measurement; run with -- --ignored --nocapture"]
fn a_burst_without_a_response(cx: &mut TestAppContext) {
    let (s, _col) = body_focused(cx, "http://127.0.0.1:9/j", false);
    report(cx, &s, "no response");
}

#[gpui_kit::test]
#[ignore = "measurement; run with -- --ignored --nocapture"]
fn a_burst_with_a_response(cx: &mut TestAppContext) {
    http_ready(cx);
    let rows: Vec<_> = (0..200)
        .map(|i| serde_json::json!({ "id": i, "name": format!("row {i}"), "ok": i % 2 == 0 }))
        .collect();
    let server = mock_server(
        cx,
        wiremock::ResponseTemplate::new(200).set_body_json(serde_json::Value::Array(rows)),
    );
    let (s, _col) = body_focused(cx, &format!("{}/j", server.uri()), true);
    report(cx, &s, "response shown");
}
