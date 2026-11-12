//! A narrow response pane folds its secondary actions into one menu, so the header is the
//! tabs over one status row and the body bar stays one row.

use crate::support;

use std::time::{Duration, SystemTime};

use apinata_app::last_response::{ResponseStore, install};
use apinata_app::request_editor::RequestEditor;
use apinata_app::send_model::{SendModel, SendState};
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, Pixels, TestAppContext, px, size,
};
use support::*;

fn kept_as(status: u16, cut: bool) -> RequestResult {
    let mut response = Response::new(status)
        .with_header("content-type", "application/json")
        .with_text_body("{\"a\":1}")
        .with_response_time(Duration::from_millis(7));
    if cut {
        response = response.with_truncated(7);
    }
    RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(response),
        Vec::new(),
    )
}

fn shown(cx: &mut TestAppContext, model: &Entity<SendModel>) -> Option<u16> {
    model.read_with(cx, |m, _| match m.state() {
        SendState::Done(r) => r.response.as_ref().map(|r| r.status()),
        _ => None,
    })
}

// An editor whose pane shows the older of two kept sends, saved minutes ago, so every
// secondary action is on offer: the saved label, Save response as example, Export as HAR, History, Back to latest.
fn viewing_an_older_send(
    cx: &mut TestAppContext,
) -> (
    tempfile::TempDir,
    tempfile::TempDir,
    AnyWindowHandle,
    Entity<RequestEditor>,
) {
    viewing_older(cx, false)
}

fn viewing_older(
    cx: &mut TestAppContext,
    cut: bool,
) -> (
    tempfile::TempDir,
    tempfile::TempDir,
    AnyWindowHandle,
    Entity<RequestEditor>,
) {
    let col = two_requests();
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    let minutes_ago = SystemTime::now() - Duration::from_secs(400);
    for (i, status) in [201u16, 202].into_iter().enumerate() {
        let at = minutes_ago + Duration::from_millis(i as u64);
        store
            .save(&root, &ip("a.yml"), &kept_as(status, cut), at)
            .unwrap();
    }
    cx.update(|cx| install(store, cx));
    let (win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    wait_real(cx, |cx| {
        pane.read_with(cx, |p, _| p.history().len() == 2) && shown(cx, &model) == Some(202)
    });
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("response-history", cx))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.within("popup-menu").click(1usize, cx))
        .unwrap();
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.viewing()) == Some(0));
    (col, state, win, editor)
}

fn sized(cx: &mut TestAppContext, win: AnyWindowHandle, width: f32) {
    cx.simulate_window_resize(win, size(px(width), px(700.)));
    frame(cx, win);
}

fn bounds(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> Bounds<Pixels> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).bounds()
    })
    .unwrap()
}

