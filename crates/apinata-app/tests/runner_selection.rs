use crate::support;

use apinata_app::runner_model::{RunState, RunTarget, RunnerModel};
use apinata_app::workspace::Workspace;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext};
use std::path::Path;
use support::*;

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("api/folder.yml", &folder("api")),
        ("api/one.yml", &request("one", "http://127.0.0.1:9/one")),
        ("api/two.yml", &request("two", "http://127.0.0.1:9/two")),
        ("other.yml", &request("other", "http://127.0.0.1:9/other")),
    ])
}

fn runner(
    cx: &mut TestAppContext,
    ws: &Entity<Workspace>,
    root: &Path,
    items: &[&str],
) -> Entity<RunnerModel> {
    let (ws, root) = (ws.clone(), root.to_path_buf());
    let items = items.iter().map(|p| ip(p)).collect();
    cx.new(|_| RunnerModel::for_items(ws, root, items))
}

fn preview(cx: &mut TestAppContext, m: &Entity<RunnerModel>) -> Vec<String> {
    m.read_with(cx, |m, _| {
        m.preview()
            .iter()
            .map(|r| r.path.as_str().to_owned())
            .collect()
    })
}

#[gpui_kit::test]
fn the_preview_lists_the_selection_in_run_order(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let m = runner(cx, &ws, &root, &["other.yml", "api"]);
    assert_eq!(
        m.read_with(cx, |m, _| m.target().clone()),
        RunTarget::Items(vec![ip("other.yml"), ip("api")])
    );
    m.update(cx, |m, cx| m.refresh_preview(cx));
    wait_real(cx, |cx| preview(cx, &m).len() == 3);
    assert_eq!(preview(cx, &m), ["api/one.yml", "api/two.yml", "other.yml"]);
    assert!(matches!(
        m.read_with(cx, |m, _| m.state().clone()),
        RunState::Idle
    ));
}

#[gpui_kit::test]
fn a_member_deleted_after_opening_runs_the_survivors_and_none_left_shows_an_empty_plan(
    cx: &mut TestAppContext,
) {
    http_ready(cx);
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let m = runner(cx, &ws, &root, &["other.yml", "api/one.yml"]);
    std::fs::remove_file(col.path().join("other.yml")).unwrap();
    m.update(cx, |m, cx| m.refresh_preview(cx));
    wait_real(cx, |cx| preview(cx, &m) == ["api/one.yml"]);
    std::fs::remove_file(col.path().join("api/one.yml")).unwrap();
    m.update(cx, |m, cx| m.refresh_preview(cx));
    wait_real(cx, |cx| preview(cx, &m).is_empty());
    assert!(matches!(
        m.read_with(cx, |m, _| m.state().clone()),
        RunState::Idle
    ));
}

#[gpui_kit::test]
fn unsaved_names_cover_every_member_and_only_them(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    ws.update(cx, |w, cx| {
        w.send(&root, edit_url("api/one.yml", "http://127.0.0.1:9/z"), cx)
    });
    cx.run_until_parked();
    let inside = runner(cx, &ws, &root, &["api"]);
    assert!(inside.update(cx, |m, cx| m.start(cx)));
    assert_eq!(inside.read_with(cx, |m, _| m.unsaved().to_vec()), ["one"]);
    inside.update(cx, |m, cx| m.cancel(cx));
    let outside = runner(cx, &ws, &root, &["other.yml"]);
    assert!(outside.update(cx, |m, cx| m.start(cx)));
    assert!(outside.read_with(cx, |m, _| m.unsaved().is_empty()));
    outside.update(cx, |m, cx| m.cancel(cx));
}

