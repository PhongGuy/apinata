//! The History command lists recent sends of the open collections and opens the chosen one.

use crate::support;

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use apinata_app::history_palette::{HISTORY_MAX, history_entries};
use apinata_app::last_response::ResponseStore;
use apinata_app::send_model::SendState;
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

fn at(ms: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(ms)
}

fn kept(item: &str, status: u16) -> RequestResult {
    let response = Response::new(status).with_text_body("k");
    RequestResult::from_parts(ip(item), "x", Outcome::Passed, Some(response), Vec::new())
}

fn seeded(cx: &mut TestAppContext, col: &tempfile::TempDir) -> Started {
    let root = col.path().to_path_buf();
    started_with(cx, col, move |paths| {
        let store = ResponseStore::new(paths.responses.clone());
        store
            .save(&root, &ip("a.yml"), &kept("a.yml", 201), at(1_000))
            .unwrap();
        store
            .save(&root, &ip("b.yml"), &kept("b.yml", 202), at(2_000))
            .unwrap();
        store
            .save(&root, &ip("a.yml"), &kept("a.yml", 203), at(3_000))
            .unwrap();
        let elsewhere = Path::new("/elsewhere");
        store
            .save(elsewhere, &ip("z.yml"), &kept("z.yml", 299), at(4_000))
            .unwrap();
    })
}

#[gpui_kit::test]
fn entries_list_open_collections_newest_first(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = seeded(cx, &col);
    let store = cx.update(|cx| cx.global::<ResponseStore>().clone());
    let recent = store.recent(std::slice::from_ref(&s.root), 100);
    let got: Vec<(String, u64)> =
        s.ws.read_with(cx, |w, cx| history_entries(&recent, w, cx))
            .into_iter()
            .map(|e| {
                assert_eq!(e.root, s.root);
                (e.label, e.seq)
            })
            .collect();
    let want = [("a", 1), ("b", 0), ("a", 0)].map(|(l, q)| (l.to_owned(), q));
    assert_eq!(got, want);
}

#[gpui_kit::test]
fn entries_skip_requests_that_are_gone(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = seeded(cx, &col);
    let store = cx.update(|cx| cx.global::<ResponseStore>().clone());
    let mut recent = store.recent(std::slice::from_ref(&s.root), 100);
    recent[0].item = ip("gone.yml");
    let got = s.ws.read_with(cx, |w, cx| history_entries(&recent, w, cx));
    assert_eq!(got.len(), 2);
}

// The cap applies to listable sends, so sends of requests that are gone never crowd out
// the ones that remain.
#[gpui_kit::test]
fn entries_cap_after_dropping_requests_that_are_gone(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = seeded(cx, &col);
    let store = cx.update(|cx| cx.global::<ResponseStore>().clone());
    let live = store.recent(std::slice::from_ref(&s.root), 1).remove(0);
    let mut gone = live.clone();
    gone.item = ip("gone.yml");
    let mut recent = vec![gone; HISTORY_MAX];
    recent.extend(std::iter::repeat_n(live, HISTORY_MAX + 5));
    let got = s.ws.read_with(cx, |w, cx| history_entries(&recent, w, cx));
    assert_eq!(got.len(), HISTORY_MAX);
    assert!(got.iter().all(|e| e.label == "a"));
}

#[gpui_kit::test]
fn choosing_a_send_opens_its_tab_showing_it(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = seeded(cx, &col);
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.input("History", cx))
        .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "enter");
    wait_for_history(cx, &s);
    // Rows: a #1, b #0, a #0. Up from no highlight, or from the first row, reaches the last
    // row (`CommandState::select_by`), which is a's oldest send.
    cx.simulate_keystrokes(s.win, "up enter");
    frame(cx, s.win);
    let open = cx
        .update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap();
    assert!(!open, "the History dialog closed");
    assert_eq!(tab_names(cx, &s.ws), ["a.yml"]);
    let editor = s.shell.read_with(cx, |sh, cx| {
        let tab = sh.workspace().read(cx).tabs().active().unwrap().id;
        sh.tab_area().read(cx).editor(tab).cloned().unwrap()
    });
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.viewing()) == Some(0));
    let status = model.read_with(cx, |m, _| match m.state() {
        SendState::Done(r) => r.response.as_ref().map(|r| r.status()),
        _ => None,
    });
    assert_eq!(status, Some(201));
    model.update(cx, |m, cx| m.back_to_latest(cx));
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| match m.state() {
            SendState::Done(r) => r.response.as_ref().map(|r| r.status()) == Some(203),
            _ => false,
        })
    });
}

// The list arrives from the store queue, so the dialog opens on a later frame.
fn wait_for_history(cx: &mut TestAppContext, s: &Started) {
    wait_real(cx, |cx| {
        frame(cx, s.win);
        present(cx, s.win, "history-palette")
    });
}

