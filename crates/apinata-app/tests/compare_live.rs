use crate::support;
use apinata_app::compare_source::CompareSource;
use apinata_app::last_response::{ResponseStore, install};
use gpui_kit::AppContext as _;
use gpui_kit::TestAppContext;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use support::*;

#[gpui_kit::test]
fn a_new_send_moves_previous_and_recomputes(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(
        cx,
        wiremock::ResponseTemplate::new(200).set_body_string("{\"n\":1}"),
    );
    let col = collection_for(&server.uri(), "");
    let (state, ws, root) = open_dir(cx, &col);
    cx.update(|cx| install(ResponseStore::new(state.path().join("responses")), cx));
    let (win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    for _ in 0..2 {
        let r = root.clone();
        model.update(cx, |m, cx| m.send(&r, ip("a.yml"), cx));
        finished(cx, &model);
        wait_real(cx, |cx| pane.read_with(cx, |p, _| !p.history().is_empty()));
    }
    wait_real(cx, |cx| pane.read_with(cx, |p, _| p.history().len() == 2));
    pane.update(cx, |p, cx| p.compare_with(CompareSource::Previous, cx));
    wait_real(cx, |cx| pane.read_with(cx, |p, _| p.comparison().is_some()));
    let first = pane.read_with(cx, |p, _| p.comparison().unwrap());
    // A third send: the pane recomputes against the second.
    let r = root.clone();
    model.update(cx, |m, cx| m.send(&r, ip("a.yml"), cx));
    finished(cx, &model);
    wait_real(cx, |cx| pane.read_with(cx, |p, _| p.history().len() == 3));
    wait_real(cx, |cx| {
        pane.read_with(cx, |p, _| {
            p.comparison()
                .is_some_and(|c| !std::sync::Arc::ptr_eq(&c, &first))
        })
    });
    frame(cx, win);
}
#[gpui_kit::test]
fn switching_tabs_removes_the_compare_tab(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(
        cx,
        wiremock::ResponseTemplate::new(200).set_body_string("{\"n\":1}"),
    );
    let url = format!("{}/ok", server.uri());
    let (a, b) = (request("a", &url), request("b", &url));
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a), ("b.yml", &b)]);
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let editor = editor_of(cx, &s, tab);
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    for sends in 1..=2usize {
        cx.update_window(s.win, |_, w, cx| w.click("send", cx))
            .unwrap();
        wait_real(cx, |cx| {
            pane.read_with(cx, |p, _| p.history().len() == sends)
        });
    }
    gpui_kit::VisualTestContext::from_window(s.win, cx)
        .dispatch_action(apinata_app::tab_area::Compare);
    wait_real(cx, |cx| pane.read_with(cx, |p, _| p.comparison().is_some()));
    assert_eq!(
        pane.read_with(cx, |p, _| p.compare_source()),
        Some(CompareSource::Previous)
    );
    open_tab(cx, &s.ws, &s.root, "b.yml");
    frame(cx, s.win);
    assert_eq!(pane.read_with(cx, |p, _| p.compare_source()), None);
}
#[test]
fn the_compare_command_is_in_the_palette() {
    let cmd = apinata_app::commands::registry()
        .into_iter()
        .find(|c| c.id == "request.compare_response")
        .expect("registered");
    assert_eq!(cmd.label, "Compare response");
    assert!(cmd.in_palette);
    assert!(cmd.default_chords.is_empty());
}

#[gpui_kit::test]
fn a_failed_resend_removes_the_comparison(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(
        cx,
        wiremock::ResponseTemplate::new(200).set_body_string("{\"n\":1}"),
    );
    let col = collection_for(&server.uri(), "");
    let (state, ws, root) = open_dir(cx, &col);
    cx.update(|cx| install(ResponseStore::new(state.path().join("responses")), cx));
    let (_win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    for sends in 1..=2usize {
        let r = root.clone();
        model.update(cx, |m, cx| m.send(&r, ip("a.yml"), cx));
        finished(cx, &model);
        wait_real(cx, |cx| {
            pane.read_with(cx, |p, _| p.history().len() == sends)
        });
    }
    pane.update(cx, |p, cx| p.compare_with(CompareSource::Previous, cx));
    wait_real(cx, |cx| pane.read_with(cx, |p, _| p.comparison().is_some()));
    // With the server gone the next send fails without a response.
    drop(server);
    let r = root.clone();
    model.update(cx, |m, cx| m.send(&r, ip("a.yml"), cx));
    finished(cx, &model);
    wait_real(cx, |cx| {
        pane.read_with(cx, |p, _| p.compare_source().is_none())
    });
    assert!(pane.read_with(cx, |p, _| p.comparison().is_none()));
}

#[gpui_kit::test]
fn a_changed_value_row_is_tinted_with_the_warning_fill(cx: &mut TestAppContext) {
    use apinata_app::compare_rows::ComparePalette;
    use apinata_app::theme::{Status, diff_fill, status_fill};
    let col = collection_for("http://127.0.0.1:1", "");
    let s = started(cx, &col);
    frame(cx, s.win);
    let (changed, warning, added) = cx
        .update_window(s.win, |_, w, cx| {
            let p = ComparePalette::new(&[], w, cx);
            (
                p.changed(),
                status_fill(Status::Warning, cx),
                diff_fill(Status::Success, cx),
            )
        })
        .unwrap();
    assert_eq!(changed, warning);
    assert_ne!(changed, added);
}

#[gpui_kit::test]
fn compare_with_nothing_to_compare_shows_a_toast(cx: &mut TestAppContext) {
    let col = collection_for("http://127.0.0.1:1", "");
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let notes = |cx: &mut TestAppContext| {
        cx.update_window(s.win, |_, w, cx| w.notifications(cx).len())
            .unwrap()
    };
    let before = notes(cx);
    gpui_kit::VisualTestContext::from_window(s.win, cx)
        .dispatch_action(apinata_app::tab_area::Compare);
    cx.run_until_parked();
    assert_eq!(notes(cx), before + 1);
}
