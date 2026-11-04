use crate::support;

use apinata_app::command::{Command, TreeOp};
use apinata_app::send_model::SendState;
use apinata_core::Outcome;
use gpui_kit::AppContext as _;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt;
use std::time::Duration;
use support::*;
use wiremock::ResponseTemplate;

type Served = (
    wiremock::MockServer,
    tempfile::TempDir,
    tempfile::TempDir,
    gpui_kit::Entity<apinata_app::workspace::Workspace>,
    std::path::PathBuf,
    gpui_kit::AnyWindowHandle,
    gpui_kit::Entity<apinata_app::request_editor::RequestEditor>,
);

fn served_editor(cx: &mut TestAppContext, response: ResponseTemplate) -> Served {
    http_ready(cx);
    let server = mock_server(cx, response);
    let col = collection_for(&server.uri(), "");
    let (state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    (server, col, state, ws, root, win, ed)
}

fn outcome(
    cx: &mut TestAppContext,
    ed: &gpui_kit::Entity<apinata_app::request_editor::RequestEditor>,
) -> Outcome {
    let model = ed.read_with(cx, |e, _| e.send_model().clone());
    finished(cx, &model).outcome
}

#[gpui_kit::test]
fn clicking_send_shows_the_response(cx: &mut TestAppContext) {
    let (server, _c, _s, _ws, _root, win, ed) = served_editor(cx, ResponseTemplate::new(200));
    cx.update_window(win, |_, w, cx| w.click("send", cx))
        .unwrap();
    assert_eq!(outcome(cx, &ed), Outcome::Passed);
    assert_eq!(served(cx, &server), ["/ok"]);
    assert!(present(cx, win, "response-status"));
}

#[gpui_kit::test]
fn the_url_typed_just_before_send_is_sent(cx: &mut TestAppContext) {
    let (server, _c, _s, _ws, _root, win, ed) = served_editor(cx, ResponseTemplate::new(200));
    type_at_end(cx, win, None, "url", "/x");
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click("send", cx)
    })
    .unwrap();
    outcome(cx, &ed);
    assert_eq!(served(cx, &server), ["/ok/x"]);
}

#[gpui_kit::test]
fn secondary_enter_in_the_url_sends(cx: &mut TestAppContext) {
    let (server, _c, _s, _ws, _root, win, ed) = served_editor(cx, ResponseTemplate::new(200));
    cx.update_window(win, |_, w, cx| {
        w.click("url", cx);
        w.press(&key("enter"), cx)
    })
    .unwrap();
    outcome(cx, &ed);
    assert_eq!(served(cx, &server), ["/ok"]);
    assert_eq!(
        ed.read_with(cx, |e, cx| e.url(cx)),
        format!("{}/ok", server.uri())
    );
}

#[gpui_kit::test]
fn the_send_button_becomes_cancel_while_sending(cx: &mut TestAppContext) {
    let slow = ResponseTemplate::new(200).set_delay(Duration::from_secs(30));
    let (server, _c, _s, _ws, _root, win, ed) = served_editor(cx, slow);
    cx.update_window(win, |_, w, cx| w.click("send", cx))
        .unwrap();
    assert!(present(cx, win, "cancel"));
    assert!(!present(cx, win, "send"));
    wait_real(cx, |cx| served(cx, &server).len() == 1);
    cx.update_window(win, |_, w, cx| w.click("cancel", cx))
        .unwrap();
    assert_eq!(outcome(cx, &ed), Outcome::Cancelled);
    assert!(present(cx, win, "send"));
}

#[gpui_kit::test]
fn a_renamed_request_sends_under_its_new_path(cx: &mut TestAppContext) {
    let (server, _c, _s, ws, root, win, ed) = served_editor(cx, ResponseTemplate::new(200));
    let rename = Command::Tree(TreeOp::Rename {
        item: ip("a.yml"),
        name: "b".into(),
        rename_file: true,
    });
    ws.update(cx, |w, cx| w.send(&root, rename, cx));
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click("send", cx)
    })
    .unwrap();
    assert_eq!(outcome(cx, &ed), Outcome::Passed);
    assert_eq!(served(cx, &server), ["/ok"]);
    let model = ed.read_with(cx, |e, _| e.send_model().clone());
    let path = model.read_with(cx, |m, _| match m.state() {
        SendState::Done(r) => r.path.as_str().to_owned(),
        _ => String::new(),
    });
    assert_eq!(path, "b.yml");
}

#[gpui_kit::test]
fn the_url_keeps_its_width_while_sending(cx: &mut TestAppContext) {
    let slow = ResponseTemplate::new(200).set_delay(Duration::from_secs(30));
    let (server, _c, _s, _ws, _root, win, _ed) = served_editor(cx, slow);
    let width = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find("url").bounds().size.width
        })
        .unwrap()
    };
    let idle = width(cx);
    cx.update_window(win, |_, w, cx| w.click("send", cx))
        .unwrap();
    wait_real(cx, |cx| served(cx, &server).len() == 1);
    assert_eq!(width(cx), idle);
    cx.update_window(win, |_, w, _| {
        let slot = w.find("send-slot").bounds();
        let cancel = w.find("cancel").bounds();
        assert!(
            cancel.left() >= slot.left() && cancel.right() <= slot.right(),
            "{cancel:?} outside {slot:?}"
        );
    })
    .unwrap();
    cx.update_window(win, |_, w, cx| w.click("cancel", cx))
        .unwrap();
}

fn send_by(cx: &mut TestAppContext, key: &str) {
    let (server, _c, _s, _ws, _root, win, ed) = served_editor(cx, ResponseTemplate::new(200));
    tab_to(cx, win, "send-button");
    cx.update_window(win, |_, w, cx| w.press(key, cx)).unwrap();
    assert_eq!(outcome(cx, &ed), Outcome::Passed, "{key} sends");
    assert_eq!(served(cx, &server), ["/ok"]);
}

#[gpui_kit::test]
fn enter_on_send_sends(cx: &mut TestAppContext) {
    send_by(cx, "enter");
}

#[gpui_kit::test]
fn space_on_send_sends(cx: &mut TestAppContext) {
    send_by(cx, "space");
}

fn cancel_by(cx: &mut TestAppContext, key: &str) {
    let slow = ResponseTemplate::new(200).set_delay(Duration::from_secs(30));
    let (server, _c, _s, _ws, _root, win, ed) = served_editor(cx, slow);
    cx.update_window(win, |_, w, cx| w.click("send", cx))
        .unwrap();
    wait_real(cx, |cx| served(cx, &server).len() == 1);
    tab_to(cx, win, "cancel-button");
    cx.update_window(win, |_, w, cx| w.press(key, cx)).unwrap();
    assert_eq!(outcome(cx, &ed), Outcome::Cancelled, "{key} cancels");
}

#[gpui_kit::test]
fn enter_on_cancel_cancels(cx: &mut TestAppContext) {
    cancel_by(cx, "enter");
}

#[gpui_kit::test]
fn space_on_cancel_cancels(cx: &mut TestAppContext) {
    cancel_by(cx, "space");
}