#[gpui_kit::test]
fn the_tab_is_titled_selected_and_its_count(cx: &mut TestAppContext) {
    let a = open_area(cx);
    a.ws.update(cx, |w, cx| {
        w.open_runner_items(&a.root, vec![ip("a.yml"), ip("b.yml")], cx)
    })
    .expect("opens");
    cx.run_until_parked();
    let entry =
        a.ws.read_with(cx, |w, _| w.tabs().active().cloned())
            .expect("a tab");
    assert_eq!(
        a.view.read_with(cx, |v, cx| v.tab_label(&entry, cx)),
        "Selected (2)"
    );
}

// The plan list of a selection run ends above the open dock in a short window.
#[gpui_kit::test]
fn a_selection_runner_holds_above_the_open_dock_in_a_short_window(cx: &mut TestAppContext) {
    use apinata_app::keymap::ToggleConsole;
    use gpui_kit::{VisualTestContext, px, size};
    http_ready(cx);
    let names: Vec<String> = (0..15).map(|i| format!("r{i:02}.yml")).collect();
    let texts: Vec<String> = (0..15)
        .map(|i| request(&format!("R{i:02}"), &format!("http://127.0.0.1:9/r{i}")))
        .collect();
    let mut files: Vec<(&str, &str)> = vec![("opencollection.yml", ROOT)];
    files.extend(
        names
            .iter()
            .map(String::as_str)
            .zip(texts.iter().map(String::as_str)),
    );
    let col = write_tree(&files);
    let s = started(cx, &col);
    let items = names.iter().map(|n| ip(n)).collect();
    s.ws.update(cx, |w, cx| w.open_runner_items(&s.root, items, cx))
        .expect("a runner tab");
    wait_real(cx, |cx| present(cx, s.win, ("run-plan-row", 14usize)));
    VisualTestContext::from_window(s.win, cx).dispatch_action(ToggleConsole);
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1000.), px(480.)));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let dock = w.find("console-dock").bounds();
        let pane = w.find("runner").bounds();
        let plan = w.find("run-plan").bounds();
        assert!(
            pane.bottom() <= dock.top() && plan.bottom() <= pane.bottom(),
            "runner {pane:?} plan {plan:?} past the dock {dock:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn an_unsaved_request_under_any_one_member_is_reported(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    ws.update(cx, |w, cx| {
        w.send(&root, edit_url("api/one.yml", "http://127.0.0.1:9/z"), cx)
    });
    cx.run_until_parked();
    let both = runner(cx, &ws, &root, &["other.yml", "api"]);
    assert!(both.update(cx, |m, cx| m.start(cx)));
    assert_eq!(both.read_with(cx, |m, _| m.unsaved().to_vec()), ["one"]);
    both.update(cx, |m, cx| m.cancel(cx));
}

// A draft has no file, so no run could include it; the note names only saved requests.
#[gpui_kit::test]
fn unsaved_names_leave_out_drafts(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    ws.update(cx, |w, cx| w.new_draft(&root, ip("api"), cx));
    cx.run_until_parked();
    let drafts: Vec<_> = ws.read_with(cx, |w, cx| {
        let snap = w.snapshot(&root, cx).unwrap();
        let keys = snap.content.requests.keys();
        keys.filter(|p| p.is_draft()).cloned().collect()
    });
    assert_eq!(drafts.len(), 1, "the draft is in the collection");
    for (item, url) in [
        (drafts[0].as_str(), "http://127.0.0.1:9/d"),
        ("api/one.yml", "http://127.0.0.1:9/z"),
    ] {
        ws.update(cx, |w, cx| w.send(&root, edit_url(item, url), cx));
    }
    cx.run_until_parked();
    let dirty = ws.read_with(cx, |w, cx| {
        w.snapshot(&root, cx).unwrap().content.requests[&drafts[0]].dirty
    });
    assert!(dirty, "the edited draft is dirty");
    let m = runner(cx, &ws, &root, &["api"]);
    assert!(m.update(cx, |m, cx| m.start(cx)));
    assert_eq!(m.read_with(cx, |m, _| m.unsaved().to_vec()), ["one"]);
    m.update(cx, |m, cx| m.cancel(cx));
}
