//! The Post-response table in the Vars tabs.

use crate::support;

use apinata_app::request_editor::Section;
use apinata_core::{Collection, PostResponseVar};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext, VisualTestContext};
use support::*;

const A: &str = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://x/0\"\nruntime:\n  variables:\n    - name: token\n      value: abc\n";
const CAPTURED: &str = "info:\n  name: c\n  type: http\n  seq: 2\nhttp:\n  method: GET\n  url: \"http://x/1\"\nruntime:\n  actions:\n    - type: set-variable\n      phase: after-response\n      selector:\n        expression: res.body.id\n        method: jsonq\n      variable:\n        name: id\n        scope: runtime\n";

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", A),
        ("c.yml", CAPTURED),
    ])
}

fn vars_tab(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle) {
    cx.update_window(win, |_, w, cx| {
        w.click(("section", Section::Vars as usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_typed_post_response_var_saves_what_core_saves(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    vars_tab(cx, win);
    type_at_end(
        cx,
        win,
        Some("post-response-vars"),
        ("kv-name", 0usize),
        "id",
    );
    cx.run_until_parked();
    type_at_end(
        cx,
        win,
        Some("post-response-vars"),
        ("kv-value", 0usize),
        "res.body.id",
    );
    cx.run_until_parked();
    let view = view_of(cx, &ws, &root, "a.yml");
    assert_eq!(
        view.post_response_vars,
        [PostResponseVar::new("id", "res.body.id")]
    );
    assert_eq!(view.vars.len(), 1);
    assert!(view.dirty);
    let mut vcx = VisualTestContext::from_window(win, cx);
    vcx.simulate_keystrokes(&key("s"));
    vcx.run_until_parked();

    let twin = tree();
    let mut c = Collection::open(twin.path()).unwrap();
    let r = c.request_mut(&ip("a.yml")).unwrap();
    r.set_post_response_vars(vec![PostResponseVar::new("id", "res.body.id")]);
    r.save().unwrap();
    let ours = std::fs::read_to_string(col.path().join("a.yml")).unwrap();
    let core = std::fs::read_to_string(twin.path().join("a.yml")).unwrap();
    assert_eq!(ours, core);
}

#[gpui_kit::test]
fn a_file_s_post_response_vars_fill_the_table(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "c.yml", false);
    vars_tab(cx, win);
    let rows = ed.read_with(cx, |e, cx| e.post_response_table().read(cx).rows(cx));
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (
            rows[0].name.as_str(),
            rows[0].value.as_str(),
            rows[0].enabled
        ),
        ("id", "res.body.id", true)
    );
    assert_eq!(
        ed.read_with(cx, |e, cx| e.post_response_table().read(cx).placeholders()),
        ("Name", "e.g. res.body.token")
    );
    assert!(present(cx, win, "post-response-vars"));
}

#[gpui_kit::test]
fn an_expression_needing_javascript_shows_a_hint(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    vars_tab(cx, win);
    type_at_end(
        cx,
        win,
        Some("post-response-vars"),
        ("kv-name", 0usize),
        "id",
    );
    cx.run_until_parked();
    assert!(
        ed.read_with(cx, |e, cx| e.post_response_hints(cx))
            .is_empty(),
        "no hint before an expression"
    );
    type_at_end(
        cx,
        win,
        Some("post-response-vars"),
        ("kv-value", 0usize),
        "res('id')",
    );
    cx.run_until_parked();
    assert_eq!(
        ed.read_with(cx, |e, cx| e.post_response_hints(cx)),
        ["`res('id')` needs a JavaScript runtime"]
    );
    assert!(present(cx, win, ("post-response-hint", 0usize)));
}

#[gpui_kit::test]
fn a_supported_expression_has_no_hint(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "c.yml", false);
    vars_tab(cx, win);
    assert!(
        ed.read_with(cx, |e, cx| e.post_response_hints(cx))
            .is_empty()
    );
    assert!(!present(cx, win, ("post-response-hint", 0usize)));
}

use apinata_app::config_editor::{ConfigEditor, ConfigSection, ConfigTarget};
use apinata_app::kv_table::KvRow;

fn config_tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("Users")),
        ("users/a.yml", &request("a", "http://x")),
    ])
}

