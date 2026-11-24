//! Section tabs count enabled rows and mark what is set on the request, following the draft.

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
fn a_tests_script_marks_the_scripts_tab(cx: &mut TestAppContext) {
    let tests = "runtime:\n  scripts:\n    - type: tests\n      code: \"test('ok', () => {})\"\n";
    let text = format!("{}{tests}", request("a", "http://x/"));
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &text)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    assert!(ed.read_with(cx, |e, cx| e.section_marked(Section::Script, cx)));
    assert!(present(cx, win, ("section-mode", Section::Script as usize)));
}

#[gpui_kit::test]
fn each_content_dot_says_what_it_marks(cx: &mut TestAppContext) {
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

/// The Auth tab's dot words for `rel` in `col`, or `None` when the tab shows no dot.
fn auth_dot(cx: &mut TestAppContext, col: &tempfile::TempDir, rel: &str) -> Option<String> {
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
    assert_eq!(marked, words.is_some(), "the dot follows section_marked");
    words
}

#[gpui_kit::test]
fn inheriting_no_auth_leaves_the_auth_tab_plain(cx: &mut TestAppContext) {
    let text = format!("{}  auth: inherit\n", request("a", "http://x/a"));
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &text)]);
    assert_eq!(auth_dot(cx, &col, "a.yml"), None);
}

#[gpui_kit::test]
fn only_the_requests_own_auth_shows_the_dot(cx: &mut TestAppContext) {
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
        auth_dot(cx, &col, "top.yml"),
        None,
        "inheriting is not set here"
    );
    assert_eq!(
        auth_dot(cx, &col, "u/list.yml"),
        None,
        "inheriting is not set here"
    );
    assert_eq!(
        auth_dot(cx, &col, "own.yml").as_deref(),
        Some("Uses API Key")
    );
}

// The dot is too small to aim at, so hovering anywhere on a marked tab says what it marks.
#[gpui_kit::test]
fn hovering_a_marked_tab_says_what_its_dot_means(cx: &mut TestAppContext) {
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
        "a counted tab has no dot to explain"
    );
}

// Every "has content" mark is the one muted dot the script switch also draws: filled, no ring.
#[gpui_kit::test]
fn every_content_mark_is_a_filled_dot(cx: &mut TestAppContext) {
    let runtime = "runtime:\n  scripts:\n    - type: before-request\n      code: \"a()\"\n    - type: tests\n      code: \"t()\"\n";
    let text = format!(
        "{}  auth:\n    type: bearer\n    token: t\n  body:\n    type: json\n    data: \"{{}}\"\n{runtime}",
        request("a", "http://x/")
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &text)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let muted = cx.update(|cx| gpui_kit::component::ActiveTheme::theme(cx).muted_foreground);
    for s in [Section::Body, Section::Auth, Section::Script] {
        let (fills, edges) = mark_paint(cx, win, ("section-mode", s as usize));
        assert_eq!(fills, vec![muted], "{s:?} is a muted dot");
        assert!(edges.is_empty(), "{s:?} draws no ring: {edges:?}");
    }
}

// The file a new request starts as, with the settings block both writers fill with defaults.
fn new_file(kind: &str, extra: &str, settings: &str) -> String {
    format!(
        "info:\n  name: n\n  type: {kind}\n  seq: 1\n{kind}:\n  method: POST\n  url: \"\"\n  auth: inherit\n{extra}settings:\n{settings}"
    )
}

const NEW_SETTINGS: &str = "  encodeUrl: true\n  timeout: inherit\n  followRedirects: true\n  maxRedirects: 5\n  forwardAuthorizationHeader: false\n";
const OLD_SETTINGS: &str = "  encodeUrl: true\n  timeout: 0\n  followRedirects: true\n  maxRedirects: 5\n  forwardAuthorizationHeader: true\n";