#[test]
fn the_registry_offers_history_in_the_view_group() {
    use apinata_app::commands::registry;
    use apinata_app::palette::PaletteGroup;
    let cmds = registry();
    let history = cmds
        .iter()
        .find(|c| c.id == "view.history")
        .expect("listed");
    assert_eq!(history.label, "History");
    assert_eq!(history.group, PaletteGroup::View);
    assert!(history.in_palette);
}

#[gpui_kit::test]
fn entries_carry_each_sends_target(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = seeded(cx, &col);
    let store = cx.update(|cx| cx.global::<ResponseStore>().clone());
    let mut recent = store.recent(std::slice::from_ref(&s.root), 100);
    recent[0].send.method = "POST".into();
    recent[0].send.url = "https://api.example.com/a?page=2".into();
    let got = s.ws.read_with(cx, |w, cx| history_entries(&recent, w, cx));
    assert_eq!(got[0].target, "POST https://api.example.com/a?page=2");
    assert_eq!(got[1].target, "");
}

#[gpui_kit::test]
fn a_palette_row_names_its_target(cx: &mut TestAppContext) {
    let col = two_requests();
    let root = col.path().to_path_buf();
    let s = started_with(cx, &col, move |paths| {
        let response = Response::new(201)
            .with_text_body("k")
            .with_url("https://api.example.com/a?page=2");
        let r = RequestResult::from_parts(
            ip("a.yml"),
            "x",
            Outcome::Passed,
            Some(response),
            Vec::new(),
        );
        ResponseStore::new(paths.responses.clone())
            .save(&root, &ip("a.yml"), &r, at(1_000))
            .unwrap();
    });
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.input("History", cx))
        .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "enter");
    wait_for_history(cx, &s);
    let name = cx
        .update_window(s.win, |_, w, _| {
            w.within("history-palette")
                .find(("history-target", 0usize))
                .label()
                .map(str::to_owned)
        })
        .unwrap();
    assert_eq!(name.as_deref(), Some("https://api.example.com/a?page=2"));
}

#[gpui_kit::test]
fn a_long_request_name_leaves_the_target_and_detail_visible(cx: &mut TestAppContext) {
    let long = "a very long request name ".repeat(20);
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request(&long, "http://x/0")),
    ]);
    let root = col.path().to_path_buf();
    let s = started_with(cx, &col, move |paths| {
        let response = Response::new(201)
            .with_text_body("k")
            .with_url("https://api.example.com/a");
        let r = RequestResult::from_parts(
            ip("a.yml"),
            "x",
            Outcome::Passed,
            Some(response),
            Vec::new(),
        );
        ResponseStore::new(paths.responses.clone())
            .save(&root, &ip("a.yml"), &r, at(1_000))
            .unwrap();
    });
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.input("History", cx))
        .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "enter");
    wait_for_history(cx, &s);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let palette = w.find("history-palette").bounds();
        let target = w
            .within("history-palette")
            .find(("history-target", 0usize))
            .bounds();
        let detail = w
            .within("history-palette")
            .find(("history-detail", 0usize))
            .bounds();
        assert!(
            target.size.width > gpui_kit::px(0.),
            "the target keeps a share"
        );
        assert!(detail.size.width > gpui_kit::px(0.));
        assert!(
            detail.right() <= palette.right(),
            "the detail stays inside the row"
        );
    })
    .unwrap();
}

// One Escape closes the History dialog even with a query typed.
#[gpui_kit::test]
fn escape_closes_the_history_palette_with_a_query_typed(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = seeded(cx, &col);
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.input("History", cx))
        .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "enter");
    wait_for_history(cx, &s);
    cx.update_window(s.win, |_, w, cx| w.input("a", cx))
        .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "escape");
    frame(cx, s.win);
    let open = cx
        .update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap();
    assert!(!open, "one Escape closed the History dialog");
    assert!(!present(cx, s.win, "history-palette"));
}

fn open_history_palette(cx: &mut TestAppContext, s: &Started) {
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.input("History", cx))
        .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "enter");
    wait_for_history(cx, s);
}

fn empty_label(cx: &mut TestAppContext, s: &Started) -> Option<String> {
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find("history-empty")
            .and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

#[gpui_kit::test]
fn an_empty_history_says_so(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_history_palette(cx, &s);
    assert_eq!(empty_label(cx, &s).as_deref(), Some("No history yet"));
}

#[gpui_kit::test]
fn a_filter_with_no_match_says_no_matching_sends(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = seeded(cx, &col);
    open_history_palette(cx, &s);
    cx.update_window(s.win, |_, w, cx| w.input("zzzz", cx))
        .unwrap();
    frame(cx, s.win);
    assert_eq!(empty_label(cx, &s), None, "not the no-history message");
    let label = cx
        .update_window(s.win, |_, w, _| {
            w.try_find("history-no-match")
                .and_then(|e| e.label().map(str::to_owned))
        })
        .unwrap();
    assert_eq!(label.as_deref(), Some("No matching sends"));
}
