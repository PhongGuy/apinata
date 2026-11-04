//! Undo and redo of the structured edits of a request tab.

use crate::support;

use apinata_app::request_editor::{RequestEditor, Section};
use apinata_app::snapshot::RequestView;
use apinata_core::{Assertion, BodySpec};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext};
use std::sync::Arc;
use support::*;

const FILE: &str = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: \"http://x/0?a=1\"\n  params:\n    - name: a\n      value: \"1\"\n      type: query\n  headers:\n    - name: H1\n      value: one\n    - name: H2\n      value: two\n  body:\n    type: form-urlencoded\n    data:\n      - name: f\n        value: v\nruntime:\n  variables:\n    - name: token\n      value: abc\n";
const JSON: &str = "info:\n  name: j\n  type: http\n  seq: 2\nhttp:\n  method: POST\n  url: \"http://x/1\"\n  body:\n    type: json\n    data: '{}'\n";

const BEARER: &str = "info:\n  name: b\n  type: http\n  seq: 3\nhttp:\n  method: GET\n  url: \"http://x/2\"\n  auth:\n    type: bearer\n    token: t\n";
const PLAIN: &str = "info:\n  name: p\n  type: http\n  seq: 5\nhttp:\n  method: GET\n  url: http://x\n  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: http://id/token\n    credentials:\n      clientId: svc\n      clientSecret: old-plain\n";
const OAUTH2: &str = "info:\n  name: o\n  type: http\n  seq: 4\nhttp:\n  method: GET\n  url: http://x\n  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: http://id/token\n";

struct Rig {
    _col: tempfile::TempDir,
    _state: tempfile::TempDir,
    ws: Entity<apinata_app::workspace::Workspace>,
    root: std::path::PathBuf,
    win: AnyWindowHandle,
    ed: Entity<RequestEditor>,
}

fn rig(cx: &mut TestAppContext, rel: &str) -> Rig {
    support::init(cx);
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", FILE),
        ("j.yml", JSON),
        ("b.yml", BEARER),
        ("o.yml", OAUTH2),
        ("p.yml", PLAIN),
    ]);
    let (state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, rel, false);
    Rig {
        _col: col,
        _state: state,
        ws,
        root,
        win,
        ed,
    }
}

impl Rig {
    fn show(&self, cx: &mut TestAppContext, section: Section) {
        cx.update_window(self.win, |_, w, cx| {
            w.click(("section", section as usize), cx)
        })
        .unwrap();
        cx.run_until_parked();
        frame(cx, self.win);
    }