fn one_row(cx: &mut TestAppContext, win: AnyWindowHandle, ids: &[&'static str]) {
    let first = bounds(cx, win, ids[0]);
    for id in &ids[1..] {
        let b = bounds(cx, win, id);
        assert!(
            (b.center().y - first.center().y).abs() < px(4.),
            "{id} {b:?} is off the row of {} {first:?}",
            ids[0]
        );
    }
}

#[gpui_kit::test]
fn a_narrow_pane_shows_tabs_over_one_status_row_and_a_one_row_body_bar(cx: &mut TestAppContext) {
    let (_col, _state, win, _editor) = viewing_an_older_send(cx);
    sized(cx, win, 520.);
    for id in [
        "response-save-example",
        "response-latest",
        "body-copy",
        "body-save",
    ] {
        assert!(!present(cx, win, id), "{id} belongs in the overflow menu");
    }
    assert!(present(cx, win, "response-more"), "an overflow menu button");
    let status = [
        "response-status",
        "response-time",
        "response-size",
        "response-saved",
        "response-history",
        "response-more",
    ];
    one_row(cx, win, &status);
    // The tabs keep a whole row of their own above the status row, so none is cut.
    let strip = bounds(cx, win, "response-tab-strip");
    let pane = bounds(cx, win, "response");
    let console = cx
        .update_window(win, |_, w, _| w.find(("response-tab", 5usize)).bounds())
        .unwrap();
    assert!(
        strip.size.width >= pane.size.width - px(1.),
        "{strip:?} in {pane:?}"
    );
    assert!(
        console.right() <= strip.right(),
        "{console:?} cut by {strip:?}"
    );
    assert!(bounds(cx, win, "response-status").top() >= strip.bottom() - px(1.));
    // The saved label shortens to its age; the full text moves to the tooltip.
    let saved = bounds(cx, win, "response-saved");
    assert!(saved.size.width < px(120.), "{saved:?} not shortened");
    one_row(
        cx,
        win,
        &[
            "body-filter",
            "body-view",
            "body-edit",
            "body-pretty",
            "body-raw",
            "body-format",
        ],
    );
}

#[gpui_kit::test]
fn the_overflow_menu_goes_back_to_the_latest_send(cx: &mut TestAppContext) {
    let (_col, _state, win, editor) = viewing_an_older_send(cx);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    sized(cx, win, 520.);
    cx.update_window(win, |_, w, cx| w.click("response-more", cx))
        .unwrap();
    frame(cx, win);
    let labels: Vec<String> = cx
        .update_window(win, |_, w, _| {
            (0..6)
                .filter_map(|ix| w.within("popup-menu").find(ix).label().map(str::to_owned))
                .collect()
        })
        .unwrap();
    assert_eq!(
        labels,
        [
            "Save response as example",
            "Copy body",
            "Save body",
            "Add schema test",
            "Export as HAR",
            "Back to latest"
        ]
    );
    cx.update_window(win, |_, w, cx| w.within("popup-menu").click(5usize, cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(model.read_with(cx, |m, _| m.viewing()), None);
    assert_eq!(shown(cx, &model), Some(202));
}

// A wide pane keeps today's layout: one header row, every action in place, no menu.
#[gpui_kit::test]
fn a_wide_pane_keeps_every_action_on_the_header_row(cx: &mut TestAppContext) {
    let (_col, _state, win, _editor) = viewing_an_older_send(cx);
    sized(cx, win, 1280.);
    assert!(!present(cx, win, "response-more"));
    one_row(
        cx,
        win,
        &[
            "response-tab-strip",
            "response-status",
            "response-saved",
            "response-save-example",
            "response-history",
            "response-latest",
        ],
    );
    let saved = bounds(cx, win, "response-saved");
    assert!(
        saved.size.width > px(160.),
        "{saved:?} shortened while wide"
    );
    one_row(
        cx,
        win,
        &["body-filter", "body-pretty", "body-copy", "body-save"],
    );
}

// Widening the pane while the More actions menu is open unrenders its button, so the menu
// must close with it rather than linger or reopen when the pane narrows again.
#[gpui_kit::test]
fn widening_the_pane_closes_an_open_overflow_menu(cx: &mut TestAppContext) {
    let (_col, _state, win, _editor) = viewing_an_older_send(cx);
    sized(cx, win, 520.);
    cx.update_window(win, |_, w, cx| w.click("response-more", cx))
        .unwrap();
    frame(cx, win);
    assert!(present(cx, win, "popup-menu"), "the menu opened");
    sized(cx, win, 1280.);
    frame(cx, win);
    assert!(
        !present(cx, win, "popup-menu"),
        "the menu outlived its button"
    );
    sized(cx, win, 520.);
    frame(cx, win);
    assert!(present(cx, win, "response-more"));
    assert!(
        !present(cx, win, "popup-menu"),
        "the menu came back on its own"
    );
}

// A body cut at the size limit must not be saved as if it were whole, so the narrow menu
// keeps the entry but disables it and says why; a whole body leaves it plain and enabled.
#[gpui_kit::test]
fn the_overflow_menu_explains_a_locked_save_body(cx: &mut TestAppContext) {
    for cut in [true, false] {
        let (_col, _state, win, editor) = viewing_older(cx, cut);
        let body = editor.read_with(cx, |e, cx| e.response().read(cx).body().clone());
        let (can_save, lock) = body.read_with(cx, |b, _| (b.can_save(), b.save_lock()));
        assert_eq!((can_save, lock.is_some()), (!cut, cut), "cut={cut}");
        sized(cx, win, 520.);
        cx.update_window(win, |_, w, cx| w.click("response-more", cx))
            .unwrap();
        frame(cx, win);
        let labels: Vec<String> = cx
            .update_window(win, |_, w, _| {
                (0..6)
                    .filter_map(|ix| w.within("popup-menu").find(ix).label().map(str::to_owned))
                    .filter(|l| l.starts_with("Save body"))
                    .collect()
            })
            .unwrap();
        let want = if cut {
            "Save body (cut at the size limit)"
        } else {
            "Save body"
        };
        assert_eq!(labels, [want], "cut={cut}");
        let ix = cx
            .update_window(win, |_, w, _| {
                (0..6).find(|&ix| {
                    w.within("popup-menu")
                        .find(ix)
                        .label()
                        .is_some_and(|l| l.starts_with("Save body"))
                })
            })
            .unwrap()
            .expect("Save body is listed");
        cx.update_window(win, |_, w, cx| w.within("popup-menu").click(ix, cx))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(cx.did_prompt_for_new_path(), !cut, "cut={cut}");
    }
}
