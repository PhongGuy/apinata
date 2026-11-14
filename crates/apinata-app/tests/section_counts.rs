//! Section tabs count enabled rows and mark a body or auth mode, following the draft.

use crate::support;

use apinata_app::request_editor::Section;
use gpui_kit::AppContext as _;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt;
use support::*;

const A: &str = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: \"http://x/0?a=1\"\n  params:\n    - name: a\n      value: \"1\"\n      type: query\n    - name: b\n      value: \"2\"\n      type: query\n      disabled: true\n  headers:\n    - name: X-One\n      value: \"1\"\n  body:\n    type: json\n    data: \"{}\"\n";

#[gpui_kit::test]
fn counts_and_dots_follow_the_draft(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", A)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let count = |cx: &mut TestAppContext, s| ed.read_with(cx, |e, cx| e.section_count(s, cx));
    let marked = |cx: &mut TestAppContext, s| ed.read_with(cx, |e, cx| e.section_marked(s, cx));
    assert_eq!(count(cx, Section::Params), Some(1));
    assert_eq!(count(cx, Section::Headers), Some(1));
    assert_eq!(count(cx, Section::Vars), Some(0));
    assert_eq!(count(cx, Section::Assertions), Some(0));
    assert_eq!(count(cx, Section::Body), None);
    assert!(marked(cx, Section::Body));
    assert!(!marked(cx, Section::Auth));
    assert!(present(
        cx,
        win,
        ("section-count", Section::Params as usize)
    ));
    assert!(present(
        cx,
        win,
        ("section-count", Section::Headers as usize)
    ));
    assert!(
        !present(cx, win, ("section-count", Section::Vars as usize)),
        "zero shows nothing"
    );
    assert!(present(cx, win, ("section-mode", Section::Body as usize)));
    assert!(!present(cx, win, ("section-mode", Section::Auth as usize)));
    assert!(
        !present(cx, win, ("section-dot", Section::Body as usize)),
        "a mode is not drawn as the unsaved dot"
    );
    // Enabling the second param raises the count once the actor has the edit.
    cx.update_window(win, |_, w, cx| {
        w.within("params").click(("kv-enabled", 1usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(count(cx, Section::Params), Some(2));
}

#[gpui_kit::test]
fn a_new_var_row_shows_a_count(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.update_window(win, |_, w, cx| {
        w.click(("section", Section::Vars as usize), cx)
    })
    .unwrap();
    assert_eq!(
        ed.read_with(cx, |e, cx| e.section_count(Section::Vars, cx)),
        Some(0)
    );
    type_at_end(cx, win, Some("vars"), ("kv-name", 0usize), "v");
    cx.run_until_parked();
    assert_eq!(
        ed.read_with(cx, |e, cx| e.section_count(Section::Vars, cx)),
        Some(1)
    );
    assert!(present(cx, win, ("section-count", Section::Vars as usize)));
}

fn script_marked(cx: &mut TestAppContext, runtime: &str) -> (bool, bool) {
    let text = format!("{}{runtime}", request("a", "http://x/"));
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &text)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let marked = ed.read_with(cx, |e, cx| e.section_marked(Section::Script, cx));
    (
        marked,
        present(cx, win, ("section-mode", Section::Script as usize)),
    )
}

#[gpui_kit::test]
fn a_pre_request_script_marks_the_script_tab(cx: &mut TestAppContext) {
    let pre =
        "runtime:\n  scripts:\n    - type: before-request\n      code: \"bru.setVar('a', 1)\"\n";
    assert_eq!(script_marked(cx, pre), (true, true));
}

#[gpui_kit::test]
fn a_post_response_script_marks_the_script_tab(cx: &mut TestAppContext) {
    let post =
        "runtime:\n  scripts:\n    - type: after-response\n      code: \"bru.setVar('b', 2)\"\n";
    assert_eq!(script_marked(cx, post), (true, true));
}

#[gpui_kit::test]
fn no_script_leaves_the_script_tab_plain(cx: &mut TestAppContext) {
    assert_eq!(script_marked(cx, ""), (false, false));
}

#[gpui_kit::test]
fn a_tests_script_marks_the_tests_tab(cx: &mut TestAppContext) {
    let tests = "runtime:\n  scripts:\n    - type: tests\n      code: \"test('ok', () => {})\"\n";
    let text = format!("{}{tests}", request("a", "http://x/"));
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &text)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    assert!(ed.read_with(cx, |e, cx| e.section_marked(Section::Tests, cx)));
    assert!(present(cx, win, ("section-mode", Section::Tests as usize)));
}

#[gpui_kit::test]
fn each_mode_ring_says_what_it_marks(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", A)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let label = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find(("section-mode", Section::Body as usize))
                .label()
                .map(str::to_owned)
        })
        .unwrap();
    assert_eq!(label.as_deref(), Some("Has a body"));
}