    fn click(
        &self,
        cx: &mut TestAppContext,
        scope: &'static str,
        id: impl Into<gpui_kit::ElementId>,
    ) {
        cx.update_window(self.win, |_, w, cx| w.within(scope).click(id, cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn undo(&self, cx: &mut TestAppContext) {
        cx.update_window(self.win, |_, w, cx| {
            self.ed.update(cx, |e, cx| e.undo_edit(w, cx))
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn redo(&self, cx: &mut TestAppContext) {
        cx.update_window(self.win, |_, w, cx| {
            self.ed.update(cx, |e, cx| e.redo_edit(w, cx))
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn view(
        &self,
        cx: &mut TestAppContext,
        rel: &str,
    ) -> std::sync::Arc<apinata_app::snapshot::RequestView> {
        view_of(cx, &self.ws, &self.root, rel)
    }

    fn rows(&self, cx: &mut TestAppContext, section: Section) -> Vec<(String, String, bool)> {
        let table = self
            .ed
            .read_with(cx, |e, _| e.table(section).cloned())
            .unwrap();
        table.read_with(cx, |t, cx| {
            t.rows(cx)
                .into_iter()
                .map(|r| (r.name, r.value, r.enabled))
                .collect()
        })
    }
}

fn names(v: &[apinata_core::KeyValue]) -> Vec<(String, bool)> {
    v.iter().map(|h| (h.name.clone(), h.enabled)).collect()
}

fn var_value(v: &apinata_app::snapshot::RequestView) -> String {
    apinata_app::kv_table::var_rows(&v.vars).remove(0).value
}

fn on(n: &str) -> (String, bool) {
    (n.into(), true)
}

fn off(n: &str) -> (String, bool) {
    (n.into(), false)
}

#[gpui_kit::test]
fn a_param_add_is_undone_and_redone(cx: &mut TestAppContext) {
    let r = rig(cx, "a.yml");
    type_at_end(cx, r.win, Some("params"), ("kv-name", 1usize), "b");
    cx.run_until_parked();
    assert_eq!(r.view(cx, "a.yml").params.len(), 2);
    r.undo(cx);
    let v = r.view(cx, "a.yml");
    assert_eq!(v.params.len(), 1);
    assert_eq!(v.url, "http://x/0?a=1");
    assert_eq!(r.rows(cx, Section::Params).len(), 1);
    r.redo(cx);
    assert_eq!(r.view(cx, "a.yml").params.len(), 2);
    assert_eq!(r.view(cx, "a.yml").url, "http://x/0?a=1&b=");
}

#[gpui_kit::test]
fn a_header_remove_is_undone(cx: &mut TestAppContext) {
    let r = rig(cx, "a.yml");
    r.show(cx, Section::Headers);
    r.click(cx, "headers", ("kv-delete", 0usize));
    assert_eq!(names(&r.view(cx, "a.yml").headers), [on("H2")]);
    r.undo(cx);
    assert_eq!(names(&r.view(cx, "a.yml").headers), [on("H1"), on("H2")]);
    assert_eq!(r.rows(cx, Section::Headers).len(), 2);
    r.redo(cx);
    assert_eq!(names(&r.view(cx, "a.yml").headers), [on("H2")]);
}

#[gpui_kit::test]
fn one_header_toggle_is_one_step(cx: &mut TestAppContext) {
    let r = rig(cx, "a.yml");
    r.show(cx, Section::Headers);
    r.click(cx, "headers", ("kv-enabled", 0usize));
    assert_eq!(names(&r.view(cx, "a.yml").headers), [off("H1"), on("H2")]);
    r.undo(cx);
    assert_eq!(names(&r.view(cx, "a.yml").headers), [on("H1"), on("H2")]);
    r.redo(cx);
    assert_eq!(names(&r.view(cx, "a.yml").headers), [off("H1"), on("H2")]);
}

#[gpui_kit::test]
fn toggle_all_is_one_step_each_way(cx: &mut TestAppContext) {
    let r = rig(cx, "a.yml");
    r.show(cx, Section::Headers);
    r.click(cx, "headers", "kv-head-enabled");
    assert_eq!(names(&r.view(cx, "a.yml").headers), [off("H1"), off("H2")]);
    r.undo(cx);
    assert_eq!(names(&r.view(cx, "a.yml").headers), [on("H1"), on("H2")]);
    r.redo(cx);
    assert_eq!(names(&r.view(cx, "a.yml").headers), [off("H1"), off("H2")]);
}

#[gpui_kit::test]
fn several_keystrokes_in_a_var_cell_are_one_step(cx: &mut TestAppContext) {
    let r = rig(cx, "a.yml");
    r.show(cx, Section::Vars);
    for ch in ["x", "y", "z"] {
        type_at_end(cx, r.win, Some("vars"), ("kv-value", 0usize), ch);
        cx.run_until_parked();
    }
    assert_eq!(var_value(&r.view(cx, "a.yml")), "abcxyz");
    r.undo(cx);
    assert_eq!(var_value(&r.view(cx, "a.yml")), "abc");
    assert_eq!(r.rows(cx, Section::Vars)[0].1, "abc");
    r.redo(cx);
    assert_eq!(var_value(&r.view(cx, "a.yml")), "abcxyz");
}

#[gpui_kit::test]
fn a_form_body_row_add_is_undone(cx: &mut TestAppContext) {
    let r = rig(cx, "a.yml");
    r.show(cx, Section::Body);
    type_at_end(cx, r.win, Some("body"), ("kv-name", 1usize), "g");
    cx.run_until_parked();
    let rows = |r: &Rig, cx: &mut TestAppContext| match &r.view(cx, "a.yml").body {
        BodySpec::FormUrlEncoded(kv) => kv.len(),
        other => panic!("{other:?}"),
    };
    assert_eq!(rows(&r, cx), 2);
    r.undo(cx);
    assert_eq!(rows(&r, cx), 1);
    r.redo(cx);
    assert_eq!(rows(&r, cx), 2);
}

#[gpui_kit::test]
fn an_assertion_add_and_edit_are_undone(cx: &mut TestAppContext) {
    let r = rig(cx, "a.yml");
    r.show(cx, Section::Assertions);
    let a = Assertion::new("res.status", "eq", Some("200".to_string()));
    cx.update_window(r.win, |_, w, cx| {
        let section = r.ed.read(cx).assertions().clone();
        section.update(cx, |s, cx| s.add_assertion(&a, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(r.view(cx, "a.yml").assertions.len(), 1);
    type_at_end(cx, r.win, Some("assertions"), ("as-value", 0usize), "1");
    cx.run_until_parked();
    assert_eq!(r.view(cx, "a.yml").assertions[0].value(), Some("2001"));
    r.undo(cx);
    assert_eq!(r.view(cx, "a.yml").assertions[0].value(), Some("200"));
    r.undo(cx);
    assert!(r.view(cx, "a.yml").assertions.is_empty());
    r.redo(cx);
    r.redo(cx);
    assert_eq!(r.view(cx, "a.yml").assertions[0].value(), Some("2001"));
}

#[gpui_kit::test]
fn a_post_response_var_is_undone(cx: &mut TestAppContext) {
    let r = rig(cx, "a.yml");
    r.show(cx, Section::Vars);
    type_at_end(
        cx,
        r.win,
        Some("post-response-vars"),
        ("kv-name", 0usize),
        "id",
    );
    cx.run_until_parked();
    assert_eq!(r.view(cx, "a.yml").post_response_vars.len(), 1);
    r.undo(cx);
    assert!(r.view(cx, "a.yml").post_response_vars.is_empty());
    r.redo(cx);
    assert_eq!(r.view(cx, "a.yml").post_response_vars.len(), 1);
}

#[gpui_kit::test]
fn undoing_to_the_saved_state_is_clean(cx: &mut TestAppContext) {
    let r = rig(cx, "a.yml");
    r.show(cx, Section::Headers);
    assert!(!r.view(cx, "a.yml").dirty);
    r.click(cx, "headers", ("kv-delete", 0usize));
    assert!(r.view(cx, "a.yml").dirty);
    r.undo(cx);
    assert!(!r.view(cx, "a.yml").dirty);
    r.redo(cx);
    assert!(r.view(cx, "a.yml").dirty);
}

#[gpui_kit::test]
fn undo_after_a_save_dirties_the_tab_again(cx: &mut TestAppContext) {
    let r = rig(cx, "a.yml");
    r.show(cx, Section::Headers);
    r.click(cx, "headers", ("kv-delete", 0usize));
    cx.update_window(r.win, |_, w, cx| r.ed.update(cx, |e, cx| e.save(w, cx)))
        .unwrap();
    cx.run_until_parked();
    assert!(!r.view(cx, "a.yml").dirty);
    r.undo(cx);
    let v = r.view(cx, "a.yml");
    assert!(v.dirty);
    assert_eq!(names(&v.headers), [on("H1"), on("H2")]);
}

#[gpui_kit::test]
fn an_outside_change_clears_the_history(cx: &mut TestAppContext) {
    let r = rig(cx, "a.yml");
    type_at_end(cx, r.win, Some("params"), ("kv-name", 1usize), "b");
    cx.run_until_parked();
    r.ws.update(cx, |w, cx| {
        w.send(&r.root, edit_url("a.yml", "http://x/9?z=1"), cx)
    });
    cx.run_until_parked();
    r.undo(cx);
    r.undo(cx);
    r.redo(cx);
    let v = r.view(cx, "a.yml");
    assert_eq!(v.url, "http://x/9?z=1");
    assert_eq!(v.params.len(), 1);
}

#[gpui_kit::test]
fn a_text_body_edit_is_not_recorded(cx: &mut TestAppContext) {
    let r = rig(cx, "j.yml");
    r.show(cx, Section::Body);
    replace_code(cx, r.win, Some("body"), "body-code", "[1]");
    cx.run_until_parked();
    let text = |r: &Rig, cx: &mut TestAppContext| match &r.view(cx, "j.yml").body {
        BodySpec::Json(t) => t.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(text(&r, cx), "[1]");
    r.undo(cx);
    assert_eq!(text(&r, cx), "[1]", "the editor keeps its own undo");
}

#[gpui_kit::test]
fn a_new_edit_clears_redo(cx: &mut TestAppContext) {
    let r = rig(cx, "a.yml");
    r.show(cx, Section::Headers);
    r.click(cx, "headers", ("kv-delete", 0usize));
    r.undo(cx);
    r.click(cx, "headers", ("kv-enabled", 1usize));
    r.redo(cx);
    assert_eq!(names(&r.view(cx, "a.yml").headers), [on("H1"), off("H2")]);
}

impl Rig {
    // Click without letting the actor answer, so the edit stays in flight.
    fn click_unparked(
        &self,
        cx: &mut TestAppContext,
        scope: &'static str,
        id: impl Into<gpui_kit::ElementId>,
    ) {
        cx.update_window(self.win, |_, w, cx| w.within(scope).click(id, cx))
            .unwrap();
    }

    fn undo_unparked(&self, cx: &mut TestAppContext) {
        cx.update_window(self.win, |_, w, cx| {
            self.ed.update(cx, |e, cx| e.undo_edit(w, cx))
        })
        .unwrap();
    }

    fn header_rows(&self, cx: &mut TestAppContext) -> Vec<apinata_app::kv_table::KvRow> {
        let table = self
            .ed
            .read_with(cx, |e, _| e.table(Section::Headers).cloned())
            .unwrap();
        table.read_with(cx, |t, cx| t.rows(cx))
    }

    fn send_header_rows(&self, cx: &mut TestAppContext, rows: Vec<apinata_app::kv_table::KvRow>) {
        cx.update_window(self.win, |_, w, cx| {
            self.ed
                .update(cx, |e, cx| e.edit_rows(Section::Headers, rows, w, cx))
        })
        .unwrap();
    }
}

// A header name with a line break is refused by core, so the edit fails.
fn bad_name(mut rows: Vec<apinata_app::kv_table::KvRow>) -> Vec<apinata_app::kv_table::KvRow> {
    rows[0].name = "bad\nname".into();
    rows
}

#[gpui_kit::test]
fn a_refused_edit_keeps_the_keystrokes_it_merged_into(cx: &mut TestAppContext) {
    let r = rig(cx, "a.yml");
    r.show(cx, Section::Headers);
    type_at_end(cx, r.win, Some("headers"), ("kv-name", 0usize), "x");
    cx.run_until_parked();
    assert_eq!(names(&r.view(cx, "a.yml").headers), [on("H1x"), on("H2")]);
    let rows = bad_name(r.header_rows(cx));
    r.send_header_rows(cx, rows);
    cx.run_until_parked();
    assert_eq!(names(&r.view(cx, "a.yml").headers), [on("H1x"), on("H2")]);
    r.undo(cx);
    assert_eq!(names(&r.view(cx, "a.yml").headers), [on("H1"), on("H2")]);
}

#[gpui_kit::test]
fn a_refused_edit_leaves_the_step_before_it(cx: &mut TestAppContext) {
    let r = rig(cx, "a.yml");
    r.show(cx, Section::Headers);
    r.click(cx, "headers", ("kv-enabled", 1usize));
    let rows = bad_name(r.header_rows(cx));
    r.send_header_rows(cx, rows);
    cx.run_until_parked();
    assert_eq!(names(&r.view(cx, "a.yml").headers), [on("H1"), off("H2")]);
    r.undo(cx);
    assert_eq!(names(&r.view(cx, "a.yml").headers), [on("H1"), on("H2")]);
}

#[gpui_kit::test]
fn a_no_op_edit_in_flight_keeps_the_history(cx: &mut TestAppContext) {
    let r = rig(cx, "a.yml");
    r.show(cx, Section::Headers);
    r.click_unparked(cx, "headers", ("kv-enabled", 0usize));
    let rows = r.header_rows(cx);
    r.send_header_rows(cx, rows);
    r.undo_unparked(cx);
    cx.run_until_parked();
    assert_eq!(names(&r.view(cx, "a.yml").headers), [on("H1"), on("H2")]);
}

#[gpui_kit::test]
fn two_edits_in_flight_undo_back_to_the_right_values(cx: &mut TestAppContext) {
    let r = rig(cx, "a.yml");
    r.show(cx, Section::Headers);
    r.click_unparked(cx, "headers", ("kv-enabled", 0usize));
    r.click_unparked(cx, "headers", ("kv-enabled", 1usize));
    r.undo_unparked(cx);
    cx.run_until_parked();
    assert_eq!(names(&r.view(cx, "a.yml").headers), [off("H1"), on("H2")]);
    r.undo(cx);
    assert_eq!(names(&r.view(cx, "a.yml").headers), [on("H1"), on("H2")]);
}

#[gpui_kit::test]
fn a_params_undo_refused_by_the_url_lock_keeps_the_history(cx: &mut TestAppContext) {
    let r = rig(cx, "a.yml");
    type_at_end(cx, r.win, Some("params"), ("kv-name", 1usize), "b");
    cx.run_until_parked();
    // The URL edit changes nothing, but it is in flight and locks the params side.
    cx.update_window(r.win, |_, w, cx| {
        r.ed.update(cx, |e, cx| e.edit_url("http://x/0?a=1&b=", w, cx))
    })
    .unwrap();
    r.undo_unparked(cx);
    cx.run_until_parked();
    assert_eq!(
        r.view(cx, "a.yml").params.len(),
        2,
        "the refused undo sent nothing"
    );
    r.undo(cx);
    assert_eq!(r.view(cx, "a.yml").params.len(), 1);
}

#[gpui_kit::test]
fn toggle_all_back_to_the_saved_state_is_clean(cx: &mut TestAppContext) {
    let r = rig(cx, "a.yml");
    r.show(cx, Section::Headers);
    r.click(cx, "headers", "kv-head-enabled");
    assert!(r.view(cx, "a.yml").dirty);
    r.undo(cx);
    assert!(!r.view(cx, "a.yml").dirty);
}

#[gpui_kit::test]
fn an_undo_that_finds_stale_history_leaves_no_url_lock(cx: &mut TestAppContext) {
    let r = rig(cx, "a.yml");
    type_at_end(cx, r.win, Some("params"), ("kv-name", 1usize), "b");
    cx.run_until_parked();
    // The URL side edits last, which makes the params step stale.
    cx.update_window(r.win, |_, w, cx| {
        r.ed.update(cx, |e, cx| e.edit_url("http://x/0?a=1", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    r.undo(cx);
    r.ws.update(cx, |w, cx| {
        w.send(&r.root, edit_url("a.yml", "http://x/8?z=1"), cx)
    });
    let locked =
        r.ed.read_with(cx, |e, cx| e.locked(apinata_app::url_unit::Side::Url, cx));
    assert!(!locked, "the undo sent nothing from the params side");
}

#[gpui_kit::test]
fn a_url_edit_and_its_undo_keep_a_header_step(cx: &mut TestAppContext) {
    let r = rig(cx, "a.yml");
    r.show(cx, Section::Headers);
    r.click(cx, "headers", ("kv-enabled", 0usize));
    for url in ["http://x/0?a=1x", "http://x/0?a=1"] {
        cx.update_window(r.win, |_, w, cx| {
            r.ed.update(cx, |e, cx| e.edit_url(url, w, cx))
        })
        .unwrap();
        cx.run_until_parked();
    }
    r.undo(cx);
    assert_eq!(names(&r.view(cx, "a.yml").headers), [on("H1"), on("H2")]);
}

#[gpui_kit::test]
fn a_url_edit_that_rewrites_params_retires_only_the_params_steps(cx: &mut TestAppContext) {
    let r = rig(cx, "a.yml");
    type_at_end(cx, r.win, Some("params"), ("kv-name", 1usize), "b");
    cx.run_until_parked();
    r.show(cx, Section::Headers);
    r.click(cx, "headers", ("kv-enabled", 0usize));
    cx.update_window(r.win, |_, w, cx| {
        r.ed.update(cx, |e, cx| e.edit_url("http://x/0?a=1&c=3", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    // The header step is on top and unaffected; the params step below it went stale.
    r.undo(cx);
    assert_eq!(names(&r.view(cx, "a.yml").headers), [on("H1"), on("H2")]);
    r.undo(cx);
    assert_eq!(r.view(cx, "a.yml").url, "http://x/0?a=1&c=3");
}

#[gpui_kit::test]
fn a_method_change_is_undone_and_redone(cx: &mut TestAppContext) {
    let r = rig(cx, "a.yml");
    assert!(!r.view(cx, "a.yml").dirty);
    cx.update_window(r.win, |_, w, cx| {
        r.ed.update(cx, |e, cx| e.set_method("PUT", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(r.view(cx, "a.yml").method, "PUT");
    assert!(r.view(cx, "a.yml").dirty);
    r.undo(cx);
    assert_eq!(r.view(cx, "a.yml").method, "POST");
    assert!(!r.view(cx, "a.yml").dirty);
    r.redo(cx);
    assert_eq!(r.view(cx, "a.yml").method, "PUT");
}

fn switch_mode(r: &Rig, cx: &mut TestAppContext, mode: apinata_app::body_section::BodyMode) {
    let body = r.ed.read_with(cx, |e, _| e.body().clone());
    cx.update_window(r.win, |_, w, cx| {
        body.update(cx, |s, cx| s.set_mode(mode, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_body_mode_switch_is_undone_and_redone(cx: &mut TestAppContext) {
    use apinata_app::body_section::BodyMode;
    let r = rig(cx, "j.yml");
    r.show(cx, Section::Body);
    switch_mode(&r, cx, BodyMode::Form);
    assert!(matches!(
        r.view(cx, "j.yml").body,
        BodySpec::FormUrlEncoded(_)
    ));
    assert!(r.view(cx, "j.yml").dirty);
    r.undo(cx);
    assert!(matches!(&r.view(cx, "j.yml").body, BodySpec::Json(t) if t == "{}"));
    assert!(!r.view(cx, "j.yml").dirty);
    let body = r.ed.read_with(cx, |e, _| e.body().clone());
    assert_eq!(body.read_with(cx, |s, _| s.mode()), Some(BodyMode::Json));
    r.redo(cx);
    assert!(matches!(
        r.view(cx, "j.yml").body,
        BodySpec::FormUrlEncoded(_)
    ));
    assert_eq!(body.read_with(cx, |s, _| s.mode()), Some(BodyMode::Form));
    switch_mode(&r, cx, BodyMode::None);
    r.undo(cx);
    assert!(matches!(
        r.view(cx, "j.yml").body,
        BodySpec::FormUrlEncoded(_)
    ));
}

fn token_text(r: &Rig, cx: &mut TestAppContext) -> String {
    use apinata_app::auth_section::AuthField;
    let token = r.ed.read_with(cx, |e, cx| {
        e.auth().read(cx).input(AuthField::Token).clone()
    });
    token.read_with(cx, |i, _| i.value().to_string())
}

fn switch_auth(r: &Rig, cx: &mut TestAppContext, mode: apinata_app::auth_section::AuthMode) {
    let auth = r.ed.read_with(cx, |e, _| e.auth().clone());
    cx.update_window(r.win, |_, w, cx| {
        auth.update(cx, |s, cx| s.set_mode(mode, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn an_auth_mode_switch_is_undone_and_redone(cx: &mut TestAppContext) {
    use apinata_app::auth_section::AuthMode;
    use apinata_core::AuthSpec;
    let r = rig(cx, "b.yml");
    r.show(cx, Section::Auth);
    switch_auth(&r, cx, AuthMode::Basic);
    assert!(matches!(r.view(cx, "b.yml").auth, AuthSpec::Basic { .. }));
    assert!(r.view(cx, "b.yml").dirty);
    r.undo(cx);
    assert_eq!(
        r.view(cx, "b.yml").auth,
        AuthSpec::Bearer { token: "t".into() }
    );
    assert!(!r.view(cx, "b.yml").dirty);
    let auth = r.ed.read_with(cx, |e, _| e.auth().clone());
    assert_eq!(auth.read_with(cx, |s, _| s.mode()), Some(AuthMode::Bearer));
    assert_eq!(token_text(&r, cx), "t");
    r.redo(cx);
    assert!(matches!(r.view(cx, "b.yml").auth, AuthSpec::Basic { .. }));
    assert_eq!(auth.read_with(cx, |s, _| s.mode()), Some(AuthMode::Basic));
}

#[gpui_kit::test]
fn an_auth_field_edit_is_one_step_and_refills_the_mode_memory(cx: &mut TestAppContext) {
    use apinata_app::auth_section::AuthMode;
    use apinata_core::AuthSpec;
    let r = rig(cx, "b.yml");
    r.show(cx, Section::Auth);
    type_at_end(cx, r.win, Some("auth"), "auth-token", "234");
    cx.run_until_parked();
    assert_eq!(
        r.view(cx, "b.yml").auth,
        AuthSpec::Bearer {
            token: "t234".into()
        }
    );
    r.undo(cx);
    assert_eq!(
        r.view(cx, "b.yml").auth,
        AuthSpec::Bearer { token: "t".into() }
    );
    assert!(
        !r.view(cx, "b.yml").dirty,
        "the run of keystrokes was one step"
    );
    assert_eq!(token_text(&r, cx), "t");
    // Leaving and returning to the mode sends the remembered value, which undo restored.
    switch_auth(&r, cx, AuthMode::None);
    switch_auth(&r, cx, AuthMode::Bearer);
    assert_eq!(
        r.view(cx, "b.yml").auth,
        AuthSpec::Bearer { token: "t".into() }
    );
    r.undo(cx);
    r.undo(cx);
    r.redo(cx);
    r.redo(cx);
    assert_eq!(
        r.view(cx, "b.yml").auth,
        AuthSpec::Bearer { token: "t".into() }
    );
    r.undo(cx);
    r.redo(cx);
    assert_eq!(
        r.view(cx, "b.yml").auth,
        AuthSpec::Bearer { token: "t".into() }
    );
}

fn access_url(r: &Rig, cx: &mut TestAppContext) -> String {
    match &r.view(cx, "o.yml").auth {
        apinata_core::AuthSpec::OAuth2(o) => o.access_token_url.clone(),
        other => panic!("{other:?}"),
    }
}

// Types a client secret without blurring it, then sends the edit a save would owe. The
// edit is left in flight, which is when a recorded step would still be trusted.
fn commit_typed_secret(r: &Rig, cx: &mut TestAppContext) {
    use apinata_app::oauth2_editor::OAuth2Field;
    cx.update_window(r.win, |_, w, _| w.activate_window())
        .unwrap();
    type_at_end(
        cx,
        r.win,
        Some("auth"),
        OAuth2Field::ClientSecret.id(),
        "s3cret",
    );
    cx.update_window(r.win, |_, _, cx| {
        r.ed.update(cx, |e, cx| e.commit_pending_secret(cx))
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_pending_oauth2_secret_is_not_an_undo_step(cx: &mut TestAppContext) {
    use apinata_app::oauth2_editor::OAuth2Field;
    let r = rig(cx, "o.yml");
    r.show(cx, Section::Auth);
    type_at_end(
        cx,
        r.win,
        Some("auth"),
        OAuth2Field::AccessTokenUrl.id(),
        "/v2",
    );
    cx.run_until_parked();
    assert_eq!(access_url(&r, cx), "http://id/token/v2");
    commit_typed_secret(&r, cx);
    // The move is not a step, so this undo has nothing to take back; a recorded one would
    // rewrite the auth from before the move and bring the literal secret's absence back.
    r.undo(cx);
    assert_eq!(access_url(&r, cx), "http://id/token/v2");
    match &r.view(cx, "o.yml").auth {
        apinata_core::AuthSpec::OAuth2(o) => {
            assert_eq!(
                o.client_secret,
                "{{process.env.APINATA_OAUTH2_CLIENT_SECRET}}"
            )
        }
        other => panic!("{other:?}"),
    }
}

#[gpui_kit::test]
fn an_auth_mode_switch_never_joins_a_typing_run(cx: &mut TestAppContext) {
    use apinata_app::auth_section::AuthMode;
    use apinata_core::AuthSpec;
    let r = rig(cx, "b.yml");
    r.show(cx, Section::Auth);
    type_at_end(cx, r.win, Some("auth"), "auth-token", "1");
    cx.run_until_parked();
    switch_auth(&r, cx, AuthMode::Basic);
    assert!(matches!(r.view(cx, "b.yml").auth, AuthSpec::Basic { .. }));
    // One undo takes back only the switch, leaving the typed token in place.
    r.undo(cx);
    assert_eq!(
        r.view(cx, "b.yml").auth,
        AuthSpec::Bearer { token: "t1".into() }
    );
}

// A row typed into the empty row, undone and redone by keys alone: the undo drops the row that
// held focus, so focus must stay in the table for the redo key to reach the editor.
fn new_row_redone_by_keys(cx: &mut TestAppContext, section: Section, scope: &'static str) {
    let r = rig(cx, "a.yml");
    r.show(cx, section);
    let before = r.rows(cx, section);
    type_at_end(cx, r.win, Some(scope), ("kv-name", before.len()), "zz");
    cx.run_until_parked();
    assert_eq!(r.rows(cx, section).len(), before.len() + 1);
    cx.simulate_keystrokes(r.win, &secondary("z"));
    cx.run_until_parked();
    assert_eq!(r.rows(cx, section), before);
    assert!(
        focus_within(cx, r.win, Some(scope)),
        "focus stays in {scope} after the undo"
    );
    cx.simulate_keystrokes(r.win, &secondary("shift-z"));
    cx.run_until_parked();
    let after = r.rows(cx, section);
    assert_eq!(
        after.len(),
        before.len() + 1,
        "the redo key brings the {scope} row back"
    );
    assert_eq!(after.last().unwrap().0, "zz");
}

#[gpui_kit::test]
fn a_new_header_row_is_redone_by_keys_after_its_undo(cx: &mut TestAppContext) {
    new_row_redone_by_keys(cx, Section::Headers, "headers");
}

#[gpui_kit::test]
fn a_new_param_row_is_redone_by_keys_after_its_undo(cx: &mut TestAppContext) {
    new_row_redone_by_keys(cx, Section::Params, "params");
}

// The same, with focus Tabbed onto the new row's Remove button before the undo drops the row:
// focus anywhere in the dropped row moves to a cell, so the redo key still reaches the editor.
#[gpui_kit::test]
fn a_new_header_row_is_redone_by_keys_after_its_undo_from_remove(cx: &mut TestAppContext) {
    let r = rig(cx, "a.yml");
    r.show(cx, Section::Headers);
    let before = r.rows(cx, Section::Headers);
    let ix = before.len();
    type_at_end(cx, r.win, Some("headers"), ("kv-name", ix), "zz");
    cx.run_until_parked();
    assert_eq!(r.rows(cx, Section::Headers).len(), ix + 1);
    tab_within_to(cx, r.win, "headers", ("kv-delete", ix));
    cx.simulate_keystrokes(r.win, &secondary("z"));
    cx.run_until_parked();
    assert_eq!(r.rows(cx, Section::Headers), before);
    assert!(
        focus_within(cx, r.win, Some("headers")),
        "focus moves into a cell after the undo"
    );
    cx.simulate_keystrokes(r.win, &secondary("shift-z"));
    cx.run_until_parked();
    let after = r.rows(cx, Section::Headers);
    assert_eq!(after.len(), ix + 1, "the redo key brings the row back");
    assert_eq!(after.last().unwrap().0, "zz");
}

// A redone delete drops a middle row while its Remove button holds focus. Focus goes where a
// delete sends it, the name of the row that took the slot, and the undo key, with no click,
// brings the row back.
#[gpui_kit::test]
fn a_redone_middle_header_delete_from_its_remove_focuses_the_next_rows_name(
    cx: &mut TestAppContext,
) {
    let r = rig(cx, "a.yml");
    r.show(cx, Section::Headers);
    type_at_end(cx, r.win, Some("headers"), ("kv-name", 2usize), "H3");
    cx.run_until_parked();
    let three = r.rows(cx, Section::Headers);
    assert_eq!(three.len(), 3);
    let dropped = vec![three[0].clone(), three[2].clone()];
    let remove_focused = |cx: &mut TestAppContext| {
        cx.update_window(r.win, |_, w, cx| {
            w.render_frame(cx);
            w.within("headers").find(("kv-delete", 1usize)).focused()
        })
        .unwrap()
    };
    tab_within_to(cx, r.win, "headers", ("kv-delete", 1usize));
    cx.update_window(r.win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        r.rows(cx, Section::Headers),
        dropped,
        "Remove drops the middle row"
    );
    cx.simulate_keystrokes(r.win, &secondary("z"));
    cx.run_until_parked();
    assert_eq!(
        r.rows(cx, Section::Headers),
        three,
        "the undo brings it back"
    );
    tab_within_to(cx, r.win, "headers", ("kv-delete", 1usize));
    assert_eq!(remove_focused(cx), Some(true));
    cx.simulate_keystrokes(r.win, &secondary("shift-z"));
    cx.run_until_parked();
    assert_eq!(
        r.rows(cx, Section::Headers),
        dropped,
        "the redo drops it again"
    );
    let name_focused = cx
        .update_window(r.win, |_, w, cx| {
            w.render_frame(cx);
            w.within("headers").find(("kv-name", 1usize)).focused()
        })
        .unwrap();
    assert_eq!(
        name_focused,
        Some(true),
        "focus moves to the name of the row now in the slot"
    );
    cx.simulate_keystrokes(r.win, &secondary("z"));
    cx.run_until_parked();
    assert_eq!(
        r.rows(cx, Section::Headers),
        three,
        "the undo key still reaches the editor"
    );
}

/// Three header rows, the middle one removed and the removal undone, so a redo drops it again.
fn three_headers_with_a_redoable_middle_delete(
    cx: &mut TestAppContext,
) -> (Rig, Vec<(String, String, bool)>) {
    let r = rig(cx, "a.yml");
    r.show(cx, Section::Headers);
    type_at_end(cx, r.win, Some("headers"), ("kv-name", 2usize), "H3");
    cx.run_until_parked();
    let three = r.rows(cx, Section::Headers);
    assert_eq!(three.len(), 3);
    r.click(cx, "headers", ("kv-delete", 1usize));
    r.undo(cx);
    assert_eq!(
        r.rows(cx, Section::Headers),
        three,
        "the undo brings it back"
    );
    (r, vec![three[0].clone(), three[2].clone()])
}

fn header_focused(cx: &mut TestAppContext, r: &Rig, id: (&'static str, usize)) -> Option<bool> {
    cx.update_window(r.win, |_, w, cx| {
        w.render_frame(cx);
        w.within("headers").find(id).focused()
    })
    .unwrap()
}

// A redone delete of the middle row while a later row's Remove holds focus: that row survives,
// one slot up, and its Remove keeps focus there.
#[gpui_kit::test]
fn a_redone_header_delete_keeps_focus_on_a_later_surviving_rows_remove(cx: &mut TestAppContext) {
    let (r, dropped) = three_headers_with_a_redoable_middle_delete(cx);
    tab_within_to(cx, r.win, "headers", ("kv-delete", 2usize));
    r.redo(cx);
    assert_eq!(
        r.rows(cx, Section::Headers),
        dropped,
        "the redo drops it again"
    );
    assert_eq!(header_focused(cx, &r, ("kv-delete", 1usize)), Some(true));
}

// The same with the first row's Remove focused: nothing above it moved, so focus stays put.
#[gpui_kit::test]
fn a_redone_header_delete_keeps_focus_on_an_earlier_surviving_rows_remove(cx: &mut TestAppContext) {
    let (r, dropped) = three_headers_with_a_redoable_middle_delete(cx);
    tab_within_to(cx, r.win, "headers", ("kv-delete", 0usize));
    r.redo(cx);
    assert_eq!(
        r.rows(cx, Section::Headers),
        dropped,
        "the redo drops it again"
    );
    assert_eq!(header_focused(cx, &r, ("kv-delete", 0usize)), Some(true));
}

const THREE: &str = "info:\n  name: h\n  type: http\n  seq: 6\nhttp:\n  method: GET\n  url: http://x\n  headers:\n    - name: h1\n      value: one\n    - name: h2\n      value: two\n    - name: h3\n      value: three\n";

/// A saved request with headers h1, h2 and h3, its Headers shown, and h3's Remove focused the
/// way a user reaches it: a click in h3's Description, then Tab.
fn three_saved_headers_with_the_last_remove_focused(cx: &mut TestAppContext) -> Rig {
    support::init(cx);
    let col = write_tree(&[("opencollection.yml", ROOT), ("h.yml", THREE)]);
    let (state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "h.yml", false);
    let r = Rig {
        _col: col,
        _state: state,
        ws,
        root,
        win,
        ed,
    };
    r.show(cx, Section::Headers);
    focus_the_last_remove(cx, &r);
    r
}

fn focus_the_last_remove(cx: &mut TestAppContext, r: &Rig) {
    r.click(cx, "headers", ("kv-description", 2usize));
    press_key(cx, r.win, "tab");
    assert_eq!(header_focused(cx, r, ("kv-delete", 2usize)), Some(true));
}

/// Notifies the workspace once more in the same update as the first snapshot with two
/// headers, as the app's later notices do before its next frame; tests draw after every
/// update, the app only once per frame.
fn renotify_when_two_headers_land(cx: &mut TestAppContext, r: &Rig) -> gpui_kit::Subscription {
    let (root, item) = (r.root.clone(), ip("h.yml"));
    let armed = std::cell::Cell::new(true);
    cx.update(|cx| {
        cx.observe(&r.ws, move |ws, cx| {
            let two = ws
                .read(cx)
                .request(&root, &item, cx)
                .map(|v| v.headers.len())
                == Some(2);
            if two && armed.replace(false) {
                ws.update(cx, |_, cx| cx.notify());
            }
        })
    })
}

fn header_names(cx: &mut TestAppContext, r: &Rig) -> Vec<String> {
    r.rows(cx, Section::Headers)
        .into_iter()
        .map(|h| h.0)
        .collect()
}

// h1 is deleted on disk while h3's Remove holds focus: h3 survives two slots up, its Remove
// keeps focus there, and Space on it removes h3.
#[gpui_kit::test]
fn an_outside_header_delete_keeps_focus_on_a_surviving_rows_remove(cx: &mut TestAppContext) {
    let r = three_saved_headers_with_the_last_remove_focused(cx);
    let _renotify = renotify_when_two_headers_land(cx, &r);
    let file = r.root.join("h.yml");
    std::fs::write(
        &file,
        THREE.replace("    - name: h1\n      value: one\n", ""),
    )
    .unwrap();
    let root = r.root.clone();
    let reload = apinata_app::command::Command::Reload(vec![file]);
    r.ws.update(cx, |w, cx| w.send(&root, reload, cx));
    cx.run_until_parked();
    assert_eq!(header_names(cx, &r), ["h2", "h3"]);
    assert_eq!(header_focused(cx, &r, ("kv-delete", 1usize)), Some(true));
    press_key(cx, r.win, "space");
    assert_eq!(
        header_names(cx, &r),
        ["h2"],
        "Space on the focused Remove removes h3"
    );
}

// A delete of h1 undone, then redone by key from h3's Remove: h3 keeps the focus.
#[gpui_kit::test]
fn a_redone_header_delete_by_key_keeps_focus_on_a_surviving_rows_remove(cx: &mut TestAppContext) {
    let r = three_saved_headers_with_the_last_remove_focused(cx);
    r.click(cx, "headers", ("kv-delete", 0usize));
    press_key(cx, r.win, &secondary("z"));
    assert_eq!(
        header_names(cx, &r),
        ["h1", "h2", "h3"],
        "the undo brings h1 back"
    );
    focus_the_last_remove(cx, &r);
    let _renotify = renotify_when_two_headers_land(cx, &r);
    press_key(cx, r.win, &secondary("shift-z"));
    assert_eq!(header_names(cx, &r), ["h2", "h3"], "the redo drops h1");
    assert_eq!(header_focused(cx, &r, ("kv-delete", 1usize)), Some(true));
    press_key(cx, r.win, "space");
    assert_eq!(
        header_names(cx, &r),
        ["h2"],
        "Space on the focused Remove removes h3"
    );
}

/// The THREE request opened in a workspace with the real watcher, its Headers shown.
fn watched_three(cx: &mut TestAppContext) -> Rig {
    support::init(cx);
    let col = write_tree(&[("opencollection.yml", ROOT), ("h.yml", THREE)]);
    let w = watched(cx, &col);
    let (win, ed) = editor_for(cx, &w.ws, &w.root, "h.yml", false);
    let r = Rig {
        _col: col,
        _state: w.state,
        ws: w.ws,
        root: w.root,
        win,
        ed,
    };
    r.show(cx, Section::Headers);
    r
}

impl Rig {
    fn save_now(&self, cx: &mut TestAppContext) {
        cx.update_window(self.win, |_, w, cx| {
            self.ed.update(cx, |e, cx| e.save(w, cx))
        })
        .unwrap();
        cx.run_until_parked();
        assert!(!self.view(cx, "h.yml").dirty, "the save lands");
    }

    /// Writes `text` to h.yml behind the app's back and waits until the Headers rows show
    /// `(name, value)`, then draws a frame.
    fn outside_write(&self, cx: &mut TestAppContext, text: &str, name: &str, value: &str) {
        settle(cx);
        std::fs::write(self.root.join("h.yml"), text).unwrap();
        wait_for(cx, "the reload to show the outside change", |cx| {
            self.rows(cx, Section::Headers)
                .iter()
                .any(|(n, v, _)| n == name && v == value)
        });
        frame(cx, self.win);
    }

    fn header_pairs(&self, cx: &mut TestAppContext) -> Vec<(String, String)> {
        self.rows(cx, Section::Headers)
            .into_iter()
            .filter(|(n, _, _)| !n.is_empty())
            .map(|(n, v, _)| (n, v))
            .collect()
    }
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(n, v)| (n.to_string(), v.to_string()))
        .collect()
}

// The issue's scenario: a step from before a disk reload must not let a later undo write the
// pre-reload headers back over the outside change.
#[gpui_kit::test]
fn undo_after_a_disk_reload_keeps_the_outside_change(cx: &mut TestAppContext) {
    let r = watched_three(cx);
    r.click(cx, "headers", ("kv-delete", 1usize));
    r.save_now(cx);
    let outside = "info:\n  name: h\n  type: http\n  seq: 6\nhttp:\n  method: GET\n  url: http://x\n  headers:\n    - name: h1\n      value: one\n    - name: h3\n      value: three-x\n";
    r.outside_write(cx, outside, "h3", "three-x");
    r.click(cx, "headers", ("kv-delete", 0usize));
    assert_eq!(r.header_pairs(cx), pairs(&[("h3", "three-x")]));
    press_key(cx, r.win, &secondary("z"));
    assert_eq!(
        r.header_pairs(cx),
        pairs(&[("h1", "one"), ("h3", "three-x")]),
        "the undo brings h1 back and keeps h3's outside value"
    );
    r.save_now(cx);
    let disk = std::fs::read_to_string(r.root.join("h.yml")).unwrap();
    assert!(disk.contains("three-x"), "{disk}");
    assert!(
        disk.contains("name: h1") && !disk.contains("name: h2"),
        "{disk}"
    );
}

// A keystroke after a reload of its field starts a new typing run: undo takes back only the
// keystrokes typed after the reload.
#[gpui_kit::test]
fn typing_after_a_disk_reload_does_not_merge_into_the_earlier_run(cx: &mut TestAppContext) {
    let r = watched_three(cx);
    type_at_end(cx, r.win, Some("headers"), ("kv-value", 2usize), "a");
    cx.run_until_parked();
    r.save_now(cx);
    let outside = THREE
        .replace("value: one", "value: one-x")
        .replace("value: three", "value: threea");
    r.outside_write(cx, &outside, "h1", "one-x");
    type_at_end(cx, r.win, Some("headers"), ("kv-value", 2usize), "b");
    cx.run_until_parked();
    assert_eq!(r.header_pairs(cx)[2], ("h3".into(), "threeab".into()));
    press_key(cx, r.win, &secondary("z"));
    assert_eq!(
        r.header_pairs(cx),
        pairs(&[("h1", "one-x"), ("h2", "two"), ("h3", "threea")]),
        "only the keystroke after the reload is undone"
    );
}

// A step on a field the reload left alone still undoes, and writes only its own field, even
// after a new step on the reloaded field.
#[gpui_kit::test]
fn a_step_on_a_field_the_reload_left_alone_still_undoes(cx: &mut TestAppContext) {
    let r = watched_three(cx);
    cx.update_window(r.win, |_, w, cx| {
        r.ed.update(cx, |e, cx| e.set_method("PUT", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    r.save_now(cx);
    let outside = THREE
        .replace("method: GET", "method: PUT")
        .replace("value: three", "value: three-x");
    r.outside_write(cx, &outside, "h3", "three-x");
    r.click(cx, "headers", ("kv-delete", 0usize));
    press_key(cx, r.win, &secondary("z"));
    press_key(cx, r.win, &secondary("z"));
    let v = r.view(cx, "h.yml");
    assert_eq!(v.method, "GET", "the method step still undoes");
    assert_eq!(
        r.header_pairs(cx),
        pairs(&[("h1", "one"), ("h2", "two"), ("h3", "three-x")]),
        "the headers keep the outside change"
    );
}

/// Runs at each workspace notify whose snapshot shows h3's outside value, given how many such
/// notifies came before it; one registered before the editor runs ahead of its refresh.
type Hook = std::rc::Rc<std::cell::RefCell<Box<dyn FnMut(&mut gpui_kit::App, usize)>>>;

fn hook(f: impl FnMut(&mut gpui_kit::App, usize) + 'static) -> Hook {
    std::rc::Rc::new(std::cell::RefCell::new(Box::new(f)))
}

fn idle() -> Hook {
    hook(|_, _| {})
}

fn on_outside_h3(
    cx: &mut TestAppContext,
    ws: &Entity<apinata_app::workspace::Workspace>,
    root: &std::path::Path,
    hook: Hook,
) -> gpui_kit::Subscription {
    let (root, item) = (root.to_path_buf(), ip("h.yml"));
    let count = std::cell::Cell::new(0);
    cx.update(|cx| {
        cx.observe(ws, move |ws, cx| {
            let landed = ws
                .read(cx)
                .request(&root, &item, cx)
                .is_some_and(|v| v.headers.iter().any(|h| h.value == "three-x"));
            if landed {
                count.set(count.get() + 1);
                (hook.borrow_mut())(cx, count.get());
            }
        })
    })
}

/// The THREE request with its Headers shown, `before` observing the workspace ahead of the
/// editor and `after` behind it.
fn race_rig(
    cx: &mut TestAppContext,
    before: Hook,
    after: Hook,
) -> (Rig, [gpui_kit::Subscription; 2]) {
    support::init(cx);
    let col = write_tree(&[("opencollection.yml", ROOT), ("h.yml", THREE)]);
    let (state, ws, root) = open_dir(cx, &col);
    let first = on_outside_h3(cx, &ws, &root, before);
    let (win, ed) = editor_for(cx, &ws, &root, "h.yml", false);
    let second = on_outside_h3(cx, &ws, &root, after);
    let r = Rig {
        _col: col,
        _state: state,
        ws,
        root,
        win,
        ed,
    };
    r.show(cx, Section::Headers);
    (r, [first, second])
}

/// Turns h1 off in the Headers table's rows as they stand and emits them, as a click would.
fn table_turns_h1_off(cx: &mut gpui_kit::App, ed: &Entity<RequestEditor>) {
    let table = ed.read(cx).table(Section::Headers).cloned().unwrap();
    let mut rows = table.read(cx).rows(cx);
    rows[0].enabled = false;
    table.update(cx, |_, cx| {
        cx.emit(apinata_app::kv_table::KvEvent::Changed(rows))
    });
}

/// Turns h2 off in the snapshot's own headers, an edit built from the reloaded content.
fn view_turns_h2_off(
    cx: &mut gpui_kit::App,
    ws: &Entity<apinata_app::workspace::Workspace>,
    root: &std::path::Path,
) {
    use apinata_app::command::{Command, Edit, Target};
    let mut headers = ws
        .read(cx)
        .request(root, &ip("h.yml"), cx)
        .unwrap()
        .headers
        .clone();
    headers[1].enabled = false;
    let edit = Command::Edit {
        target: Target::Request(ip("h.yml")),
        edit: Edit::Headers(headers),
    };
    ws.update(cx, |w, cx| w.send(root, edit, cx));
}

fn renotify(cx: &mut gpui_kit::App, ws: &Entity<apinata_app::workspace::Workspace>) {
    ws.update(cx, |_, cx| cx.notify());
}

impl Rig {
    /// Writes h3's outside value to disk and reloads the file, as the watcher does.
    fn reload_outside_h3(&self, cx: &mut TestAppContext) {
        let file = self.root.join("h.yml");
        std::fs::write(&file, THREE.replace("value: three", "value: three-x")).unwrap();
        let reload = apinata_app::command::Command::Reload(vec![file]);
        let root = self.root.clone();
        self.ws.update(cx, |w, cx| w.send(&root, reload, cx));
        cx.run_until_parked();
        frame(cx, self.win);
        cx.run_until_parked();
    }

    /// After everything settled: h3 keeps its outside value, the user's h1 edit either landed
    /// on top of it or is held back as stale, and a save leaves the outside value on disk.
    fn assert_outside_h3_kept(&self, cx: &mut TestAppContext) {
        let v = self.view(cx, "h.yml");
        let h3 = v.headers.iter().find(|h| h.name == "h3").unwrap();
        assert_eq!(
            h3.value, "three-x",
            "the outside change survives: {:?}",
            v.headers
        );
        let held = ws_snap(cx, &self.ws, &self.root)
            .stale
            .contains_key(&apinata_app::command::Target::Request(ip("h.yml")));
        assert!(
            held || !v.headers[0].enabled,
            "the h1 edit landed or is held"
        );
        if v.dirty {
            self.save_now(cx);
        }
        let disk = std::fs::read_to_string(self.root.join("h.yml")).unwrap();
        assert!(disk.contains("three-x"), "{disk}");
    }
}

// Window one: the table emits an edit in the same update the reload lands in, before the
// editor's refresh has loaded the reloaded rows.
#[gpui_kit::test]
fn a_table_edit_in_the_update_a_reload_lands_in_keeps_the_outside_change(cx: &mut TestAppContext) {
    let slot: std::rc::Rc<std::cell::RefCell<Option<Rig2>>> = Default::default();
    let s = slot.clone();
    let before = hook(move |cx, n| {
        if n == 1 {
            let (_, ed, _) = s.borrow().clone().unwrap();
            table_turns_h1_off(cx, &ed);
        }
    });
    let (r, _subs) = race_rig(cx, before, idle());
    *slot.borrow_mut() = Some((r.win, r.ed.clone(), r.ws.clone()));
    r.reload_outside_h3(cx);
    r.assert_outside_h3_kept(cx);
}

// As above, with a second load of the editor before the next draw.
#[gpui_kit::test]
fn a_table_edit_in_the_update_a_reload_lands_in_keeps_it_across_two_loads(cx: &mut TestAppContext) {
    let slot: std::rc::Rc<std::cell::RefCell<Option<Rig2>>> = Default::default();
    let (s1, s2) = (slot.clone(), slot.clone());
    let before = hook(move |cx, n| {
        if n == 1 {
            let (_, ed, _) = s1.borrow().clone().unwrap();
            table_turns_h1_off(cx, &ed);
        }
    });
    let after = hook(move |cx, n| {
        if n == 1 {
            let (_, _, ws) = s2.borrow().clone().unwrap();
            renotify(cx, &ws);
        }
    });
    let (r, _subs) = race_rig(cx, before, after);
    *slot.borrow_mut() = Some((r.win, r.ed.clone(), r.ws.clone()));
    r.reload_outside_h3(cx);
    r.assert_outside_h3_kept(cx);
}

type Rig2 = (
    AnyWindowHandle,
    Entity<RequestEditor>,
    Entity<apinata_app::workspace::Workspace>,
);

// Window two: the reload lands while Headers has an edit in flight, so the refresh leaves the
// table on its pre-reload rows; the table's next edit is built from them.
fn busy_field_race(cx: &mut TestAppContext, loads: usize) {
    let slot: std::rc::Rc<std::cell::RefCell<Option<(Rig2, std::path::PathBuf)>>> =
        Default::default();
    let (s1, s2) = (slot.clone(), slot.clone());
    let before = hook(move |cx, n| {
        if n == 1 {
            let ((_, _, ws), root) = s1.borrow().clone().unwrap();
            view_turns_h2_off(cx, &ws, &root);
        }
    });
    let after = hook(move |cx, n| {
        let ((_, ed, ws), _) = s2.borrow().clone().unwrap();
        if n < loads {
            renotify(cx, &ws);
        } else if n == loads {
            table_turns_h1_off(cx, &ed);
        }
    });
    let (r, _subs) = race_rig(cx, before, after);
    *slot.borrow_mut() = Some(((r.win, r.ed.clone(), r.ws.clone()), r.root.clone()));
    r.reload_outside_h3(cx);
    r.assert_outside_h3_kept(cx);
}

#[gpui_kit::test]
fn a_table_edit_while_its_field_is_busy_through_a_reload_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    busy_field_race(cx, 1);
}

#[gpui_kit::test]
fn a_table_edit_while_its_field_is_busy_through_two_loads_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    busy_field_race(cx, 2);
}

const TYPED: &str = "info:\n  name: h\n  type: http\n  seq: 6\nhttp:\n  method: GET\n  url: http://x\nruntime:\n  variables:\n    - name: v1\n      value: one\n    - name: v3\n      value:\n        type: number\n        data: \"1\"\n";

fn v3_is_boolean(v: &apinata_app::snapshot::RequestView) -> bool {
    v.vars.iter().any(
        |v| matches!(&v.value, apinata_core::VarValue::Typed { kind, .. } if kind == "boolean"),
    )
}

/// Runs once against the editor and its window at the first workspace notify where the
/// reload has landed.
type OnEditor = dyn Fn(&mut gpui_kit::App, &Entity<RequestEditor>, AnyWindowHandle);

type Rows = Vec<apinata_app::kv_table::KvRow>;

/// The table under test in a shown section.
type TableOf = fn(&RequestEditor, &gpui_kit::App) -> Entity<apinata_app::kv_table::KvTable>;

/// Opens `text` as h.yml and reloads `outside` over it. At the first workspace notify where
/// `landed(opened, now)` holds, the rows `table` still shows in `section` are taken and
/// `edit`ed; once the editor has refreshed, they come back as a table change from rows the
/// reload replaced.
/// Returns the rig, the request file, its view and whether the edit was held as stale.
fn reload_race(
    cx: &mut TestAppContext,
    text: &str,
    outside: &str,
    (section, table): (Section, TableOf),
    landed: fn(&RequestView, &RequestView) -> bool,
    edit: fn(&mut Rows),
) -> (Rig, std::path::PathBuf, Arc<RequestView>, bool) {
    let taken: std::rc::Rc<std::cell::RefCell<Option<Rows>>> = Default::default();
    let (t1, t2) = (taken.clone(), taken);
    let before = Box::new(
        move |cx: &mut gpui_kit::App, ed: &Entity<RequestEditor>, _| {
            let table = table(ed.read(cx), cx);
            let mut rows = table.read(cx).rows(cx);
            edit(&mut rows);
            *t1.borrow_mut() = Some(rows);
        },
    );
    let after = Box::new(
        move |cx: &mut gpui_kit::App, ed: &Entity<RequestEditor>, _| {
            let table = table(ed.read(cx), cx);
            let rows = t2.borrow_mut().take().unwrap();
            table.update(cx, |_, cx| {
                cx.emit(apinata_app::kv_table::KvEvent::Changed(rows))
            });
        },
    );
    race_around_refresh(cx, (text, outside), section, landed, before, after)
}

/// Opens `text` as h.yml, shows `section` and reloads `outside` over it. `before` runs at
/// the first workspace notify where `landed(opened, now)` holds, ahead of the editor's
/// refresh; `after` runs at the same notify once the editor has refreshed.
/// Returns the rig, the request file, its view and whether an edit was held as stale.
fn race_around_refresh(
    cx: &mut TestAppContext,
    (text, outside): (&str, &str),
    section: Section,
    landed: fn(&RequestView, &RequestView) -> bool,
    before: Box<OnEditor>,
    after: Box<OnEditor>,
) -> (Rig, std::path::PathBuf, Arc<RequestView>, bool) {
    support::init(cx);
    let col = write_tree(&[("opencollection.yml", ROOT), ("h.yml", text)]);
    let (state, ws, root) = open_dir(cx, &col);
    type Slot = Option<(Entity<RequestEditor>, AnyWindowHandle)>;
    let slot: std::rc::Rc<std::cell::RefCell<Slot>> = Default::default();
    let opened: std::rc::Rc<std::cell::RefCell<Option<Arc<RequestView>>>> = Default::default();
    let observe = |cx: &mut TestAppContext, run: Box<OnEditor>| {
        let (r2, item, s, o) = (root.clone(), ip("h.yml"), slot.clone(), opened.clone());
        let fired = std::cell::Cell::new(false);
        cx.update(|cx| {
            cx.observe(&ws, move |ws, cx| {
                let now = ws.read(cx).request(&r2, &item, cx);
                let done = (o.borrow().as_ref())
                    .zip(now)
                    .is_some_and(|(o, now)| landed(o, &now));
                if done && !fired.replace(true) {
                    let (ed, win) = s.borrow().clone().unwrap();
                    run(cx, &ed, win);
                }
            })
        })
    };
    let _before = observe(cx, before);
    let (win, ed) = editor_for(cx, &ws, &root, "h.yml", false);
    *slot.borrow_mut() = Some((ed.clone(), win));
    *opened.borrow_mut() = cx.read(|cx| ws.read(cx).request(&root, &ip("h.yml"), cx));
    let _after = observe(cx, after);
    let r = Rig {
        _col: col,
        _state: state,
        ws,
        root,
        win,
        ed,
    };
    r.show(cx, section);
    let file = r.root.join("h.yml");
    std::fs::write(&file, outside).unwrap();
    let reload = apinata_app::command::Command::Reload(vec![file.clone()]);
    let root = r.root.clone();
    r.ws.update(cx, |w, cx| w.send(&root, reload, cx));
    cx.run_until_parked();
    frame(cx, r.win);
    cx.run_until_parked();
    let v = r.view(cx, "h.yml");
    let held = ws_snap(cx, &r.ws, &r.root)
        .stale
        .contains_key(&apinata_app::command::Target::Request(ip("h.yml")));
    (r, file, v, held)
}

// The reload turns v1 off and changes v3's declared type, a field the Vars table does not
// show. The user's v1 toggle, built from the rows before the reload, is handled after the
// refresh: its rows match the reload's in every shown field, yet must not write the old type.
#[gpui_kit::test]
fn a_table_edit_from_rows_a_hidden_field_reload_replaced_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    let outside = TYPED
        .replace("type: number", "type: boolean")
        .replace("value: one\n", "value: one\n      disabled: true\n");
    let landed = |_: &RequestView, now: &RequestView| v3_is_boolean(now);
    let (r, file, v, held) = reload_race(
        cx,
        TYPED,
        &outside,
        (Section::Vars, vars_table),
        landed,
        |rows| rows[0].enabled = false,
    );
    assert!(v3_is_boolean(&v), "the outside type survives");
    // The outside change already turned v1 off, so only the hold-back shows the edit was seen.
    assert!(held, "the v1 edit built from the replaced entries is held");
    if v.dirty {
        r.save_now(cx);
    }
    let disk = std::fs::read_to_string(&file).unwrap();
    assert!(disk.contains("type: boolean"), "{disk}");
}

const NOTED: &str = "info:\n  name: h\n  type: http\n  seq: 7\nhttp:\n  method: GET\n  url: http://x\n  headers:\n    - name: h1\n      value: one\n      x-note: old\n    - name: h2\n      value: two\n";

// The reload changes only h1's unknown `x-note`. A table change from the rows the reload
// replaced matches the reload in every field core models, since any shown or modeled
// difference already counts as lagging, yet must not write h1's old note back.
#[gpui_kit::test]
fn a_table_edit_from_rows_an_unknown_key_reload_replaced_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    let landed = |o: &RequestView, now: &RequestView| !o.headers[0].same_source(&now.headers[0]);
    unknown_key_race(cx, NOTED, (Section::Headers, headers_table), landed);
}

fn vars_table(ed: &RequestEditor, _: &gpui_kit::App) -> Entity<apinata_app::kv_table::KvTable> {
    ed.table(Section::Vars).cloned().unwrap()
}

fn headers_table(ed: &RequestEditor, _: &gpui_kit::App) -> Entity<apinata_app::kv_table::KvTable> {
    ed.table(Section::Headers).cloned().unwrap()
}

fn post_response_table(
    ed: &RequestEditor,
    _: &gpui_kit::App,
) -> Entity<apinata_app::kv_table::KvTable> {
    ed.post_response_table().clone()
}

fn body_table(ed: &RequestEditor, cx: &gpui_kit::App) -> Entity<apinata_app::kv_table::KvTable> {
    ed.body().read(cx).table().clone()
}

/// Runs the unknown-key race on `text`'s first entry in `table`, whose `x-note: old` the
/// reload turns into `x-note: new`, and asserts the new note survives the stale change.
fn unknown_key_race(
    cx: &mut TestAppContext,
    text: &str,
    table: (Section, TableOf),
    landed: fn(&RequestView, &RequestView) -> bool,
) {
    let outside = text.replace("x-note: old", "x-note: new");
    let (r, file, v, held) = reload_race(cx, text, &outside, table, landed, |_| {});
    if v.dirty {
        r.save_now(cx);
    }
    let disk = std::fs::read_to_string(&file).unwrap();
    assert!(disk.contains("x-note: new"), "{disk}");
    assert!(held, "the change built from the replaced entries is held");
}

const NOTED_ACTION: &str = "info:\n  name: h\n  type: http\n  seq: 7\nhttp:\n  method: GET\n  url: http://x\nruntime:\n  actions:\n    - type: set-variable\n      phase: after-response\n      selector:\n        expression: res.body.id\n        method: jsonq\n      variable:\n        name: id\n        scope: runtime\n      x-note: old\n";

// As the header case, for a post-response variable's unknown key.
#[gpui_kit::test]
fn a_post_response_edit_from_rows_an_unknown_key_reload_replaced_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    let landed = |o: &RequestView, now: &RequestView| {
        !o.post_response_vars[0].same_source(&now.post_response_vars[0])
    };
    unknown_key_race(
        cx,
        NOTED_ACTION,
        (Section::Vars, post_response_table),
        landed,
    );
}

const NOTED_PART: &str = "info:\n  name: h\n  type: http\n  seq: 7\nhttp:\n  method: POST\n  url: http://x\n  body:\n    type: multipart-form\n    data:\n      - name: f\n        type: text\n        value: v\n        x-note: old\n";

fn first_part(v: &RequestView) -> &apinata_core::MultipartField {
    match &v.body {
        BodySpec::Multipart(fields) => &fields[0],
        other => panic!("{other:?}"),
    }
}

// As the header case, for a multipart field's unknown key.
#[gpui_kit::test]
fn a_multipart_edit_from_rows_an_unknown_key_reload_replaced_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    let landed = |o: &RequestView, now: &RequestView| !first_part(o).same_source(first_part(now));
    unknown_key_race(cx, NOTED_PART, (Section::Body, body_table), landed);
}

// The reload changes the multipart field's shown value. A body table change from the rows
// the reload replaced must count as lagging like any other table's, not write `v` back.
#[gpui_kit::test]
fn a_multipart_edit_from_rows_a_shown_value_reload_replaced_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    let outside = NOTED_PART.replace("value: v", "value: w");
    let landed = |o: &RequestView, now: &RequestView| first_part(o) != first_part(now);
    let (r, file, v, held) = reload_race(
        cx,
        NOTED_PART,
        &outside,
        (Section::Body, body_table),
        landed,
        |_| {},
    );
    if v.dirty {
        r.save_now(cx);
    }
    let disk = std::fs::read_to_string(&file).unwrap();
    assert!(disk.contains("value: w"), "{disk}");
    assert!(held, "the change built from the replaced rows is held");
}

const NOTED_FILE: &str = "info:\n  name: h\n  type: http\n  seq: 7\nhttp:\n  method: POST\n  url: http://x\n  body:\n    type: file\n    data:\n      - filePath: /tmp/b.json\n        contentType: application/json\n        selected: true\n        x-note: old\n";

fn first_file(v: &RequestView) -> &apinata_core::FileEntry {
    match &v.body {
        BodySpec::File(files) => &files[0],
        other => panic!("{other:?}"),
    }
}

// As the header case, for a file body entry's unknown key.
#[gpui_kit::test]
fn a_file_body_edit_from_rows_an_unknown_key_reload_replaced_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    let landed = |o: &RequestView, now: &RequestView| !first_file(o).same_source(first_file(now));
    unknown_key_race(cx, NOTED_FILE, (Section::Body, body_table), landed);
}

const NOTED_ASSERT: &str = "info:\n  name: h\n  type: http\n  seq: 7\nhttp:\n  method: GET\n  url: http://x\nruntime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n      x-note: old\n    - expression: res.body.id\n      operator: isDefined\n";

/// Reloads `outside` over [`NOTED_ASSERT`] and, in the window before the editor refreshes,
/// changes the second assertion's operator in the table; the section's edit reaches the
/// editor only after the refresh. Saves and returns the file on disk and whether the edit
/// was held.
fn assertion_race(
    cx: &mut TestAppContext,
    outside: &str,
    landed: fn(&RequestView, &RequestView) -> bool,
) -> (String, bool) {
    let before = Box::new(|cx: &mut gpui_kit::App, ed: &Entity<RequestEditor>, win| {
        let section = ed.read(cx).assertions().clone();
        cx.update_window(win, |_, w, cx| {
            section.update(cx, |s, cx| s.set_operator(1, "isUndefined", w, cx))
        })
        .unwrap();
    });
    let after = Box::new(|_: &mut gpui_kit::App, _: &Entity<RequestEditor>, _| {});
    let (r, file, v, held) = race_around_refresh(
        cx,
        (NOTED_ASSERT, outside),
        Section::Assertions,
        landed,
        before,
        after,
    );
    if v.dirty {
        r.save_now(cx);
    }
    (std::fs::read_to_string(&file).unwrap(), held)
}

// The reload changes the first assertion's shown value. An assertions table edit built from
// the entries the reload replaced must count as lagging, not write `200` back.
#[gpui_kit::test]
fn an_assertion_edit_from_entries_a_shown_value_reload_replaced_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    let outside = NOTED_ASSERT.replace("value: \"200\"", "value: \"201\"");
    let landed = |o: &RequestView, now: &RequestView| o.assertions[0] != now.assertions[0];
    let (disk, held) = assertion_race(cx, &outside, landed);
    assert!(disk.contains("value: \"201\""), "{disk}");
    assert!(held, "the edit built from the replaced entries is held");
}

// As the shown-value case, for an assertion's unknown key. The reload also makes the
// table's own operator change, so the stale edit matches it in every modeled field and
// only the YAML the first entry was read from tells them apart.
#[gpui_kit::test]
fn an_assertion_edit_from_entries_an_unknown_key_reload_replaced_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    let outside = NOTED_ASSERT
        .replace("x-note: old", "x-note: new")
        .replace("operator: isDefined", "operator: isUndefined");
    let landed =
        |o: &RequestView, now: &RequestView| !o.assertions[0].same_source(&now.assertions[0]);
    let (disk, held) = assertion_race(cx, &outside, landed);
    assert!(disk.contains("x-note: new"), "{disk}");
    assert!(held, "the edit built from the replaced entries is held");
}

/// A whole-value section edit, its file before and after an outside reload, and the text
/// each leaves on disk. The stale edit differs from `outside` only in what the reload
/// changed, so a held edit and an ordinary one cannot be mistaken for each other.
struct SectionRace {
    text: &'static str,
    outside: &'static str,
    section: Section,
    landed: fn(&RequestView, &RequestView) -> bool,
    edit: fn(&mut gpui_kit::App, &Entity<RequestEditor>, AnyWindowHandle),
    /// On disk once the outside change survives the stale edit.
    kept: &'static str,
    /// On disk once the edit lands without a reload.
    edited: &'static str,
    /// Watches the section's edits, noting whether each lags as it reaches the editor;
    /// `None` for a text section, which never counts its edits as lagging.
    watch: Option<Watch>,
    /// Whether the stale edit reaches the editor at all. A text section reads its editor
    /// when the change event arrives, after the refresh put the outside text there, so it
    /// sends that text and nothing is held.
    held: bool,
}

impl SectionRace {
    /// Makes the edit in the window before the editor's refresh of the reload, saves, and
    /// asserts the outside change survives and the edit is held.
    fn race(self, cx: &mut TestAppContext) {
        let (r, file, v, held) = race_around_refresh(
            cx,
            (self.text, self.outside),
            self.section,
            self.landed,
            Box::new(self.edit),
            Box::new(|_: &mut gpui_kit::App, _: &Entity<RequestEditor>, _| {}),
        );
        if v.dirty {
            r.save_now(cx);
        }
        let disk = std::fs::read_to_string(&file).unwrap();
        assert!(disk.contains(self.kept), "{disk}");
        assert_eq!(held, self.held, "whether the stale edit is held");
    }

    /// Makes the edit once an unrelated reload has settled, saves, and asserts it lands,
    /// nothing is held and the section did not count it as lagging.
    fn plain(self, cx: &mut TestAppContext) {
        support::init(cx);
        let col = write_tree(&[("opencollection.yml", ROOT), ("h.yml", self.text)]);
        let (state, ws, root) = open_dir(cx, &col);
        let (win, ed) = editor_for(cx, &ws, &root, "h.yml", false);
        let r = Rig {
            _col: col,
            _state: state,
            ws,
            root,
            win,
            ed,
        };
        r.show(cx, self.section);
        let file = r.root.join("h.yml");
        std::fs::write(&file, self.text.replace("url: http://x", "url: http://y")).unwrap();
        let reload = apinata_app::command::Command::Reload(vec![file]);
        let root = r.root.clone();
        r.ws.update(cx, |w, cx| w.send(&root, reload, cx));
        cx.run_until_parked();
        frame(cx, r.win);
        assert_eq!(r.view(cx, "h.yml").url, "http://y", "the reload settled");
        let lagged = Lagged::default();
        let _watch = (self.watch).map(|w| cx.update(|cx| w(cx, &r.ed, lagged.clone())));
        cx.update(|cx| (self.edit)(cx, &r.ed, r.win));
        cx.run_until_parked();
        if self.watch.is_some() {
            let lagged = lagged.borrow();
            assert!(!lagged.is_empty(), "the section sent an edit");
            assert!(!lagged.iter().any(|&l| l), "an ordinary edit does not lag");
        }
        assert!(
            ws_snap(cx, &r.ws, &r.root).stale.is_empty(),
            "nothing is held"
        );
        r.save_now(cx);
        let disk = std::fs::read_to_string(r.root.join("h.yml")).unwrap();
        assert!(disk.contains(self.edited), "{disk}");
    }
}

/// Whether each edit a watched section sent lagged as it arrived.
type Lagged = std::rc::Rc<std::cell::RefCell<Vec<bool>>>;

type Watch = fn(&mut gpui_kit::App, &Entity<RequestEditor>, Lagged) -> gpui_kit::Subscription;

/// As [`watch`], for the auth section, whose `lags` reads the edit's load stamp.
fn watch_auth(
    cx: &mut gpui_kit::App,
    ed: &Entity<RequestEditor>,
    l: Lagged,
) -> gpui_kit::Subscription {
    let auth = ed.read(cx).auth().clone();
    cx.subscribe(
        &auth,
        move |s, e: &apinata_app::section::SectionEdit, cx| {
            let lags = s.read(cx).lags(e);
            l.borrow_mut().push(lags);
        },
    )
}

/// Notes, for each edit `section` emits, what its `lags` says on delivery.
fn watch<T: gpui_kit::EventEmitter<apinata_app::section::SectionEdit>>(
    cx: &mut gpui_kit::App,
    section: Entity<T>,
    lagged: Lagged,
    lags: fn(&T, &apinata_app::command::Edit, &gpui_kit::App) -> bool,
) -> gpui_kit::Subscription {
    cx.subscribe(
        &section,
        move |s, e: &apinata_app::section::SectionEdit, cx| {
            let l = lags(s.read(cx), &e.edit, cx);
            lagged.borrow_mut().push(l);
        },
    )
}

const SETTINGS_FILE: &str = "info:\n  name: h\n  type: http\n  seq: 7\nhttp:\n  method: GET\n  url: http://x\nsettings:\n  maxRedirects: 5\n";

// The user picks a custom timeout; the reload picks the same and changes max redirects.
fn settings_race() -> SectionRace {
    SectionRace {
        text: SETTINGS_FILE,
        outside: "info:\n  name: h\n  type: http\n  seq: 7\nhttp:\n  method: GET\n  url: http://x\nsettings:\n  timeout: 0\n  maxRedirects: 7\n",
        section: Section::Settings,
        landed: |o, now| o.settings != now.settings,
        edit: |cx, ed, win| {
            let section = ed.read(cx).settings().clone();
            cx.update_window(win, |_, w, cx| {
                section.update(cx, |s, cx| {
                    s.set_timeout_mode(apinata_app::settings_section::TimeoutMode::Custom, w, cx)
                })
            })
            .unwrap();
        },
        kept: "maxRedirects: 7",
        edited: "timeout: 0",
        watch: Some(|cx, ed, l| {
            watch(cx, ed.read(cx).settings().clone(), l, |s, e, cx| {
                s.lags(e, cx)
            })
        }),
        held: true,
    }
}

#[gpui_kit::test]
fn a_settings_edit_from_values_a_reload_replaced_keeps_the_outside_change(cx: &mut TestAppContext) {
    settings_race().race(cx);
}

#[gpui_kit::test]
fn a_settings_edit_after_a_settled_reload_lands(cx: &mut TestAppContext) {
    settings_race().plain(cx);
}

// The user removes tag b; the reload removes it too and renames a.
fn tags_race() -> SectionRace {
    SectionRace {
        text: "info:\n  name: h\n  type: http\n  seq: 7\n  tags:\n    - a\n    - b\nhttp:\n  method: GET\n  url: http://x\n",
        outside: "info:\n  name: h\n  type: http\n  seq: 7\n  tags:\n    - x\nhttp:\n  method: GET\n  url: http://x\n",
        section: Section::Settings,
        landed: |o, now| o.tags != now.tags,
        edit: |cx, ed, _| {
            let tags = ed.read(cx).settings().read(cx).tags().clone();
            tags.update(cx, |t, cx| t.remove(1, cx));
        },
        kept: "- x",
        edited: "tags:\n    - a\n",
        watch: Some(|cx, ed, l| {
            watch(cx, ed.read(cx).settings().clone(), l, |s, e, cx| {
                s.lags(e, cx)
            })
        }),
        held: true,
    }
}

#[gpui_kit::test]
fn a_tags_edit_from_values_a_reload_replaced_keeps_the_outside_change(cx: &mut TestAppContext) {
    tags_race().race(cx);
}

#[gpui_kit::test]
fn a_tags_edit_after_a_settled_reload_lands(cx: &mut TestAppContext) {
    tags_race().plain(cx);
}

/// Replaces the whole text of `code` as typing would, so its change event fires.
fn type_into(
    cx: &mut gpui_kit::App,
    win: AnyWindowHandle,
    code: Entity<gpui_kit::component::input::EditorState>,
    text: &str,
) {
    cx.update_window(win, |_, w, cx| {
        code.update(cx, |c, cx| c.replace_all(text.to_string(), w, cx))
    })
    .unwrap();
}

// The user adds `!` to the docs; the reload adds it too and rewrites the first word.
fn docs_race() -> SectionRace {
    SectionRace {
        text: "info:\n  name: h\n  type: http\n  seq: 7\nhttp:\n  method: GET\n  url: http://x\ndocs: old notes\n",
        outside: "info:\n  name: h\n  type: http\n  seq: 7\nhttp:\n  method: GET\n  url: http://x\ndocs: new notes!\n",
        section: Section::Docs,
        landed: |o, now| o.docs != now.docs,
        edit: |cx, ed, win| {
            let code = ed.read(cx).docs().read(cx).code().clone();
            type_into(cx, win, code, "old notes!");
        },
        kept: "new notes!",
        edited: "old notes!",
        watch: None,
        held: false,
    }
}

#[gpui_kit::test]
fn a_docs_edit_from_text_a_reload_replaced_keeps_the_outside_change(cx: &mut TestAppContext) {
    docs_race().race(cx);
}

#[gpui_kit::test]
fn a_docs_edit_after_a_settled_reload_lands(cx: &mut TestAppContext) {
    docs_race().plain(cx);
}

const SCRIPTED: &str = "info:\n  name: h\n  type: http\n  seq: 7\nhttp:\n  method: GET\n  url: http://x\nruntime:\n  scripts:\n    - type: before-request\n      code: a()\n    - type: after-response\n      code: a()\n    - type: tests\n      code: a()\n";

// The user appends `b()` to one phase's script; the reload appends it too and rewrites the
// first call, in that phase only.
fn script_race(phase: apinata_core::ScriptPhase) -> SectionRace {
    use apinata_core::ScriptPhase::*;
    let (outside, section, landed, edit): (_, _, fn(&RequestView, &RequestView) -> bool, _) =
        match phase {
            PreRequest => (
                "info:\n  name: h\n  type: http\n  seq: 7\nhttp:\n  method: GET\n  url: http://x\nruntime:\n  scripts:\n    - type: before-request\n      code: |-\n        x()\n        b()\n    - type: after-response\n      code: a()\n    - type: tests\n      code: a()\n",
                Section::Script,
                |o, now| o.scripts.pre_request != now.scripts.pre_request,
                (|cx, ed, win| script_edit(cx, ed.read(cx).script().clone(), PreRequest, win))
                    as fn(&mut gpui_kit::App, &Entity<RequestEditor>, AnyWindowHandle),
            ),
            PostResponse => (
                "info:\n  name: h\n  type: http\n  seq: 7\nhttp:\n  method: GET\n  url: http://x\nruntime:\n  scripts:\n    - type: before-request\n      code: a()\n    - type: after-response\n      code: |-\n        x()\n        b()\n    - type: tests\n      code: a()\n",
                Section::Script,
                |o, now| o.scripts.post_response != now.scripts.post_response,
                |cx, ed, win| script_edit(cx, ed.read(cx).script().clone(), PostResponse, win),
            ),
            _ => (
                "info:\n  name: h\n  type: http\n  seq: 7\nhttp:\n  method: GET\n  url: http://x\nruntime:\n  scripts:\n    - type: before-request\n      code: a()\n    - type: after-response\n      code: a()\n    - type: tests\n      code: |-\n        x()\n        b()\n",
                Section::Tests,
                |o, now| o.scripts.tests != now.scripts.tests,
                |cx, ed, win| script_edit(cx, ed.read(cx).tests_script().clone(), Tests, win),
            ),
        };
    SectionRace {
        text: SCRIPTED,
        outside,
        section,
        landed,
        edit,
        kept: "x()\n        b()",
        edited: "a()\n        b()",
        watch: None,
        held: false,
    }
}

fn script_edit(
    cx: &mut gpui_kit::App,
    section: Entity<apinata_app::script_section::ScriptSection>,
    phase: apinata_core::ScriptPhase,
    win: AnyWindowHandle,
) {
    cx.update_window(win, |_, w, cx| {
        section.update(cx, |s, cx| assert!(s.append_text(phase, "a()\nb()", w, cx)))
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_pre_request_script_edit_from_text_a_reload_replaced_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    script_race(apinata_core::ScriptPhase::PreRequest).race(cx);
}

#[gpui_kit::test]
fn a_post_response_script_edit_from_text_a_reload_replaced_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    script_race(apinata_core::ScriptPhase::PostResponse).race(cx);
}

#[gpui_kit::test]
fn a_tests_script_edit_from_text_a_reload_replaced_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    script_race(apinata_core::ScriptPhase::Tests).race(cx);
}

#[gpui_kit::test]
fn a_pre_request_script_edit_after_a_settled_reload_lands(cx: &mut TestAppContext) {
    script_race(apinata_core::ScriptPhase::PreRequest).plain(cx);
}

#[gpui_kit::test]
fn a_tests_script_edit_after_a_settled_reload_lands(cx: &mut TestAppContext) {
    script_race(apinata_core::ScriptPhase::Tests).plain(cx);
}

// The user rewrites the query; the reload rewrites it the same way and changes the variables.
fn graphql_race() -> SectionRace {
    SectionRace {
        text: "info:\n  name: h\n  type: graphql\n  seq: 7\ngraphql:\n  url: http://x\n  body:\n    query: \"{ a }\"\n    variables: '{\"old\": 1}'\n",
        outside: "info:\n  name: h\n  type: graphql\n  seq: 7\ngraphql:\n  url: http://x\n  body:\n    query: \"{ b }\"\n    variables: '{\"new\": 1}'\n",
        section: Section::Body,
        landed: |o, now| o.body != now.body,
        edit: |cx, ed, win| {
            let query = ed.read(cx).graphql().read(cx).query().clone();
            type_into(cx, win, query, "{ b }");
        },
        kept: "new",
        edited: "{ b }",
        watch: None,
        held: false,
    }
}

#[gpui_kit::test]
fn a_graphql_edit_from_text_a_reload_replaced_keeps_the_outside_change(cx: &mut TestAppContext) {
    graphql_race().race(cx);
}

#[gpui_kit::test]
fn a_graphql_edit_after_a_settled_reload_lands(cx: &mut TestAppContext) {
    graphql_race().plain(cx);
}

// The user moves the API key to the query; the reload moves it too and changes its value.
fn auth_race() -> SectionRace {
    SectionRace {
        text: "info:\n  name: h\n  type: http\n  seq: 7\nhttp:\n  method: GET\n  url: http://x\n  auth:\n    type: apikey\n    key: k\n    value: old\n    placement: header\n",
        outside: "info:\n  name: h\n  type: http\n  seq: 7\nhttp:\n  method: GET\n  url: http://x\n  auth:\n    type: apikey\n    key: k\n    value: new\n    placement: query\n",
        section: Section::Auth,
        landed: |o, now| o.auth != now.auth,
        edit: |cx, ed, win| {
            let auth = ed.read(cx).auth().clone();
            cx.update_window(win, |_, w, cx| {
                auth.update(cx, |a, cx| {
                    a.set_placement(apinata_core::ApiKeyPlacement::Query, w, cx)
                })
            })
            .unwrap();
        },
        kept: "value: new",
        edited: "placement: query",
        watch: Some(watch_auth),
        held: true,
    }
}

#[gpui_kit::test]
fn an_auth_edit_from_values_a_reload_replaced_keeps_the_outside_change(cx: &mut TestAppContext) {
    auth_race().race(cx);
}

#[gpui_kit::test]
fn an_auth_edit_after_a_settled_reload_lands(cx: &mut TestAppContext) {
    auth_race().plain(cx);
}

// The user moves the OAuth 1.0 parameters to the query in the sub-editor; the reload moves
// them too and changes the consumer key. The sub-editor's event reaches the section only
// after the refresh loaded the outside spec.
fn oauth1_race() -> SectionRace {
    SectionRace {
        text: "info:\n  name: h\n  type: http\n  seq: 7\nhttp:\n  method: GET\n  url: http://x\n  auth:\n    type: oauth1\n    consumerKey: old\n    signatureMethod: PLAINTEXT\n    placement: header\n",
        outside: "info:\n  name: h\n  type: http\n  seq: 7\nhttp:\n  method: GET\n  url: http://x\n  auth:\n    type: oauth1\n    consumerKey: new\n    signatureMethod: PLAINTEXT\n    placement: query\n",
        section: Section::Auth,
        landed: |o, now| o.auth != now.auth,
        edit: |cx, ed, win| {
            let oauth1 = ed.read(cx).auth().read(cx).oauth1().clone();
            cx.update_window(win, |_, w, cx| {
                oauth1.update(cx, |e, cx| {
                    e.set_placement(apinata_core::OAuth1Placement::Query, w, cx)
                })
            })
            .unwrap();
        },
        kept: "consumerKey: new",
        edited: "placement: query",
        watch: Some(watch_auth),
        held: true,
    }
}

#[gpui_kit::test]
fn an_oauth1_edit_from_a_spec_a_reload_replaced_keeps_the_outside_change(cx: &mut TestAppContext) {
    oauth1_race().race(cx);
}

#[gpui_kit::test]
fn an_oauth1_edit_after_a_settled_reload_lands(cx: &mut TestAppContext) {
    oauth1_race().plain(cx);
}

// As the OAuth 1.0 race, for the OAuth 2.0 sub-editor: the user switches the grant and the
// reload switches it too and changes the token URL.
fn oauth2_race() -> SectionRace {
    SectionRace {
        text: OAUTH2_GRANT,
        outside: "info:\n  name: h\n  type: http\n  seq: 7\nhttp:\n  method: GET\n  url: http://x\n  auth:\n    type: oauth2\n    flow: resource_owner_password_credentials\n    accessTokenUrl: http://id/new\n",
        section: Section::Auth,
        landed: |o, now| o.auth != now.auth,
        edit: |cx, ed, win| {
            let oauth2 = ed.read(cx).auth().read(cx).oauth2().clone();
            cx.update_window(win, |_, w, cx| {
                oauth2.update(cx, |e, cx| {
                    e.set_flow(apinata_core::OAuth2Flow::Password, w, cx)
                })
            })
            .unwrap();
        },
        kept: "accessTokenUrl: http://id/new",
        edited: "flow: resource_owner_password_credentials",
        watch: Some(watch_auth),
        held: true,
    }
}

const OAUTH2_GRANT: &str = "info:\n  name: h\n  type: http\n  seq: 7\nhttp:\n  method: GET\n  url: http://x\n  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: http://id/token\n";

#[gpui_kit::test]
fn an_oauth2_edit_from_a_spec_a_reload_replaced_keeps_the_outside_change(cx: &mut TestAppContext) {
    oauth2_race().race(cx);
}

#[gpui_kit::test]
fn an_oauth2_edit_after_a_settled_reload_lands(cx: &mut TestAppContext) {
    oauth2_race().plain(cx);
}

// The user omits Accept; the reload omits it too and also omits User-Agent.
fn omit_race() -> SectionRace {
    SectionRace {
        text: "info:\n  name: h\n  type: http\n  seq: 7\nhttp:\n  method: GET\n  url: http://x\n",
        outside: "info:\n  name: h\n  type: http\n  seq: 7\nhttp:\n  method: GET\n  url: http://x\nsettings:\n  omitHeaders:\n    - Accept\n    - User-Agent\n",
        section: Section::Headers,
        landed: |o, now| o.omitted_headers != now.omitted_headers,
        edit: |cx, _, win| {
            cx.update_window(win, |_, w, cx| w.click("default-header-send-accept", cx))
                .unwrap();
        },
        kept: "- User-Agent",
        edited: "omitHeaders:\n    - Accept\n",
        watch: Some(|cx, ed, l| {
            watch(cx, ed.read(cx).default_headers().clone(), l, |s, e, _| {
                s.lags(e)
            })
        }),
        held: true,
    }
}

#[gpui_kit::test]
fn a_default_header_edit_from_values_a_reload_replaced_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    omit_race().race(cx);
}

#[gpui_kit::test]
fn a_default_header_edit_after_a_settled_reload_lands(cx: &mut TestAppContext) {
    omit_race().plain(cx);
}

// The user switches the JSON body to Text; the reload does too, with text of its own.
fn body_mode_race() -> SectionRace {
    SectionRace {
        text: "info:\n  name: h\n  type: http\n  seq: 7\nhttp:\n  method: POST\n  url: http://x\n  body:\n    type: json\n    data: '{}'\n",
        outside: "info:\n  name: h\n  type: http\n  seq: 7\nhttp:\n  method: POST\n  url: http://x\n  body:\n    type: text\n    data: hello\n",
        section: Section::Body,
        landed: |o, now| o.body != now.body,
        edit: |cx, ed, win| {
            let body = ed.read(cx).body().clone();
            cx.update_window(win, |_, w, cx| {
                body.update(cx, |b, cx| {
                    b.set_mode(apinata_app::body_section::BodyMode::Text, w, cx)
                })
            })
            .unwrap();
        },
        kept: "data: hello",
        edited: "type: text",
        watch: Some(|cx, ed, l| watch(cx, ed.read(cx).body().clone(), l, |s, e, _| s.lags(e))),
        held: true,
    }
}

#[gpui_kit::test]
fn a_body_mode_switch_from_a_body_a_reload_replaced_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    body_mode_race().race(cx);
}

#[gpui_kit::test]
fn a_body_mode_switch_after_a_settled_reload_lands(cx: &mut TestAppContext) {
    body_mode_race().plain(cx);
}

impl Rig {
    /// After a key press that should find nothing to take back: the rows and the disk keep
    /// the outside value, and the tab is clean with nothing held.
    fn assert_untouched_after_reload(&self, cx: &mut TestAppContext, rows: &[(&str, &str)]) {
        assert_eq!(self.header_pairs(cx), pairs(rows));
        let v = self.view(cx, "h.yml");
        assert!(!v.dirty, "nothing was written over the outside change");
        assert!(ws_snap(cx, &self.ws, &self.root).stale.is_empty());
        let disk = std::fs::read_to_string(self.root.join("h.yml")).unwrap();
        assert!(
            disk.contains("three-x") && !disk.contains("name: h2"),
            "{disk}"
        );
    }
}

const TWO_OUTSIDE: &str = "info:\n  name: h\n  type: http\n  seq: 6\nhttp:\n  method: GET\n  url: http://x\n  headers:\n    - name: h1\n      value: one\n    - name: h3\n      value: three-x\n";

// Ctrl+Z straight after a reload of the field it would undo finds nothing to take back.
#[gpui_kit::test]
fn undo_right_after_a_disk_reload_writes_nothing(cx: &mut TestAppContext) {
    let r = watched_three(cx);
    r.click(cx, "headers", ("kv-delete", 1usize));
    r.save_now(cx);
    r.outside_write(cx, TWO_OUTSIDE, "h3", "three-x");
    press_key(cx, r.win, &secondary("z"));
    settle(cx);
    r.assert_untouched_after_reload(cx, &[("h1", "one"), ("h3", "three-x")]);
}

// A redo with a reload of its field between it and its undo finds nothing to redo.
#[gpui_kit::test]
fn redo_after_a_disk_reload_since_its_undo_writes_nothing(cx: &mut TestAppContext) {
    let r = watched_three(cx);
    r.click(cx, "headers", ("kv-delete", 1usize));
    r.save_now(cx);
    press_key(cx, r.win, &secondary("z"));
    assert_eq!(
        header_names(cx, &r),
        ["h1", "h2", "h3"],
        "the undo brings h2 back"
    );
    r.save_now(cx);
    r.outside_write(cx, TWO_OUTSIDE, "h3", "three-x");
    press_key(cx, r.win, &secondary("shift-z"));
    settle(cx);
    r.assert_untouched_after_reload(cx, &[("h1", "one"), ("h3", "three-x")]);
}

impl Rig {
    /// Writes `text` to h.yml and reloads it, without drawing.
    fn reload_undrawn(&self, cx: &mut TestAppContext, text: &str) {
        let file = self.root.join("h.yml");
        std::fs::write(&file, text).unwrap();
        let reload = apinata_app::command::Command::Reload(vec![file]);
        let root = self.root.clone();
        self.ws.update(cx, |w, cx| w.send(&root, reload, cx));
        cx.run_until_parked();
    }

    fn click_url(&self, cx: &mut TestAppContext) {
        cx.update_window(self.win, |_, w, cx| {
            use gpui_kit::test::TestWindowExt as _;
            w.click("url", cx)
        })
        .unwrap();
        frame(cx, self.win);
        assert!(
            holds_focus(cx, self.win, "url"),
            "the user moved focus to the URL"
        );
    }
}

fn without_h1() -> String {
    THREE.replace("    - name: h1\n      value: one\n", "")
}

fn without_h1_h2() -> String {
    without_h1().replace("    - name: h2\n      value: two\n", "")
}

impl Rig {
    /// Shows `section` the way a command does, leaving focus where it is.
    fn show_quietly(&self, cx: &mut TestAppContext, section: Section) {
        self.ed.update(cx, |e, cx| e.show(section, cx));
        frame(cx, self.win);
    }
}

/// Hides Headers in the update the first snapshot with two headers lands in, after the
/// editor's refresh: the reload's pending Remove focus then never gets the draw it waits for.
fn hide_headers_when_two_land(cx: &mut TestAppContext, r: &Rig) -> gpui_kit::Subscription {
    let (root, item, ed) = (r.root.clone(), ip("h.yml"), r.ed.clone());
    let armed = std::cell::Cell::new(true);
    cx.update(|cx| {
        cx.observe(&r.ws, move |ws, cx| {
            let two = ws
                .read(cx)
                .request(&root, &item, cx)
                .map(|v| v.headers.len())
                == Some(2);
            if two && armed.replace(false) {
                ed.update(cx, |e, cx| e.show(Section::Body, cx));
            }
        })
    })
}

// A reload moves h3 while its Remove holds focus and Headers hides before the next draw; the
// user moves on to the URL (and `twice`, another reload lands while hidden). Showing Headers
// again leaves focus on the URL.
fn hidden_headers_reloads(cx: &mut TestAppContext, twice: bool) {
    let r = three_saved_headers_with_the_last_remove_focused(cx);
    let _hide = hide_headers_when_two_land(cx, &r);
    r.reload_undrawn(cx, &without_h1());
    frame(cx, r.win);
    assert_eq!(r.ed.read_with(cx, |e, _| e.section()), Section::Body);
    r.click_url(cx);
    if twice {
        r.reload_undrawn(cx, &without_h1_h2());
        frame(cx, r.win);
    }
    r.show_quietly(cx, Section::Headers);
    assert!(
        holds_focus(cx, r.win, "url"),
        "showing Headers keeps focus on the URL"
    );
}

#[gpui_kit::test]
fn a_reload_hidden_before_its_draw_leaves_focus_where_the_user_moved_it(cx: &mut TestAppContext) {
    hidden_headers_reloads(cx, false);
}

#[gpui_kit::test]
fn a_second_reload_while_hidden_leaves_focus_where_the_user_moved_it(cx: &mut TestAppContext) {
    hidden_headers_reloads(cx, true);
}

#[gpui_kit::test]
fn showing_headers_again_without_a_reload_leaves_focus_where_the_user_moved_it(
    cx: &mut TestAppContext,
) {
    let r = three_saved_headers_with_the_last_remove_focused(cx);
    r.show_quietly(cx, Section::Body);
    r.click_url(cx);
    r.show_quietly(cx, Section::Headers);
    assert!(holds_focus(cx, r.win, "url"));
}

const PASSWORD_GRANT: &str = "info:\n  name: h\n  type: http\n  seq: 7\nhttp:\n  method: GET\n  url: http://x\n  auth:\n    type: oauth2\n    flow: resource_owner_password_credentials\n    accessTokenUrl: http://id/token\n    credentials:\n      clientId: svc\n      clientSecret: old-plain\n    resourceOwner:\n      username: u\n      password: old-pw\n";

#[gpui_kit::test]
fn moving_oauth2_secrets_with_two_uncommitted_secrets_lands_after_a_settled_reload(
    cx: &mut TestAppContext,
) {
    SectionRace {
        text: PASSWORD_GRANT,
        outside: PASSWORD_GRANT,
        section: Section::Auth,
        landed: |_, _| false,
        edit: |cx, ed, win| {
            use apinata_app::oauth2_editor::OAuth2Field;
            let oauth2 = ed.read(cx).auth().read(cx).oauth2().clone();
            // Both secrets change without the Enter or blur that would commit them.
            for (f, text) in [
                (OAuth2Field::ClientSecret, "new-plain"),
                (OAuth2Field::Password, "new-pw"),
            ] {
                let input = oauth2.read(cx).input(f).clone();
                cx.update_window(win, |_, w, cx| {
                    input.update(cx, |i, cx| i.set_value(text.to_string(), w, cx))
                })
                .unwrap();
            }
            // The button sits below the fold, so the section body scrolls it into view first.
            cx.update_window(win, |_, w, cx| {
                let body = w.find("section-body").bounds();
                let below = w.find("oauth2-move-secrets").bounds().bottom() - body.bottom();
                if below > gpui_kit::px(0.) {
                    let up = gpui_kit::point(gpui_kit::px(0.), -below - gpui_kit::px(8.));
                    w.scroll("section-body", gpui_kit::ScrollDelta::Pixels(up), cx);
                }
                w.render_frame(cx);
                w.click("oauth2-move-secrets", cx);
            })
            .unwrap();
        },
        kept: "",
        edited: "{{process.env.APINATA_OAUTH2_CLIENT_SECRET}}",
        watch: Some(watch_auth),
        held: false,
    }
    .plain(cx);
}