/// Whether `section` of a request file `text` shows its dot, checked on the model and the tab.
fn dotted(cx: &mut TestAppContext, root: &str, text: &str, section: Section) -> bool {
    let col = write_tree(&[("opencollection.yml", root), ("n.yml", text)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "n.yml", false);
    let marked = ed.read_with(cx, |e, cx| e.section_marked(section, cx));
    let drawn = present(cx, win, ("section-mode", section as usize));
    assert_eq!(marked, drawn, "the dot follows section_marked");
    marked
}

const BEARER_ROOT: &str =
    "opencollection: 1.0.0\ninfo:\n  name: c\nrequest:\n  auth:\n    type: bearer\n    token: t\n";

#[gpui_kit::test]
fn a_new_http_request_inheriting_auth_has_no_auth_dot(cx: &mut TestAppContext) {
    let text = new_file("http", "", NEW_SETTINGS);
    assert!(!dotted(cx, BEARER_ROOT, &text, Section::Auth));
}

#[gpui_kit::test]
fn a_new_graphql_request_inheriting_auth_has_no_auth_dot(cx: &mut TestAppContext) {
    let text = new_file("graphql", "", NEW_SETTINGS);
    assert!(!dotted(cx, BEARER_ROOT, &text, Section::Auth));
}

#[gpui_kit::test]
fn a_graphql_request_with_its_own_auth_has_the_auth_dot(cx: &mut TestAppContext) {
    let text = new_file("graphql", "", NEW_SETTINGS).replace(
        "  auth: inherit\n",
        "  auth:\n    type: bearer\n    token: t\n",
    );
    assert!(dotted(cx, ROOT, &text, Section::Auth));
}

#[gpui_kit::test]
fn default_settings_leave_the_settings_tab_plain(cx: &mut TestAppContext) {
    for kind in ["http", "graphql"] {
        for settings in [NEW_SETTINGS, OLD_SETTINGS] {
            let text = new_file(kind, "", settings);
            assert!(
                !dotted(cx, ROOT, &text, Section::Settings),
                "{kind}: {settings}"
            );
        }
    }
    let bare = "info:\n  name: n\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: u\n";
    assert!(
        !dotted(cx, ROOT, bare, Section::Settings),
        "no settings block"
    );
}

#[gpui_kit::test]
fn each_custom_setting_marks_the_settings_tab(cx: &mut TestAppContext) {
    let custom = [
        ("encodeUrl: true", "encodeUrl: false"),
        ("timeout: inherit", "timeout: 2500"),
        ("followRedirects: true", "followRedirects: false"),
        ("maxRedirects: 5", "maxRedirects: 9"),
    ];
    for kind in ["http", "graphql"] {
        for (from, to) in custom {
            let text = new_file(kind, "", &NEW_SETTINGS.replace(from, to));
            assert!(dotted(cx, ROOT, &text, Section::Settings), "{kind}: {to}");
        }
        let tagged = new_file(kind, "", NEW_SETTINGS)
            .replace("  seq: 1\n", "  seq: 1\n  tags:\n    - smoke\n");
        assert!(dotted(cx, ROOT, &tagged, Section::Settings), "{kind}: tags");
    }
}

#[gpui_kit::test]
fn the_settings_dot_says_what_it_marks(cx: &mut TestAppContext) {
    let text = new_file(
        "http",
        "",
        &NEW_SETTINGS.replace("timeout: inherit", "timeout: 2500"),
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("n.yml", &text)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (_win, ed) = editor_for(cx, &ws, &root, "n.yml", false);
    let words = ed.read_with(cx, |e, cx| e.section_mark(Section::Settings, cx));
    assert_eq!(words.as_deref(), Some("Has custom settings"));
}

#[gpui_kit::test]
fn docs_mark_the_docs_tab(cx: &mut TestAppContext) {
    let with_docs = format!(
        "{}docs: \"Lists users.\"\n",
        new_file("http", "", NEW_SETTINGS)
    );
    assert!(dotted(cx, ROOT, &with_docs, Section::Docs));
    assert!(!dotted(
        cx,
        ROOT,
        &new_file("http", "", NEW_SETTINGS),
        Section::Docs
    ));
}
