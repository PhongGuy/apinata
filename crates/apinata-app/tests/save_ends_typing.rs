//! A save ends the typing run: one undo after it takes back only what was typed since.

use crate::support;

use apinata_app::request_editor::{RequestEditor, Section};
use apinata_core::AuthSpec;
use gpui_kit::component::input::InputState;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext};
use support::*;

const BEARER: &str = "info:\n  name: b\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://x/2\"\n  auth:\n    type: bearer\n    token: t\n";
// One row in every table, so each typing key has a cell to type in.
const FULL: &str = "info:\n  name: a\n  type: http\n  seq: 2\nhttp:\n  method: POST\n  url: \"http://x/0?a=1\"\n  params:\n    - name: a\n      value: \"1\"\n      type: query\n  headers:\n    - name: H1\n      value: one\n  body:\n    type: form-urlencoded\n    data:\n      - name: f\n        value: v\nruntime:\n  variables:\n    - name: token\n      value: abc\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n  actions:\n    - type: set-variable\n      phase: after-response\n      selector:\n        expression: res.body.id\n        method: jsonq\n      variable:\n        name: id\n        scope: runtime\n  scripts:\n    - type: before-request\n      code: \"let a = 1\"\n";

struct Rig {
    _col: tempfile::TempDir,
    _state: tempfile::TempDir,
    ws: Entity<apinata_app::workspace::Workspace>,
    root: std::path::PathBuf,
    win: AnyWindowHandle,
    ed: Entity<RequestEditor>,
}

