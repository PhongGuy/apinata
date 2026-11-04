use crate::support;

use apinata_app::response_pane::ResponsePane;
use apinata_core::Outcome;
use gpui_kit::{AppContext, TestAppContext, WindowOptions};
use support::*;
use wiremock::ResponseTemplate;

const CAPTURE: &str = "runtime:\n  actions:\n    - type: set-variable\n      phase: after-response\n      selector:\n        expression: res.body.id\n        method: jsonq\n      variable:\n        name: id\n        scope: runtime\n    - type: set-variable\n      phase: after-response\n      selector:\n        expression: res('id')\n        method: jsonq\n      variable:\n        name: viaQuery\n        scope: runtime\n";

#[gpui_kit::test]
fn a_send_captures_into_the_collection_and_the_next_send_uses_it(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(
        cx,
        ResponseTemplate::new(201).set_body_raw("{\"id\":42}", "application/json"),
    );
    let col = collection_for(&server.uri(), CAPTURE);
    let (_state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    assert!(model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx)));
    assert_eq!(finished(cx, &model).outcome, Outcome::Passed);
    let vars = ws
        .read_with(cx, |w, cx| w.runtime_vars(&root, cx))
        .expect("an open collection");
    let listed: Vec<(String, Option<String>)> = vars
        .entries()
        .iter()
        .map(|e| (e.name().to_owned(), e.value().map(str::to_owned)))
        .collect();
    assert_eq!(listed, [("id".to_owned(), Some("42".to_owned()))]);
    let next = format!("{}/item/{{{{id}}}}", server.uri());
    ws.update(cx, |w, cx| w.send(&root, edit_url("a.yml", &next), cx));
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    finished(cx, &model);
    assert_eq!(served(cx, &server), ["/ok", "/item/42"]);
}

#[gpui_kit::test]
fn a_capture_survives_an_environment_switch(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(
        cx,
        ResponseTemplate::new(201).set_body_raw("{\"id\":42}", "application/json"),
    );
    let col = collection_for(&server.uri(), CAPTURE);
    std::fs::create_dir_all(col.path().join("environments")).unwrap();
    std::fs::write(
        col.path().join("environments/dev.yml"),
        "name: dev\nvariables:\n  - name: other\n    value: x\n",
    )
    .unwrap();
    let (_state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    assert!(model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx)));
    finished(cx, &model);
    ws.update(cx, |w, cx| {
        w.send(
            &root,
            apinata_app::command::Command::SelectEnvironment(Some("dev".into())),
            cx,
        )
    });
    let next = format!("{}/item/{{{{id}}}}", server.uri());
    ws.update(cx, |w, cx| w.send(&root, edit_url("a.yml", &next), cx));
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    finished(cx, &model);
    assert_eq!(served(cx, &server), ["/ok", "/item/42"]);
}

#[gpui_kit::test]
fn the_response_pane_lists_a_skipped_capture_with_the_warnings(cx: &mut TestAppContext) {
    let s = send_once(
        cx,
        ResponseTemplate::new(201).set_body_raw("{\"id\":42}", "application/json"),
        CAPTURE,
    );
    support::init(cx);
    let model = s.model.clone();
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |p, cx| p.warnings(cx)),
        ["Post-response variable \"viaQuery\" not set: \"res('id')\" needs a JavaScript runtime"]
    );
    assert!(present(cx, win, ("response-warning", 0usize)));
}