/// The Auth tab's ring words for `rel` in `col`, or `None` when the tab shows no ring.
fn auth_ring(cx: &mut TestAppContext, col: &tempfile::TempDir, rel: &str) -> Option<String> {
    let (_state, ws, root) = open_dir(cx, col);
    let (win, ed) = editor_for(cx, &ws, &root, rel, false);
    let marked = ed.read_with(cx, |e, cx| e.section_marked(Section::Auth, cx));
    let words = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find(("section-mode", Section::Auth as usize))
                .and_then(|e| e.label().map(str::to_owned))
        })
        .unwrap();
    assert_eq!(marked, words.is_some(), "the ring follows section_marked");
    words
}

#[gpui_kit::test]
fn inheriting_no_auth_leaves_the_auth_tab_plain(cx: &mut TestAppContext) {
    let text = format!("{}  auth: inherit\n", request("a", "http://x/a"));
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &text)]);
    assert_eq!(auth_ring(cx, &col, "a.yml"), None);
}

#[gpui_kit::test]
fn the_auth_ring_names_the_auth_that_applies(cx: &mut TestAppContext) {
    let root = format!("{ROOT}request:\n  auth:\n    type: bearer\n    token: t\n");
    let users = format!(
        "{}request:\n  auth:\n    type: basic\n    username: u\n    password: p\n",
        folder("users")
    );
    let inherits = |name: &str| format!("{}  auth: inherit\n", request(name, "http://x/"));
    let own = format!(
        "{}  auth:\n    type: apikey\n    key: k\n    value: v\n",
        request("own", "http://x/")
    );
    let col = write_tree(&[
        ("opencollection.yml", &root),
        ("u/folder.yml", &users),
        ("u/list.yml", &inherits("list")),
        ("top.yml", &inherits("top")),
        ("own.yml", &own),
    ]);
    assert_eq!(
        auth_ring(cx, &col, "top.yml").as_deref(),
        Some("Uses Bearer Token from the collection")
    );
    assert_eq!(
        auth_ring(cx, &col, "u/list.yml").as_deref(),
        Some("Uses Basic Auth from folder \"users\"")
    );
    assert_eq!(
        auth_ring(cx, &col, "own.yml").as_deref(),
        Some("Uses API Key")
    );
}

// The ring is too small to aim at, so hovering anywhere on a marked tab says what it marks.
#[gpui_kit::test]
fn hovering_a_marked_tab_says_what_its_ring_means(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", A)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let tip = |cx: &mut TestAppContext, ix: usize| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.hover(("section", ix), cx);
        })
        .unwrap();
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(2));
        cx.run_until_parked();
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find("section-mode-tooltip")
                .and_then(|e| e.label().map(str::to_owned))
        })
        .unwrap()
    };
    assert_eq!(
        tip(cx, Section::Body as usize).as_deref(),
        Some("Has a body")
    );
    assert_eq!(
        tip(cx, Section::Params as usize),
        None,
        "a counted tab has no ring to explain"
    );
}
