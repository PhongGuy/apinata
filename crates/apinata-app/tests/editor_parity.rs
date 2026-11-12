use crate::support;

use apinata_app::assertions_section::AssertionsSection;
use apinata_app::auth_section::{AuthMode, AuthSection};
use apinata_app::body_section::{BodyMode, BodySection};
use apinata_app::command::{Command, Target};
use apinata_app::request_editor::{RequestEditor, Section};
use apinata_app::settings_section::{SettingsSection, TimeoutMode};
use apinata_app::workspace::Workspace;
use apinata_core::ApiKeyPlacement;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Context, Entity, TestAppContext, Window};
use std::path::{Path, PathBuf};
use support::*;

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../apinata-core/tests/fixtures/reference/2026-10-09-edit-parity")
}

struct Case {
    col: tempfile::TempDir,
    _state: tempfile::TempDir,
    ws: Entity<Workspace>,
    root: PathBuf,
    rel: String,
    win: AnyWindowHandle,
    ed: Entity<RequestEditor>,
}

/// Open a copy of the case's normalised fixture as a one-request collection, in an editor.
fn case(cx: &mut TestAppContext, id: &str) -> Case {
    let cases: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(corpus().join("cases.json")).unwrap())
            .unwrap();
    let fixture = cases
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .and_then(|c| c["fixture"].as_str())
        .expect("a case")
        .to_owned();
    let rel = Path::new(&fixture)
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let text = std::fs::read_to_string(corpus().join("normalized").join(&fixture)).unwrap();
    let col = write_tree(&[("opencollection.yml", ROOT), (rel.as_str(), text.as_str())]);
    let (state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, &rel, false);
    Case {
        col,
        _state: state,
        ws,
        root,
        rel,
        win,
        ed,
    }
}

fn finish(cx: &mut TestAppContext, c: &Case, id: &str) {
    cx.run_until_parked();
    c.ws.update(cx, |w, cx| {
        w.send(&c.root, Command::Save(Target::Request(ip(&c.rel))), cx)
    });
    cx.run_until_parked();
    let ours = std::fs::read_to_string(c.col.path().join(&c.rel)).unwrap();
    let bruno =
        std::fs::read_to_string(corpus().join("expected").join(format!("{id}.yml"))).unwrap();
    assert_eq!(ours, bruno, "{id}");
}

fn open_section(cx: &mut TestAppContext, c: &Case, s: Section) {
    cx.update_window(c.win, |_, w, cx| {
        w.click(("section", s as usize), cx);
        w.render_frame(cx)
    })
    .unwrap();
}

fn act<V: 'static>(
    cx: &mut TestAppContext,
    c: &Case,
    pick: impl FnOnce(&RequestEditor) -> Entity<V>,
    f: impl FnOnce(&mut V, &mut Window, &mut Context<V>),
) {
    let view = c.ed.read_with(cx, |e, _| pick(e));
    cx.update_window(c.win, |_, w, cx| view.update(cx, |v, cx| f(v, w, cx)))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(c.win, |_, w, cx| w.render_frame(cx))
        .unwrap();
}

fn body_mode(cx: &mut TestAppContext, c: &Case, m: BodyMode) {
    open_section(cx, c, Section::Body);
    act(
        cx,
        c,
        |e| e.body().clone(),
        |b: &mut BodySection, w, cx| b.set_mode(m, w, cx),
    );
}

fn auth_mode(cx: &mut TestAppContext, c: &Case, m: AuthMode) {
    open_section(cx, c, Section::Auth);
    act(
        cx,
        c,
        |e| e.auth().clone(),
        |a: &mut AuthSection, w, cx| a.set_mode(m, w, cx),
    );
}

