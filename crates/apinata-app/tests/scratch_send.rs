//! Sending from the scratch draft with no collection open.

use crate::support;

use apinata_app::command::{Command, Edit, Target};
use apinata_app::persist::Paths;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

fn server_and_globals(cx: &mut TestAppContext) -> (wiremock::MockServer, tempfile::TempDir) {
    http_ready(cx);
    let server = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let files = blank_data_dir();
    let paths = Paths::in_dir(files.path());
    std::fs::create_dir_all(&paths.global_environments).unwrap();
    let g = format!(
        "name: g\nvariables:\n  - name: base\n    value: {}\n",
        server.uri()
    );
    std::fs::write(paths.global_environments.join("g.yml"), g).unwrap();
    (server, files)
}

#[gpui_kit::test]
fn the_button_opens_an_untitled_draft(cx: &mut TestAppContext) {
    let (_server, files) = server_and_globals(cx);
    let s = started_blank(cx, files);
    cx.update_window(s.win, |_, w, cx| {
        use gpui_kit::test::TestWindowExt as _;
        w.within("no-collection-actions")
            .click(("empty-action", 0usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    let (root, item) = active_draft(cx, &s.ws);
    assert!(s.ws.read_with(cx, |w, _| w.is_scratch(&root)));
    assert!(item.is_draft());
}

#[gpui_kit::test]
fn a_scratch_draft_sends_to_localhost_with_a_global_variable(cx: &mut TestAppContext) {
    let (server, files) = server_and_globals(cx);
    let s = started_blank(cx, files);
    s.ws.update(cx, |w, cx| w.new_scratch_request(cx));
    cx.run_until_parked();
    s.ws.update(cx, |w, cx| {
        w.select_global_environment(Some("g".into()), cx)
    });
    cx.run_until_parked();
    let (root, item) = active_draft(cx, &s.ws);
    s.ws.update(cx, |w, cx| {
        w.send(
            &root,
            Command::Edit {
                target: Target::Request(item.clone()),
                edit: Edit::Url("{{base}}/ok".into()),
            },
            cx,
        )
    });
    cx.run_until_parked();
    let model = sender(cx, &s.ws);
    assert!(model.update(cx, |m, cx| m.send(&root, item.clone(), cx)));
    let result = finished(cx, &model);
    assert_eq!(served(cx, &server), ["/ok"]);
    assert_eq!(result.response.as_ref().map(|r| r.status()), Some(200));
}

#[gpui_kit::test]
fn an_unknown_variable_is_an_error_not_a_literal(cx: &mut TestAppContext) {
    let (server, files) = server_and_globals(cx);
    let s = started_blank(cx, files);
    s.ws.update(cx, |w, cx| w.new_scratch_request(cx));
    cx.run_until_parked();
    let (root, item) = active_draft(cx, &s.ws);
    s.ws.update(cx, |w, cx| {
        w.send(
            &root,
            Command::Edit {
                target: Target::Request(item.clone()),
                edit: Edit::Url("{{base}}/ok".into()),
            },
            cx,
        )
    });
    cx.run_until_parked();
    let model = sender(cx, &s.ws);
    model.update(cx, |m, cx| m.send(&root, item, cx));
    let result = finished(cx, &model);
    assert!(result.response.is_none());
    assert!(served(cx, &server).is_empty());
}