fn rig(cx: &mut TestAppContext, files: &[(&str, &str)], rel: &str) -> Rig {
    support::init(cx);
    let mut tree = vec![("opencollection.yml", ROOT)];
    tree.extend_from_slice(files);
    let col = write_tree(&tree);
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

    // Focuses the input at its end, then types one keystroke per character, as a user does.
    fn focus_end(
        &self,
        cx: &mut TestAppContext,
        scope: Option<&'static str>,
        id: gpui_kit::ElementId,
    ) {
        cx.update_window(self.win, |_, w, cx| {
            match scope {
                Some(s) => w.within(s).click(id, cx),
                None => w.click(id, cx),
            }
            w.press("end", cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn keys(&self, cx: &mut TestAppContext, text: &str) {
        for ch in text.chars() {
            cx.simulate_keystrokes(self.win, &ch.to_string());
            cx.run_until_parked();
        }
    }

    fn press(&self, cx: &mut TestAppContext, keys: &str) {
        cx.simulate_keystrokes(self.win, keys);
        cx.run_until_parked();
    }

    fn save(&self, cx: &mut TestAppContext) {
        cx.update_window(self.win, |_, w, cx| {
            self.ed.update(cx, |e, cx| e.save(w, cx))
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
}

#[gpui_kit::test]
fn one_undo_in_an_auth_value_after_a_save_keeps_the_saved_text(cx: &mut TestAppContext) {
    use apinata_app::auth_section::AuthField;
    let r = rig(cx, &[("b.yml", BEARER)], "b.yml");
    r.show(cx, Section::Auth);
    r.focus_end(cx, Some("auth"), "auth-token".into());
    r.keys(cx, "new1");
    r.save(cx);
    assert!(!r.view(cx, "b.yml").dirty);
    r.keys(cx, "xyz");
    assert_eq!(
        r.view(cx, "b.yml").auth,
        AuthSpec::Bearer {
            token: "tnew1xyz".into()
        }
    );
    r.press(cx, &secondary("z"));
    let text = r.ed.read_with(cx, |e, cx| {
        e.auth()
            .read(cx)
            .input(AuthField::Token)
            .read(cx)
            .value()
            .to_string()
    });
    assert_eq!(text, "tnew1");
    assert_eq!(
        r.view(cx, "b.yml").auth,
        AuthSpec::Bearer {
            token: "tnew1".into()
        }
    );
}

#[gpui_kit::test]
fn one_undo_outside_the_auth_input_after_a_save_keeps_the_saved_text(cx: &mut TestAppContext) {
    let r = rig(cx, &[("b.yml", BEARER)], "b.yml");
    r.show(cx, Section::Auth);
    r.focus_end(cx, Some("auth"), "auth-token".into());
    r.keys(cx, "new1");
    r.save(cx);
    r.keys(cx, "xyz");
    cx.update_window(r.win, |_, w, cx| {
        r.ed.update(cx, |e, cx| e.undo_edit(w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        r.view(cx, "b.yml").auth,
        AuthSpec::Bearer {
            token: "tnew1".into()
        }
    );
}

// Types `new1` into a cell, saves, types `xyz`, and presses undo once in the cell.
fn run_across_a_save(
    cx: &mut TestAppContext,
    section: Section,
    scope: &'static str,
    id: gpui_kit::ElementId,
) -> Rig {
    let r = rig(cx, &[("a.yml", FULL)], "a.yml");
    r.show(cx, section);
    r.focus_end(cx, Some(scope), id);
    r.keys(cx, "new1");
    r.save(cx);
    assert!(!r.view(cx, "a.yml").dirty);
    r.keys(cx, "xyz");
    r.press(cx, &secondary("z"));
    r
}

#[gpui_kit::test]
fn one_undo_in_a_var_cell_after_a_save_keeps_the_saved_text(cx: &mut TestAppContext) {
    let r = run_across_a_save(cx, Section::Vars, "vars", ("kv-value", 0usize).into());
    let v = r.view(cx, "a.yml");
    assert_eq!(apinata_app::kv_table::var_rows(&v.vars)[0].value, "abcnew1");
}

#[gpui_kit::test]
fn one_undo_in_a_header_cell_after_a_save_keeps_the_saved_text(cx: &mut TestAppContext) {
    let r = run_across_a_save(cx, Section::Headers, "headers", ("kv-value", 0usize).into());
    assert_eq!(r.view(cx, "a.yml").headers[0].value, "onenew1");
}

#[gpui_kit::test]
fn one_undo_in_a_param_cell_after_a_save_keeps_the_saved_text(cx: &mut TestAppContext) {
    let r = run_across_a_save(cx, Section::Params, "params", ("kv-value", 0usize).into());
    assert_eq!(r.view(cx, "a.yml").params[0].value, "1new1");
}

#[gpui_kit::test]
fn one_undo_in_a_form_body_cell_after_a_save_keeps_the_saved_text(cx: &mut TestAppContext) {
    let r = run_across_a_save(cx, Section::Body, "body", ("kv-value", 0usize).into());
    match &r.view(cx, "a.yml").body {
        apinata_core::BodySpec::FormUrlEncoded(kv) => assert_eq!(kv[0].value, "vnew1"),
        other => panic!("{other:?}"),
    }
}

#[gpui_kit::test]
fn one_undo_in_an_assertion_after_a_save_keeps_the_saved_text(cx: &mut TestAppContext) {
    let r = run_across_a_save(
        cx,
        Section::Assertions,
        "assertions",
        ("as-value", 0usize).into(),
    );
    assert_eq!(r.view(cx, "a.yml").assertions[0].value(), Some("200new1"));
}

#[gpui_kit::test]
fn one_undo_in_a_post_response_var_after_a_save_keeps_the_saved_text(cx: &mut TestAppContext) {
    let r = run_across_a_save(
        cx,
        Section::Vars,
        "post-response-vars",
        ("kv-value", 0usize).into(),
    );
    assert_eq!(
        r.view(cx, "a.yml").post_response_vars[0].expression(),
        "res.body.idnew1"
    );
}

#[gpui_kit::test]
fn one_undo_in_the_url_after_a_save_keeps_the_saved_text(cx: &mut TestAppContext) {
    let r = rig(cx, &[("a.yml", FULL)], "a.yml");
    cx.update_window(r.win, |_, w, cx| {
        w.click("url", cx);
        w.press("end", cx);
    })
    .unwrap();
    cx.run_until_parked();
    r.keys(cx, "new1");
    r.save(cx);
    r.keys(cx, "xyz");
    r.press(cx, &secondary("z"));
    assert_eq!(r.view(cx, "a.yml").url, "http://x/0?a=1new1");
}

#[gpui_kit::test]
fn one_undo_in_a_script_after_a_save_keeps_the_saved_text(cx: &mut TestAppContext) {
    let r = rig(cx, &[("a.yml", FULL)], "a.yml");
    r.show(cx, Section::Script);
    cx.update_window(r.win, |_, w, cx| {
        w.click_at("script-pre-request-code", code_at(), cx);
        w.press("end", cx);
    })
    .unwrap();
    cx.run_until_parked();
    r.keys(cx, "new1");
    r.save(cx);
    r.keys(cx, "xyz");
    r.press(cx, &secondary("z"));
    assert_eq!(r.view(cx, "a.yml").scripts.pre_request, "let a = 1new1");
}

// The request opened with the real watcher, so a write behind the app's back reloads it.
fn watched_rig(cx: &mut TestAppContext, rel: &str, text: &str) -> Rig {
    support::init(cx);
    let col = write_tree(&[("opencollection.yml", ROOT), (rel, text)]);
    let w = watched(cx, &col);
    let (win, ed) = editor_for(cx, &w.ws, &w.root, rel, false);
    Rig {
        _col: col,
        _state: w.state,
        ws: w.ws,
        root: w.root,
        win,
        ed,
    }
}

impl Rig {
    // Writes `text` to `rel` behind the app's back and waits until the view shows `url`.
    fn outside_write(&self, cx: &mut TestAppContext, rel: &str, text: &str) {
        settle(cx);
        std::fs::write(self.root.join(rel), text).unwrap();
        let before = self.view(cx, rel).reloaded;
        wait_for(cx, "the reload", |cx| self.view(cx, rel).reloaded != before);
        frame(cx, self.win);
    }
}

#[gpui_kit::test]
fn a_reload_after_a_save_that_keeps_the_typed_value_still_ends_the_run(cx: &mut TestAppContext) {
    let r = watched_rig(cx, "b.yml", BEARER);
    r.show(cx, Section::Auth);
    r.focus_end(cx, Some("auth"), "auth-token".into());
    r.keys(cx, "new1");
    r.save(cx);
    let outside = BEARER
        .replace("token: t", "token: tnew1")
        .replace("x/2", "x/3");
    r.outside_write(cx, "b.yml", &outside);
    assert_eq!(r.view(cx, "b.yml").url, "http://x/3");
    r.keys(cx, "xyz");
    r.press(cx, &secondary("z"));
    assert_eq!(
        r.view(cx, "b.yml").auth,
        AuthSpec::Bearer {
            token: "tnew1".into()
        }
    );
}

#[gpui_kit::test]
fn a_reload_under_unsaved_typing_ends_the_run(cx: &mut TestAppContext) {
    let r = watched_rig(cx, "b.yml", BEARER);
    r.show(cx, Section::Auth);
    r.focus_end(cx, Some("auth"), "auth-token".into());
    r.keys(cx, "new1");
    assert!(r.view(cx, "b.yml").dirty);
    r.outside_write(cx, "b.yml", &BEARER.replace("x/2", "x/3"));
    r.keys(cx, "xyz");
    r.press(cx, &secondary("z"));
    assert_eq!(
        r.view(cx, "b.yml").auth,
        AuthSpec::Bearer {
            token: "tnew1".into()
        }
    );
}

mod config {
    use super::*;
    use apinata_app::config_editor::{ConfigEditor, ConfigSection, ConfigTarget};

    const FOLDER: &str = "info:\n  name: Users\n  type: folder\n  seq: 1\nrequest:\n  auth:\n    type: bearer\n    token: t\n  headers:\n    - name: h1\n      value: one\n  variables:\n    - name: v\n      value: abc\n";

    struct Cfg {
        _col: tempfile::TempDir,
        _state: tempfile::TempDir,
        ws: Entity<apinata_app::workspace::Workspace>,
        root: std::path::PathBuf,
        win: AnyWindowHandle,
        ed: Entity<ConfigEditor>,
    }

    fn cfg(cx: &mut TestAppContext, section: ConfigSection) -> Cfg {
        support::init(cx);
        cx.update(apinata_app::keymap::init);
        let col = write_tree(&[("opencollection.yml", ROOT), ("users/folder.yml", FOLDER)]);
        let (state, ws, root) = open_dir(cx, &col);
        let (ws2, r) = (ws.clone(), root.clone());
        let (win, ed) = cx
            .update(|cx| {
                gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                    let t = ConfigTarget::Folder(ip("users"));
                    cx.new(|cx| ConfigEditor::new(ws2.clone(), r.clone(), t, w, cx))
                })
            })
            .unwrap();
        cx.run_until_parked();
        ed.update(cx, |e, cx| e.set_section(section, cx));
        frame(cx, win);
        Cfg {
            _col: col,
            _state: state,
            ws,
            root,
            win,
            ed,
        }
    }

    impl Cfg {
        fn view(
            &self,
            cx: &mut TestAppContext,
        ) -> std::sync::Arc<apinata_app::snapshot::ConfigView> {
            let snap = self
                .ws
                .read_with(cx, |w, cx| w.snapshot(&self.root, cx))
                .unwrap();
            snap.content.folders[&ip("users")].clone()
        }

        fn keys(&self, cx: &mut TestAppContext, text: &str) {
            for ch in text.chars() {
                cx.simulate_keystrokes(self.win, &ch.to_string());
                cx.run_until_parked();
            }
        }

        // Types `new1` at the end of the input, saves, types `xyz`, and presses undo once.
        fn run_across_a_save(
            &self,
            cx: &mut TestAppContext,
            scope: Option<&'static str>,
            id: gpui_kit::ElementId,
        ) {
            cx.update_window(self.win, |_, w, cx| {
                match scope {
                    Some(s) => w.within(s).click(id, cx),
                    None => w.click(id, cx),
                }
                w.press("end", cx);
            })
            .unwrap();
            cx.run_until_parked();
            self.keys(cx, "new1");
            self.ed.update(cx, |e, cx| e.save(cx));
            cx.run_until_parked();
            assert!(!self.view(cx).dirty);
            self.keys(cx, "xyz");
            cx.simulate_keystrokes(self.win, &secondary("z"));
            cx.run_until_parked();
        }
    }

    #[gpui_kit::test]
    fn one_undo_in_a_folder_auth_value_after_a_save_keeps_the_saved_text(cx: &mut TestAppContext) {
        let c = cfg(cx, ConfigSection::Auth);
        c.run_across_a_save(cx, None, "auth-token".into());
        assert_eq!(
            c.view(cx).auth,
            AuthSpec::Bearer {
                token: "tnew1".into()
            }
        );
    }

    #[gpui_kit::test]
    fn one_undo_in_a_folder_header_after_a_save_keeps_the_saved_text(cx: &mut TestAppContext) {
        let c = cfg(cx, ConfigSection::Headers);
        c.run_across_a_save(cx, Some("config-headers"), ("kv-value", 0usize).into());
        assert_eq!(c.view(cx).headers[0].value, "onenew1");
    }

    #[gpui_kit::test]
    fn one_undo_in_a_folder_var_after_a_save_keeps_the_saved_text(cx: &mut TestAppContext) {
        let c = cfg(cx, ConfigSection::Vars);
        c.run_across_a_save(cx, Some("config-vars"), ("kv-value", 0usize).into());
        let v = c.view(cx);
        assert_eq!(apinata_app::kv_table::var_rows(&v.vars)[0].value, "abcnew1");
    }

    #[gpui_kit::test]
    fn a_save_leaves_an_open_composition_in_a_folder_auth_value_alone(cx: &mut TestAppContext) {
        use apinata_app::auth_section::AuthField;
        let c = cfg(cx, ConfigSection::Auth);
        cx.update_window(c.win, |_, w, cx| {
            w.click("auth-token", cx);
            w.press("end", cx);
        })
        .unwrap();
        cx.run_until_parked();
        c.keys(cx, "new1");
        let input = c.ed.read_with(cx, |e, cx| {
            e.auth().read(cx).input(AuthField::Token).clone()
        });
        let [before, after] = compose_across_a_save(cx, c.win, &input, |cx| {
            c.ed.update(cx, |e, cx| e.save(cx));
            cx.run_until_parked();
        });
        assert_eq!(before.0, "tnew1かな");
        assert_eq!(after, before);
        commit(cx, c.win, &input);
        assert_eq!(
            c.view(cx).auth,
            AuthSpec::Bearer {
                token: "tnew1かな".into()
            }
        );
    }

    const COLLECTION: &str = "opencollection: 1.0.0\ninfo:\n  name: c\nrequest:\n  auth:\n    type: bearer\n    token: t\n  headers:\n    - name: h1\n      value: one\n";

    // The settings opened with the real watcher, so a write behind the app's back reloads them.
    fn watched_cfg(cx: &mut TestAppContext, target: ConfigTarget) -> (Cfg, ConfigTarget) {
        support::init(cx);
        cx.update(apinata_app::keymap::init);
        let col = write_tree(&[
            ("opencollection.yml", COLLECTION),
            ("users/folder.yml", FOLDER),
        ]);
        let w = watched(cx, &col);
        let (ws2, r, t) = (w.ws.clone(), w.root.clone(), target.clone());
        let (win, ed) = cx
            .update(|cx| {
                gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                    cx.new(|cx| ConfigEditor::new(ws2.clone(), r.clone(), t.clone(), w, cx))
                })
            })
            .unwrap();
        cx.run_until_parked();
        ed.update(cx, |e, cx| e.set_section(ConfigSection::Auth, cx));
        frame(cx, win);
        let c = Cfg {
            _col: col,
            _state: w.state,
            ws: w.ws,
            root: w.root,
            win,
            ed,
        };
        (c, target)
    }

    impl Cfg {
        fn target_view(
            &self,
            cx: &mut TestAppContext,
            target: &ConfigTarget,
        ) -> std::sync::Arc<apinata_app::snapshot::ConfigView> {
            let snap = self
                .ws
                .read_with(cx, |w, cx| w.snapshot(&self.root, cx))
                .unwrap();
            match target {
                ConfigTarget::Folder(p) => snap.content.folders[p].clone(),
                ConfigTarget::Collection => snap.content.collection.clone(),
            }
        }

        fn token_text(&self, cx: &mut TestAppContext) -> String {
            use apinata_app::auth_section::AuthField;
            self.ed.read_with(cx, |e, cx| {
                let input = e.auth().read(cx).input(AuthField::Token).clone();
                input.read(cx).value().to_string()
            })
        }
    }

    fn file_of(target: &ConfigTarget) -> (&'static str, &'static str) {
        match target {
            ConfigTarget::Folder(_) => ("users/folder.yml", FOLDER),
            ConfigTarget::Collection => ("opencollection.yml", COLLECTION),
        }
    }

    // Types into the token, saves, has the file's token replaced on disk, types again and
    // undoes once: the undo stops at the reloaded token, not the saved one.
    fn reload_after_a_save_ends_the_run(cx: &mut TestAppContext, target: ConfigTarget) {
        let (c, target) = watched_cfg(cx, target);
        let (rel, text) = file_of(&target);
        cx.update_window(c.win, |_, w, cx| {
            w.click("auth-token", cx);
            w.press("end", cx);
        })
        .unwrap();
        cx.run_until_parked();
        c.keys(cx, "new1");
        c.ed.update(cx, |e, cx| e.save(cx));
        cx.run_until_parked();
        assert!(!c.target_view(cx, &target).dirty);
        settle(cx);
        std::fs::write(c.root.join(rel), text.replace("token: t", "token: out")).unwrap();
        let out = AuthSpec::Bearer {
            token: "out".into(),
        };
        wait_for(cx, "the reload", |cx| {
            c.target_view(cx, &target).auth == out
        });
        frame(cx, c.win);
        assert_eq!(c.token_text(cx), "out");
        c.keys(cx, "xyz");
        cx.simulate_keystrokes(c.win, &secondary("z"));
        cx.run_until_parked();
        assert_eq!(c.token_text(cx), "out");
        assert_eq!(c.target_view(cx, &target).auth, out);
    }

    #[gpui_kit::test]
    fn a_folder_reload_after_a_save_ends_the_run(cx: &mut TestAppContext) {
        reload_after_a_save_ends_the_run(cx, ConfigTarget::Folder(ip("users")));
    }

    #[gpui_kit::test]
    fn a_collection_reload_after_a_save_ends_the_run(cx: &mut TestAppContext) {
        reload_after_a_save_ends_the_run(cx, ConfigTarget::Collection);
    }

    // Unsaved typing survives an outside write as a conflict; the reload still ends its run.
    fn reload_under_unsaved_typing_ends_the_run(cx: &mut TestAppContext, target: ConfigTarget) {
        let (c, target) = watched_cfg(cx, target);
        let (rel, text) = file_of(&target);
        cx.update_window(c.win, |_, w, cx| {
            w.click("auth-token", cx);
            w.press("end", cx);
        })
        .unwrap();
        cx.run_until_parked();
        c.keys(cx, "new1");
        assert!(c.target_view(cx, &target).dirty);
        settle(cx);
        std::fs::write(c.root.join(rel), text.replace("value: one", "value: two")).unwrap();
        wait_for(cx, "the reload", |cx| {
            c.target_view(cx, &target).disk == apinata_app::snapshot::DiskState::Changed
        });
        frame(cx, c.win);
        c.keys(cx, "xyz");
        cx.simulate_keystrokes(c.win, &secondary("z"));
        cx.run_until_parked();
        assert_eq!(c.token_text(cx), "tnew1");
        assert_eq!(
            c.target_view(cx, &target).auth,
            AuthSpec::Bearer {
                token: "tnew1".into()
            }
        );
    }

    #[gpui_kit::test]
    fn a_folder_reload_under_unsaved_typing_ends_the_run(cx: &mut TestAppContext) {
        reload_under_unsaved_typing_ends_the_run(cx, ConfigTarget::Folder(ip("users")));
    }

    #[gpui_kit::test]
    fn a_collection_reload_under_unsaved_typing_ends_the_run(cx: &mut TestAppContext) {
        reload_under_unsaved_typing_ends_the_run(cx, ConfigTarget::Collection);
    }
}

#[gpui_kit::test]
fn one_undo_in_an_environment_value_after_a_save_keeps_the_saved_text(cx: &mut TestAppContext) {
    use apinata_core::VarValue;
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: host\n    value: x\n",
        ),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.update_window(win, |_, w, cx| {
        w.click(("env-value", 0usize), cx);
        w.press("end", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let keys = |cx: &mut TestAppContext, text: &str| {
        for ch in text.chars() {
            cx.simulate_keystrokes(win, &ch.to_string());
            cx.run_until_parked();
        }
    };
    keys(cx, "new1");
    ed.update(cx, |e, cx| e.save(cx));
    cx.run_until_parked();
    assert!(!env_view(cx, &ws, &root, "dev").dirty);
    keys(cx, "xyz");
    cx.simulate_keystrokes(win, &secondary("z"));
    cx.run_until_parked();
    match &env_view(cx, &ws, &root, "dev").variables[0].value {
        VarValue::Text(t) => assert_eq!(t, "xnew1"),
        _ => panic!("not a text value"),
    }
}

const TEXT_BODY: &str = "info:\n  name: t\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: \"http://x/t\"\n  body:\n    type: text\n    data: abc\nsettings:\n  encodeUrl: true\n  timeout: 0\n  followRedirects: true\n  maxRedirects: 5\n";
const GRAPHQL: &str = "info:\n  name: g\n  type: graphql\n  seq: 1\ngraphql:\n  method: POST\n  url: \"http://x/g\"\n  body:\n    query: \"{ users }\"\n    variables: \"{}\"\n";

impl Rig {
    // Clicks into the code editor `id` (inside `scope`), types `new1`, saves, types `xyz` and
    // presses undo once in the editor.
    fn code_run_across_a_save(
        &self,
        cx: &mut TestAppContext,
        rel: &str,
        scope: &'static str,
        id: &'static str,
    ) {
        cx.update_window(self.win, |_, w, cx| {
            w.within(scope).click_at(id, code_at(), cx);
            w.press("end", cx);
        })
        .unwrap();
        cx.run_until_parked();
        self.keys(cx, "new1");
        self.save(cx);
        assert!(!self.view(cx, rel).dirty);
        self.keys(cx, "xyz");
        self.press(cx, &secondary("z"));
    }
}

#[gpui_kit::test]
fn one_undo_in_a_text_body_after_a_save_keeps_the_saved_text(cx: &mut TestAppContext) {
    let r = rig(cx, &[("t.yml", TEXT_BODY)], "t.yml");
    r.show(cx, Section::Body);
    r.code_run_across_a_save(cx, "t.yml", "body", "body-code");
    let shown = r.ed.read_with(cx, |e, cx| {
        e.body().read(cx).code().read(cx).value().to_string()
    });
    assert_eq!(shown, "abcnew1");
    assert_eq!(
        r.view(cx, "t.yml").body,
        apinata_core::BodySpec::Text("abcnew1".into())
    );
}

// The GraphQL query (or variables) text the editor shows and the model holds.
fn graphql_texts(r: &Rig, cx: &mut TestAppContext, variables: bool) -> (String, String) {
    let shown = r.ed.read_with(cx, |e, cx| {
        let g = e.graphql().read(cx);
        let ed = if variables { g.variables() } else { g.query() };
        ed.read(cx).value().to_string()
    });
    match r.view(cx, "g.yml").body.clone() {
        apinata_core::BodySpec::Graphql {
            query,
            variables: v,
        } => (shown, if variables { v } else { query }),
        other => panic!("{other:?}"),
    }
}

#[gpui_kit::test]
fn one_undo_in_a_graphql_query_after_a_save_keeps_the_saved_text(cx: &mut TestAppContext) {
    let r = rig(cx, &[("g.yml", GRAPHQL)], "g.yml");
    r.show(cx, Section::Body);
    r.code_run_across_a_save(cx, "g.yml", "body", "graphql-query");
    let want = "{ users }new1".to_string();
    assert_eq!(graphql_texts(&r, cx, false), (want.clone(), want));
}

#[gpui_kit::test]
fn one_undo_in_graphql_variables_after_a_save_keeps_the_saved_text(cx: &mut TestAppContext) {
    let r = rig(cx, &[("g.yml", GRAPHQL)], "g.yml");
    r.show(cx, Section::Body);
    r.code_run_across_a_save(cx, "g.yml", "body", "graphql-variables");
    let want = "{}new1".to_string();
    assert_eq!(graphql_texts(&r, cx, true), (want.clone(), want));
}

#[gpui_kit::test]
fn one_undo_in_the_docs_after_a_save_keeps_the_saved_text(cx: &mut TestAppContext) {
    let r = rig(cx, &[("t.yml", TEXT_BODY)], "t.yml");
    r.show(cx, Section::Docs);
    r.code_run_across_a_save(cx, "t.yml", "docs", "docs-code");
    let shown = r.ed.read_with(cx, |e, cx| {
        e.docs().read(cx).code().read(cx).value().to_string()
    });
    assert_eq!(shown, "new1");
    assert_eq!(r.view(cx, "t.yml").docs, "new1");
}

#[gpui_kit::test]
fn one_undo_in_a_settings_input_after_a_save_keeps_the_saved_text(cx: &mut TestAppContext) {
    let r = rig(cx, &[("t.yml", TEXT_BODY)], "t.yml");
    r.show(cx, Section::Settings);
    r.focus_end(cx, Some("settings"), "set-max-redirects".into());
    r.keys(cx, "1");
    r.save(cx);
    assert!(!r.view(cx, "t.yml").dirty);
    r.keys(cx, "2");
    r.press(cx, &secondary("z"));
    let shown = cx
        .update_window(r.win, |_, w, cx| {
            w.render_frame(cx);
            w.find("set-max-redirects")
                .value()
                .unwrap_or_default()
                .to_owned()
        })
        .unwrap();
    assert_eq!(shown, "51");
    assert_eq!(r.view(cx, "t.yml").settings.max_redirects, 51);
}

#[gpui_kit::test]
fn one_undo_in_the_timeout_after_a_save_keeps_the_saved_text(cx: &mut TestAppContext) {
    let text = TEXT_BODY.replace("timeout: 0", "timeout: 5000");
    let r = rig(cx, &[("t.yml", text.as_str())], "t.yml");
    r.show(cx, Section::Settings);
    r.focus_end(cx, Some("settings"), "set-timeout".into());
    r.keys(cx, "1");
    r.save(cx);
    assert!(!r.view(cx, "t.yml").dirty);
    r.keys(cx, "2");
    r.press(cx, &secondary("z"));
    assert_eq!(
        r.view(cx, "t.yml").settings.timeout,
        apinata_core::Timeout::Millis(50001)
    );
}

#[gpui_kit::test]
fn one_undo_in_an_environment_name_after_a_save_keeps_the_saved_text(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: host\n    value: x\n",
        ),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.update_window(win, |_, w, cx| {
        w.click(("env-name", 0usize), cx);
        w.press("end", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let keys = |cx: &mut TestAppContext, text: &str| {
        for ch in text.chars() {
            cx.simulate_keystrokes(win, &ch.to_string());
            cx.run_until_parked();
        }
    };
    keys(cx, "new1");
    ed.update(cx, |e, cx| e.save(cx));
    cx.run_until_parked();
    assert!(!env_view(cx, &ws, &root, "dev").dirty);
    keys(cx, "xyz");
    cx.simulate_keystrokes(win, &secondary("z"));
    cx.run_until_parked();
    assert_eq!(
        env_view(cx, &ws, &root, "dev").variables[0].name,
        "hostnew1"
    );
}

// Opens a two-character preedit in `input` with the IME caret between them, as many IMEs place
// it, ends the typing run, and returns the (marked range, selection) before and after.
fn compose_then_end<S: gpui_kit::EntityInputHandler>(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    input: &Entity<S>,
    end: impl FnOnce(&Entity<S>, &mut gpui_kit::Window, &mut gpui_kit::App),
    selected: fn(&S) -> std::ops::Range<usize>,
) -> [(Option<std::ops::Range<usize>>, std::ops::Range<usize>); 2] {
    let state = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, cx| {
            input.update(cx, |s, cx| (s.marked_text_range(w, cx), selected(s)))
        })
        .unwrap()
    };
    cx.update_window(win, |_, w, cx| {
        input.update(cx, |s, cx| {
            s.replace_and_mark_text_in_range(None, "ka", Some(1..1), w, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    let before = state(cx);
    cx.update_window(win, |_, w, cx| end(input, w, cx)).unwrap();
    cx.run_until_parked();
    [before, state(cx)]
}

#[gpui_kit::test]
fn ending_an_input_run_leaves_an_open_composition_and_its_caret_alone(cx: &mut TestAppContext) {
    use apinata_app::auth_section::AuthField;
    use gpui_kit::EntityInputHandler as _;
    let r = rig(cx, &[("b.yml", BEARER)], "b.yml");
    r.show(cx, Section::Auth);
    r.focus_end(cx, Some("auth"), "auth-token".into());
    r.keys(cx, "new1");
    let input = r.ed.read_with(cx, |e, cx| {
        e.auth().read(cx).input(AuthField::Token).clone()
    });
    let [before, after] =
        compose_then_end(cx, r.win, &input, apinata_app::ui::end_input_run, |s| {
            s.selected_range()
        });
    assert_eq!(before, (Some(5..7), 6..6));
    assert_eq!(after, before);
    cx.update_window(r.win, |_, w, cx| {
        input.update(cx, |s, cx| s.replace_text_in_range(None, "か", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(input.read_with(cx, |s, _| s.value().to_string()), "tnew1か");
    assert_eq!(
        r.view(cx, "b.yml").auth,
        AuthSpec::Bearer {
            token: "tnew1か".into()
        }
    );
}

#[gpui_kit::test]
fn ending_an_editor_run_leaves_an_open_composition_and_its_caret_alone(cx: &mut TestAppContext) {
    use gpui_kit::EntityInputHandler as _;
    let r = rig(cx, &[("t.yml", TEXT_BODY)], "t.yml");
    r.show(cx, Section::Body);
    cx.update_window(r.win, |_, w, cx| {
        w.within("body").click_at("body-code", code_at(), cx);
        w.press("end", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let code = r.ed.read_with(cx, |e, cx| e.body().read(cx).code().clone());
    let [before, after] =
        compose_then_end(cx, r.win, &code, apinata_app::ui::end_editor_run, |s| {
            s.selected_range()
        });
    assert_eq!(before, (Some(3..5), 4..4));
    assert_eq!(after, before);
    cx.update_window(r.win, |_, w, cx| {
        code.update(cx, |s, cx| s.replace_text_in_range(None, "か", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(code.read_with(cx, |s, _| s.value().to_string()), "abcか");
    assert_eq!(
        r.view(cx, "t.yml").body,
        apinata_core::BodySpec::Text("abcか".into())
    );
}

// A field's text, IME marked range (UTF-16) and selection.
type Field = (
    String,
    Option<std::ops::Range<usize>>,
    std::ops::Range<usize>,
);

fn field(cx: &mut TestAppContext, win: AnyWindowHandle, input: &Entity<InputState>) -> Field {
    use gpui_kit::EntityInputHandler as _;
    cx.update_window(win, |_, w, cx| {
        input.update(cx, |s, cx| {
            (
                s.value().to_string(),
                s.marked_text_range(w, cx),
                s.selected_range(),
            )
        })
    })
    .unwrap()
}

// Opens a non-ASCII two-character preedit at the caret of `input`, with the IME caret between
// them, runs `save`, and returns the field before and after it.
fn compose_across_a_save(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    input: &Entity<InputState>,
    save: impl FnOnce(&mut TestAppContext),
) -> [Field; 2] {
    use gpui_kit::EntityInputHandler as _;
    cx.update_window(win, |_, w, cx| {
        input.update(cx, |s, cx| {
            s.replace_and_mark_text_in_range(None, "かな", Some(1..1), w, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    let before = field(cx, win, input);
    assert!(before.1.is_some(), "the composition opened");
    save(cx);
    [before, field(cx, win, input)]
}

// Commits the open composition as it reads.
fn commit(cx: &mut TestAppContext, win: AnyWindowHandle, input: &Entity<InputState>) {
    use gpui_kit::EntityInputHandler as _;
    cx.update_window(win, |_, w, cx| {
        input.update(cx, |s, cx| s.replace_text_in_range(None, "かな", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_save_leaves_an_open_composition_in_an_auth_value_alone(cx: &mut TestAppContext) {
    use apinata_app::auth_section::AuthField;
    let r = rig(cx, &[("b.yml", BEARER)], "b.yml");
    r.show(cx, Section::Auth);
    r.focus_end(cx, Some("auth"), "auth-token".into());
    r.keys(cx, "new1");
    let input = r.ed.read_with(cx, |e, cx| {
        e.auth().read(cx).input(AuthField::Token).clone()
    });
    let [before, after] = compose_across_a_save(cx, r.win, &input, |cx| r.save(cx));
    assert_eq!(before.0, "tnew1かな");
    assert_eq!(after, before);
    commit(cx, r.win, &input);
    assert_eq!(
        input.read_with(cx, |s, _| s.value().to_string()),
        "tnew1かな"
    );
    assert_eq!(
        r.view(cx, "b.yml").auth,
        AuthSpec::Bearer {
            token: "tnew1かな".into()
        }
    );
    r.save(cx);
    assert!(!r.view(cx, "b.yml").dirty);
}

#[gpui_kit::test]
fn a_save_leaves_an_open_composition_in_the_url_alone(cx: &mut TestAppContext) {
    let r = rig(cx, &[("b.yml", BEARER)], "b.yml");
    cx.update_window(r.win, |_, w, cx| {
        w.click("url", cx);
        w.press("end", cx);
    })
    .unwrap();
    cx.run_until_parked();
    r.keys(cx, "new1");
    let input = r.ed.read_with(cx, |e, _| e.url_input().clone());
    let [before, after] = compose_across_a_save(cx, r.win, &input, |cx| r.save(cx));
    assert_eq!(before.0, "http://x/2new1かな");
    assert_eq!(after, before);
    commit(cx, r.win, &input);
    assert_eq!(r.view(cx, "b.yml").url, "http://x/2new1かな");
}

#[gpui_kit::test]
fn a_save_leaves_an_open_composition_in_an_environment_value_alone(cx: &mut TestAppContext) {
    use apinata_core::VarValue;
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: host\n    value: x\n",
        ),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.update_window(win, |_, w, cx| {
        w.click(("env-value", 0usize), cx);
        w.press("end", cx);
    })
    .unwrap();
    cx.run_until_parked();
    for ch in "new1".chars() {
        cx.simulate_keystrokes(win, &ch.to_string());
        cx.run_until_parked();
    }
    let input = ed.read_with(cx, |e, cx| {
        e.table().read(cx).value_input(0).unwrap().clone()
    });
    let [before, after] = compose_across_a_save(cx, win, &input, |cx| {
        ed.update(cx, |e, cx| e.save(cx));
        cx.run_until_parked();
    });
    assert_eq!(before.0, "xnew1かな");
    assert_eq!(after, before);
    commit(cx, win, &input);
    match &env_view(cx, &ws, &root, "dev").variables[0].value {
        VarValue::Text(t) => assert_eq!(t, "xnew1かな"),
        _ => panic!("not a text value"),
    }
}

#[test]
fn the_committed_text_cuts_the_composition_by_utf16_units() {
    use apinata_app::ui::committed_text;
    assert_eq!(committed_text("é𝄞かab", Some(3..4)), "é𝄞ab");
    assert_eq!(committed_text("é𝄞かab", Some(1..3)), "éかab");
    assert_eq!(committed_text("abc", None), "abc");
    assert_eq!(committed_text("abc", Some(2..9)), "abc");
}