fn click(
    cx: &mut TestAppContext,
    c: &Case,
    scope: &'static str,
    id: impl Into<gpui_kit::ElementId>,
) {
    cx.update_window(c.win, |_, w, cx| {
        w.render_frame(cx);
        w.within(scope).click(id, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

fn typed(
    cx: &mut TestAppContext,
    c: &Case,
    scope: &'static str,
    id: impl Into<gpui_kit::ElementId>,
    text: &str,
) {
    type_at_end(cx, c.win, Some(scope), id, text);
    cx.run_until_parked();
}

/// Answer the next file prompt with paths under the tab's collection root.
fn pick(cx: &mut TestAppContext, c: &Case, ix: usize, rels: &[&str]) {
    pick_with(cx, c, ("kv-pick", ix), rels);
}

// Picks `rels` through the button `id`, such as an empty file body's first-file button.
fn pick_with(cx: &mut TestAppContext, c: &Case, id: impl Into<gpui_kit::ElementId>, rels: &[&str]) {
    let root =
        c.ws.read_with(cx, |w, _| w.tabs().active().unwrap().root.clone());
    click(cx, c, "body", id);
    let paths: Vec<PathBuf> = rels.iter().map(|r| root.join(r)).collect();
    cx.simulate_path_prompt_response(move |_| Some(paths));
    cx.run_until_parked();
}

fn kv_row(cx: &mut TestAppContext, c: &Case, ix: usize, name: &str, value: &str) {
    typed(cx, c, "body", ("kv-name", ix), name);
    if !value.is_empty() {
        typed(cx, c, "body", ("kv-value", ix), value);
    }
}

#[gpui_kit::test]
fn body_json_edit(cx: &mut TestAppContext) {
    let c = case(cx, "body-json-edit");
    open_section(cx, &c, Section::Body);
    replace_code(cx, c.win, Some("body"), "body-code", "{\"a\": 1");
    finish(cx, &c, "body-json-edit");
}

#[gpui_kit::test]
fn body_json_to_form(cx: &mut TestAppContext) {
    let c = case(cx, "body-json-to-form");
    body_mode(cx, &c, BodyMode::Form);
    kv_row(cx, &c, 0, "a", "1");
    kv_row(cx, &c, 1, "b", "2");
    click(cx, &c, "body", ("kv-enabled", 1usize));
    finish(cx, &c, "body-json-to-form");
}

#[gpui_kit::test]
fn body_json_to_none(cx: &mut TestAppContext) {
    let c = case(cx, "body-json-to-none");
    body_mode(cx, &c, BodyMode::None);
    finish(cx, &c, "body-json-to-none");
}

#[gpui_kit::test]
fn body_text(cx: &mut TestAppContext) {
    let c = case(cx, "body-text");
    body_mode(cx, &c, BodyMode::Text);
    replace_code(cx, c.win, Some("body"), "body-code", "hello");
    cx.update_window(c.win, |_, w, cx| {
        w.press("enter", cx);
        w.input("world", cx)
    })
    .unwrap();
    finish(cx, &c, "body-text");
}

#[gpui_kit::test]
fn body_xml(cx: &mut TestAppContext) {
    let c = case(cx, "body-xml");
    body_mode(cx, &c, BodyMode::Xml);
    replace_code(cx, c.win, Some("body"), "body-code", "<a/>");
    finish(cx, &c, "body-xml");
}

#[gpui_kit::test]
fn body_form_empty(cx: &mut TestAppContext) {
    let c = case(cx, "body-form-empty");
    body_mode(cx, &c, BodyMode::Form);
    finish(cx, &c, "body-form-empty");
}

#[gpui_kit::test]
fn body_form_edit_one(cx: &mut TestAppContext) {
    let c = case(cx, "body-form-edit-one");
    open_section(cx, &c, Section::Body);
    replace_input(cx, c.win, Some("body"), ("kv-value", 0usize), "plain");
    finish(cx, &c, "body-form-edit-one");
}

#[gpui_kit::test]
fn body_multipart_new(cx: &mut TestAppContext) {
    let c = case(cx, "body-multipart-new");
    body_mode(cx, &c, BodyMode::Multipart);
    kv_row(cx, &c, 0, "t", "v");
    kv_row(cx, &c, 1, "f", "");
    pick(cx, &c, 1, &["a.png", "b.txt"]);
    typed(cx, &c, "body", ("kv-ctype", 1usize), "image/png");
    kv_row(cx, &c, 2, "off", "x");
    click(cx, &c, "body", ("kv-enabled", 2usize));
    finish(cx, &c, "body-multipart-new");
}

#[gpui_kit::test]
fn body_multipart_disable(cx: &mut TestAppContext) {
    let c = case(cx, "body-multipart-disable");
    open_section(cx, &c, Section::Body);
    click(cx, &c, "body", ("kv-enabled", 2usize));
    finish(cx, &c, "body-multipart-disable");
}

#[gpui_kit::test]
fn body_file(cx: &mut TestAppContext) {
    let c = case(cx, "body-file");
    body_mode(cx, &c, BodyMode::File);
    // A pick fills a blank content type, which the reference editor does not; the cells are
    // retyped and cleared so the saved entries still match the reference's.
    pick_with(cx, &c, "body-file-choose", &["data/a.bin"]);
    replace_input(
        cx,
        c.win,
        Some("body"),
        ("kv-ctype", 0usize),
        "application/octet-stream",
    );
    cx.run_until_parked();
    pick(cx, &c, 1, &["b.json"]);
    replace_input(cx, c.win, Some("body"), ("kv-ctype", 1usize), "");
    finish(cx, &c, "body-file");
}

#[gpui_kit::test]
fn auth_basic(cx: &mut TestAppContext) {
    let c = case(cx, "auth-basic");
    auth_mode(cx, &c, AuthMode::Basic);
    typed(cx, &c, "auth", "auth-username", "u");
    typed(cx, &c, "auth", "auth-password", "{{pw}}");
    finish(cx, &c, "auth-basic");
}

#[gpui_kit::test]
fn auth_bearer(cx: &mut TestAppContext) {
    let c = case(cx, "auth-bearer");
    auth_mode(cx, &c, AuthMode::Bearer);
    typed(cx, &c, "auth", "auth-token", "t0k");
    finish(cx, &c, "auth-bearer");
}

#[gpui_kit::test]
fn auth_apikey_query(cx: &mut TestAppContext) {
    let c = case(cx, "auth-apikey-query");
    auth_mode(cx, &c, AuthMode::ApiKey);
    typed(cx, &c, "auth", "auth-key", "k");
    typed(cx, &c, "auth", "auth-value", "v");
    act(
        cx,
        &c,
        |e| e.auth().clone(),
        |a: &mut AuthSection, w, cx| a.set_placement(ApiKeyPlacement::Query, w, cx),
    );
    finish(cx, &c, "auth-apikey-query");
}

#[gpui_kit::test]
fn auth_inherit(cx: &mut TestAppContext) {
    let c = case(cx, "auth-inherit");
    auth_mode(cx, &c, AuthMode::Inherit);
    finish(cx, &c, "auth-inherit");
}

#[gpui_kit::test]
fn assert_add(cx: &mut TestAppContext) {
    let c = case(cx, "assert-add");
    open_section(cx, &c, Section::Assertions);
    typed(cx, &c, "assertions", ("as-expr", 0usize), "res.status");
    typed(cx, &c, "assertions", ("as-value", 0usize), "200");
    typed(cx, &c, "assertions", ("as-expr", 1usize), "res.body.ok");
    act(
        cx,
        &c,
        |e| e.assertions().clone(),
        |a: &mut AssertionsSection, w, cx| a.set_operator(1, "isTruthy", w, cx),
    );
    click(cx, &c, "assertions", ("as-enabled", 1usize));
    finish(cx, &c, "assert-add");
}

#[gpui_kit::test]
fn assert_edit(cx: &mut TestAppContext) {
    let c = case(cx, "assert-edit");
    open_section(cx, &c, Section::Assertions);
    act(
        cx,
        &c,
        |e| e.assertions().clone(),
        |a: &mut AssertionsSection, w, cx| a.set_operator(0, "neq", w, cx),
    );
    replace_input(cx, c.win, Some("assertions"), ("as-value", 0usize), "500");
    finish(cx, &c, "assert-edit");
}

#[gpui_kit::test]
fn assert_clear(cx: &mut TestAppContext) {
    let c = case(cx, "assert-clear");
    open_section(cx, &c, Section::Assertions);
    click(cx, &c, "assertions", ("as-delete", 0usize));
    finish(cx, &c, "assert-clear");
}

#[gpui_kit::test]
fn settings_timeout(cx: &mut TestAppContext) {
    let c = case(cx, "settings-timeout");
    open_section(cx, &c, Section::Settings);
    replace_input(cx, c.win, Some("settings"), "set-timeout", "5000");
    finish(cx, &c, "settings-timeout");
}

#[gpui_kit::test]
fn settings_all(cx: &mut TestAppContext) {
    let c = case(cx, "settings-all");
    open_section(cx, &c, Section::Settings);
    // The limit is set first: with following off its field is disabled.
    replace_input(cx, c.win, Some("settings"), "set-max-redirects", "2");
    cx.run_until_parked();
    for id in ["set-encode-url", "set-follow-redirects", "set-forward-auth"] {
        click(cx, &c, "settings", id);
    }
    act(
        cx,
        &c,
        |e| e.settings().clone(),
        |s: &mut SettingsSection, w, cx| s.set_timeout_mode(TimeoutMode::Inherit, w, cx),
    );
    finish(cx, &c, "settings-all");
}

#[gpui_kit::test]
fn tags_set(cx: &mut TestAppContext) {
    let c = case(cx, "tags-set");
    open_section(cx, &c, Section::Settings);
    cx.update_window(c.win, |_, w, cx| {
        w.within("settings").click("tag-text", cx);
        w.input("smoke,", cx);
        w.input(" v2 ", cx);
        w.press("enter", cx);
    })
    .unwrap();
    finish(cx, &c, "tags-set");
}

#[gpui_kit::test]
fn docs_set(cx: &mut TestAppContext) {
    let c = case(cx, "docs-set");
    open_section(cx, &c, Section::Docs);
    replace_code(cx, c.win, Some("docs"), "docs-code", "# Title");
    cx.update_window(c.win, |_, w, cx| {
        w.press("enter", cx);
        w.input("line two", cx)
    })
    .unwrap();
    finish(cx, &c, "docs-set");
}

#[gpui_kit::test]
fn a_pre_request_script_saves_as_bruno_writes_it(cx: &mut TestAppContext) {
    use apinata_app::command::Edit;
    use apinata_core::ScriptPhase;
    let research =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../apinata-core/tests/fixtures/reference");
    let input = std::fs::read_to_string(
        research.join("2026-10-09-edit-parity/normalized/cli-docs/request_1.yml"),
    )
    .unwrap();
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("request_1.yml", input.as_str()),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let edit = Command::Edit {
        target: Target::Request(ip("request_1.yml")),
        edit: Edit::Script(
            ScriptPhase::PreRequest,
            "bru.setVar(\"t\", Date.now());".into(),
        ),
    };
    ws.update(cx, |w, cx| w.send(&root, edit, cx));
    ws.update(cx, |w, cx| {
        w.send(
            &root,
            Command::Save(Target::Request(ip("request_1.yml"))),
            cx,
        )
    });
    cx.run_until_parked();
    let want = std::fs::read_to_string(
        research.join("2026-10-11-script-edit-parity/expected/script-add-pre/request_1.yml"),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(col.path().join("request_1.yml")).unwrap(),
        want
    );
}
