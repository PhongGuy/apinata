use crate::support;
use apinata_app::compare_source::CompareSource;
use apinata_app::compare_view::CompareRow;
use apinata_app::diff_doc::RowKind;
use apinata_app::last_response::{ResponseStore, install};
use apinata_app::send_model::SendState;
use apinata_core::{BodyComparison, Outcome, RequestResult, Response, StatusChange};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use std::time::{Duration, SystemTime};
use support::*;

fn kept_as(status: u16, content_type: &str, body: &str) -> RequestResult {
    RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(
            Response::new(status)
                .with_header("content-type", content_type)
                .with_text_body(body),
        ),
        Vec::new(),
    )
}

// Two kept sends (201 then 202) with `a.yml` open on the newest, plus optional extra file text.
fn two_sends(
    cx: &mut TestAppContext,
    files: Vec<(&'static str, String)>,
) -> (
    tempfile::TempDir,
    AnyWindowHandle,
    Entity<apinata_app::request_editor::RequestEditor>,
) {
    two_bodies(cx, files, "application/json", [r#"{"n":1}"#, r#"{"n":2}"#])
}

// `two_sends` with the given content type and bodies for the older and newer send.
fn two_bodies(
    cx: &mut TestAppContext,
    files: Vec<(&'static str, String)>,
    content_type: &str,
    bodies: [&str; 2],
) -> (
    tempfile::TempDir,
    AnyWindowHandle,
    Entity<apinata_app::request_editor::RequestEditor>,
) {
    let col = write_tree(
        &files
            .iter()
            .map(|(p, t)| (*p, t.as_str()))
            .collect::<Vec<_>>(),
    );
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    let t0 = SystemTime::now() - Duration::from_secs(400);
    for (i, (status, body)) in [(201u16, bodies[0]), (202, bodies[1])]
        .into_iter()
        .enumerate()
    {
        store
            .save(
                &root,
                &ip("a.yml"),
                &kept_as(status, content_type, body),
                t0 + Duration::from_millis(i as u64),
            )
            .unwrap();
    }
    cx.update(|cx| install(store, cx));
    let (win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    wait_real(cx, |cx| {
        pane.read_with(cx, |p, _| p.history().len() == 2)
            && model.read_with(cx, |m, _| matches!(m.state(), SendState::Done(_)))
    });
    frame(cx, win);
    std::mem::forget(col);
    (state, win, editor)
}
fn plain_files() -> Vec<(&'static str, String)> {
    vec![
        ("opencollection.yml", ROOT.to_owned()),
        ("a.yml", request("a", "http://x/0")),
    ]
}

#[gpui_kit::test]
fn previous_send_compares_the_newest_with_the_one_before(cx: &mut TestAppContext) {
    let (_s, win, editor) = two_sends(cx, plain_files());
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    cx.update_window(win, |_, w, cx| w.click("response-compare", cx))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.within("popup-menu").click(0usize, cx))
        .unwrap();
    wait_real(cx, |cx| pane.read_with(cx, |p, _| p.comparison().is_some()));
    frame(cx, win);
    let c = pane.read_with(cx, |p, _| p.comparison().unwrap());
    let StatusChange::Changed { old, new } = &c.status else {
        panic!("{:?}", c.status)
    };
    assert_eq!((old.code, new.code), (201, 202));
    assert!(matches!(c.body, BodyComparison::Json { .. }));
    cx.update_window(win, |_, w, _| {
        assert!(
            w.try_find(("response-tab", 6usize)).is_some(),
            "the Compare tab is in the strip"
        );
        assert!(w.try_find("compare-summary").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn with_one_kept_send_previous_is_disabled_with_a_reason(cx: &mut TestAppContext) {
    // Only the oldest send exists once the viewed one is the oldest.
    let (_s, _win, editor) = two_sends(cx, plain_files());
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    pane.update(cx, |p, cx| p.show_history(0, cx));
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.viewing()) == Some(0));
    let choices = pane.read_with(cx, |p, cx| p.compare_choices(cx));
    assert_eq!(
        (choices[0].label.as_str(), choices[0].disabled),
        ("Previous send", Some("No earlier send"))
    );
}

const WITH_EXAMPLES: &str = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: http://x/0\nexamples:\n  - name: golden\n    response:\n      status: 202\n      headers:\n        - name: content-type\n          value: application/json\n      body:\n        type: json\n        data: '{\"n\":2}'\n  - name: bare\n    request:\n      url: http://x/0\n";

#[gpui_kit::test]
fn an_example_is_a_source_and_one_without_a_response_is_disabled(cx: &mut TestAppContext) {
    let (_s, _win, editor) = two_sends(
        cx,
        vec![
            ("opencollection.yml", ROOT.to_owned()),
            ("a.yml", WITH_EXAMPLES.to_owned()),
        ],
    );
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    let choices = pane.read_with(cx, |p, cx| p.compare_choices(cx));
    let got: Vec<_> = choices
        .iter()
        .map(|c| (c.label.as_str(), c.disabled.is_some()))
        .collect();
    assert_eq!(
        got,
        [("Previous send", false), ("golden", false), ("bare", true)]
    );
    pane.update(cx, |p, cx| p.compare_with(CompareSource::Example(0), cx));
    wait_real(cx, |cx| pane.read_with(cx, |p, _| p.comparison().is_some()));
    let c = pane.read_with(cx, |p, _| p.comparison().unwrap());
    assert_eq!(c.status, StatusChange::Same);
    assert_eq!(c.body, BodyComparison::Identical);
}

#[gpui_kit::test]
fn closing_removes_the_tab_and_returns_to_the_body(cx: &mut TestAppContext) {
    let (_s, win, editor) = two_sends(cx, plain_files());
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    pane.update(cx, |p, cx| p.compare_with(CompareSource::Previous, cx));
    wait_real(cx, |cx| pane.read_with(cx, |p, _| p.comparison().is_some()));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("compare-close", cx))
        .unwrap();
    frame(cx, win);
    assert!(pane.read_with(cx, |p, _| p.compare_source().is_none()));
    cx.update_window(win, |_, w, _| {
        assert!(w.try_find(("response-tab", 6usize)).is_none())
    })
    .unwrap();
}

#[gpui_kit::test]
fn at_compact_width_the_button_folds_into_the_more_menu(cx: &mut TestAppContext) {
    let (_s, win, _editor) = two_sends(cx, plain_files());
    cx.simulate_window_resize(win, gpui_kit::size(gpui_kit::px(520.), gpui_kit::px(700.)));
    frame(cx, win);
    cx.update_window(win, |_, w, _| {
        assert!(w.try_find("response-compare").is_none())
    })
    .unwrap();
    cx.update_window(win, |_, w, cx| w.click("response-more", cx))
        .unwrap();
    frame(cx, win);
    let labels: Vec<String> = cx
        .update_window(win, |_, w, _| {
            let menu = w.within("popup-menu");
            (0..8usize)
                .filter_map(|i| {
                    menu.try_find(i)
                        .and_then(|row| row.label().map(str::to_owned))
                })
                .collect() // probe indexes 0..8
        })
        .unwrap();
    assert!(
        labels.iter().any(|l| l == "Compare with Previous send"),
        "{labels:?}"
    );
}

#[gpui_kit::test]
fn the_compact_menu_gives_the_reason_a_source_is_disabled(cx: &mut TestAppContext) {
    let (_s, win, editor) = two_sends(cx, plain_files());
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    pane.update(cx, |p, cx| p.show_history(0, cx));
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.viewing()) == Some(0));
    cx.simulate_window_resize(win, gpui_kit::size(gpui_kit::px(520.), gpui_kit::px(700.)));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("response-more", cx))
        .unwrap();
    frame(cx, win);
    let labels: Vec<String> = cx
        .update_window(win, |_, w, _| {
            let menu = w.within("popup-menu");
            (0..8usize)
                .filter_map(|i| {
                    menu.try_find(i)
                        .and_then(|row| row.label().map(str::to_owned))
                })
                .collect()
        })
        .unwrap();
    assert!(
        labels
            .iter()
            .any(|l| l == "Compare with Previous send (No earlier send)"),
        "{labels:?}"
    );
}

#[gpui_kit::test]
fn a_draft_offers_only_the_previous_send(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let (_state, ws, root) = open_dir(cx, &col);
    ws.update(cx, |w, cx| {
        w.new_draft(&root, apinata_core::ItemPath::root(), cx)
    })
    .expect("open");
    cx.run_until_parked();
    let draft = ws.read_with(cx, |w, _| {
        w.tabs()
            .active()
            .expect("the draft tab")
            .item()
            .expect("a request")
            .as_str()
            .to_owned()
    });
    let (_win, editor) = editor_for(cx, &ws, &root, &draft, false);
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    let choices = pane.read_with(cx, |p, cx| p.compare_choices(cx));
    assert_eq!(choices.len(), 1);
    assert_eq!(choices[0].label, "Previous send");
}

#[gpui_kit::test]
fn a_text_body_without_a_final_newline_shows_git_s_marker_row(cx: &mut TestAppContext) {
    let (_s, win, editor) = two_bodies(cx, plain_files(), "text/plain", ["one\ntwo\n", "one\n2"]);
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    cx.update_window(win, |_, w, cx| w.click("response-compare", cx))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.within("popup-menu").click(0usize, cx))
        .unwrap();
    wait_real(cx, |cx| pane.read_with(cx, |p, _| p.comparison().is_some()));
    frame(cx, win);
    let c = pane.read_with(cx, |p, _| p.comparison().unwrap());
    let rows = apinata_app::compare_view::rows(&c, false);
    let at = rows
        .iter()
        .position(|r| matches!(r, CompareRow::Diff(d) if d.kind == RowKind::NoNewline))
        .expect("a marker row");
    let CompareRow::Diff(marker) = &rows[at] else {
        unreachable!()
    };
    assert_eq!(marker.text, "\\ No newline at end of file");
    assert!(
        matches!(&rows[at - 1], CompareRow::Diff(d) if d.kind == RowKind::Added && d.text == "2"),
        "after the line it marks"
    );
    // The row is on screen in the Compare list.
    assert!(present(cx, win, ("diff-line", at)));
}

#[gpui_kit::test]
fn a_json_change_mark_is_its_own_coloured_element(cx: &mut TestAppContext) {
    let (_s, win, editor) = two_bodies(
        cx,
        plain_files(),
        "application/json",
        [r#"{"n":1}"#, r#"{"n":1,"m":2}"#],
    );
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    cx.update_window(win, |_, w, cx| w.click("response-compare", cx))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.within("popup-menu").click(0usize, cx))
        .unwrap();
    wait_real(cx, |cx| pane.read_with(cx, |p, _| p.comparison().is_some()));
    frame(cx, win);
    let marks = (0..30usize)
        .filter(|ix| present(cx, win, ("compare-mark", *ix)))
        .count();
    assert!(
        marks >= 1,
        "the +/-/~ mark of a change row is a span of its own"
    );
}

#[gpui_kit::test]
fn the_change_marks_share_one_column_width_in_every_theme(cx: &mut TestAppContext) {
    let (_s, win, editor) = two_bodies(
        cx,
        plain_files(),
        "application/json",
        [r#"{"n":1,"a":1}"#, r#"{"n":2,"m":2}"#],
    );
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    cx.update_window(win, |_, w, cx| w.click("response-compare", cx))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.within("popup-menu").click(0usize, cx))
        .unwrap();
    wait_real(cx, |cx| pane.read_with(cx, |p, _| p.comparison().is_some()));
    for name in [
        apinata_app::theme::DARK,
        apinata_app::theme::LIGHT,
        apinata_app::theme::HC_DARK,
        apinata_app::theme::HC_LIGHT,
    ] {
        use_theme(cx, name);
        frame(cx, win);
        let widths: Vec<f32> = cx
            .update_window(win, |_, w, cx| {
                w.render_frame(cx);
                let want = f32::from(apinata_app::style::SPACE_3.to_pixels(w.rem_size()));
                assert!(want > 0.);
                (0..40usize)
                    .filter_map(|ix| w.try_find(("compare-mark", ix)))
                    .map(|m| f32::from(m.bounds().size.width) - want)
                    .collect()
            })
            .unwrap();
        // One removed, one added and one changed row.
        assert!(widths.len() >= 3, "{name}: marks {widths:?}");
        // Each mark's width minus the pinned column width is zero.
        assert!(
            widths.iter().all(|d| d.abs() < 0.01),
            "{name}: width minus column {widths:?}"
        );
    }
}