fn config_editor(
    cx: &mut TestAppContext,
    ws: &gpui_kit::Entity<apinata_app::workspace::Workspace>,
    root: &std::path::Path,
    target: ConfigTarget,
) -> (gpui_kit::AnyWindowHandle, gpui_kit::Entity<ConfigEditor>) {
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let (ws, root) = (ws.clone(), root.to_path_buf());
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ConfigEditor::new(ws, root, target, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    (win, view)
}

fn kv(name: &str, value: &str) -> KvRow {
    KvRow {
        name: name.into(),
        value: value.into(),
        enabled: true,
        ..KvRow::default()
    }
}

#[gpui_kit::test]
fn a_folder_post_response_var_saves_what_core_saves(cx: &mut TestAppContext) {
    let (col, twin) = (config_tree(), config_tree());
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = config_editor(cx, &ws, &root, ConfigTarget::Folder(ip("users")));
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| {
            e.edit_post_response_rows(vec![kv("token", "res.body.token")], w, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    let users = ws
        .read_with(cx, |w, cx| w.snapshot(&root, cx))
        .unwrap()
        .content
        .folders[&ip("users")]
        .clone();
    assert!(users.dirty);
    assert_eq!(
        users.post_response_vars,
        [PostResponseVar::new("token", "res.body.token")]
    );
    ed.update(cx, |e, cx| e.save(cx));
    cx.run_until_parked();

    let mut c = Collection::open(twin.path()).unwrap();
    let f = c.folder_file_mut(&ip("users")).unwrap();
    f.set_post_response_vars(vec![PostResponseVar::new("token", "res.body.token")]);
    f.save().unwrap();
    let rel = "users/folder.yml";
    assert_eq!(
        std::fs::read_to_string(col.path().join(rel)).unwrap(),
        std::fs::read_to_string(twin.path().join(rel)).unwrap()
    );
}

#[gpui_kit::test]
fn the_collection_settings_show_post_response_rows_and_hints(cx: &mut TestAppContext) {
    let col = config_tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = config_editor(cx, &ws, &root, ConfigTarget::Collection);
    ed.update(cx, |e, cx| e.set_section(ConfigSection::Vars, cx));
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| {
            e.edit_post_response_rows(vec![kv("x", "res.cookies.a")], w, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        ed.read_with(cx, |e, cx| e.post_response_hints(cx)),
        ["`res.cookies.a` needs a JavaScript runtime"]
    );
    assert!(present(cx, win, "config-post-response-vars"));
    assert!(present(cx, win, ("config-post-response-hint", 0usize)));
}

const MIXED: &str = "info:\n  name: m\n  type: http\n  seq: 3\nhttp:\n  method: GET\n  url: \"http://x/2\"\nruntime:\n  variables:\n    - name: token\n      value: abc\n  actions:\n    - type: set-variable\n      phase: after-response\n      selector:\n        expression: res.body.id\n        method: jsonq\n      variable:\n        name: id\n        scope: runtime\n    - type: set-variable\n      phase: after-response\n      disabled: true\n      selector:\n        expression: res.body.off\n        method: jsonq\n      variable:\n        name: off\n        scope: runtime\n";

#[gpui_kit::test]
fn the_vars_badge_counts_enabled_post_response_vars(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("m.yml", MIXED)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (_win, ed) = editor_for(cx, &ws, &root, "m.yml", false);
    assert_eq!(
        ed.read_with(cx, |e, cx| e.section_count(Section::Vars, cx)),
        Some(2)
    );
}

#[gpui_kit::test]
fn every_vars_table_reads_name_and_value(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    vars_tab(cx, win);
    let heads = |cx: &mut TestAppContext, post: bool| {
        ed.read_with(cx, |e, cx| {
            let t = if post {
                e.post_response_table()
            } else {
                e.table(Section::Vars).unwrap()
            };
            t.read(cx).headings()
        })
    };
    assert_eq!(heads(cx, true), ["Name", "Value"]);
    assert_eq!(heads(cx, false), ["Name", "Value"]);
    let names = ed.read_with(cx, |e, cx| {
        e.table(Section::Vars).unwrap().read(cx).placeholders().0
    });
    assert_eq!(names, "Name");

    let col = config_tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (_win, ed) = config_editor(cx, &ws, &root, ConfigTarget::Collection);
    assert_eq!(
        ed.read_with(cx, |e, cx| e.post_response_table().read(cx).headings()),
        ["Name", "Value"]
    );
    assert_eq!(
        ed.read_with(cx, |e, cx| e
            .table(ConfigSection::Vars)
            .unwrap()
            .read(cx)
            .headings()),
        ["Name", "Value"]
    );
}

// Asserts a muted "Pre-request" heading `heading` is drawn above the table wrapper `table`.
fn heading_above(
    cx: &mut TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    heading: &'static str,
    table: &'static str,
) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let h = w.try_find(heading).expect("a Pre-request heading");
        assert_eq!(h.label(), Some("Pre-request"));
        let t = w.find(table).bounds();
        assert!(h.bounds().bottom() <= t.top(), "{:?} vs {t:?}", h.bounds());
    })
    .unwrap();
}

// Whether `heading` reads `label`, sits above `table`, and matches `twin`'s height and inset.
fn heading_matches(
    cx: &mut TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    (heading, label): (&'static str, &str),
    table: &'static str,
    twin: &'static str,
) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let head = w.find(heading);
        assert_eq!(head.label(), Some(label));
        let (h, t, p) = (head.bounds(), w.find(table).bounds(), w.find(twin).bounds());
        assert!(
            h.bottom() <= t.top() + gpui_kit::px(0.5),
            "{h:?} above {t:?}"
        );
        assert_eq!(h.size.height, p.size.height, "same text size as {twin}");
        assert_eq!(h.left(), p.left(), "same inset as {twin}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_vars_tabs_head_the_first_table_pre_request(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    vars_tab(cx, win);
    heading_above(cx, win, "pre-request-heading", "vars");

    let col = config_tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = config_editor(cx, &ws, &root, ConfigTarget::Folder(ip("users")));
    ed.update(cx, |e, cx| e.set_section(ConfigSection::Vars, cx));
    cx.run_until_parked();
    heading_above(cx, win, "config-pre-request-heading", "config-vars");
}

#[gpui_kit::test]
fn the_vars_table_has_a_pre_request_heading(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    vars_tab(cx, win);
    heading_matches(
        cx,
        win,
        ("pre-request-heading", "Pre-request"),
        "vars",
        "post-response-heading",
    );
}

#[gpui_kit::test]
fn the_config_vars_table_has_a_pre_request_heading(cx: &mut TestAppContext) {
    let col = config_tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = config_editor(cx, &ws, &root, ConfigTarget::Folder(ip("users")));
    ed.update(cx, |e, cx| e.set_section(ConfigSection::Vars, cx));
    cx.run_until_parked();
    heading_matches(
        cx,
        win,
        ("config-pre-request-heading", "Pre-request"),
        "config-vars",
        "config-post-response-heading",
    );
}
