//! The undo and redo keys of a request tab: which focus sends them to the structured
//! history and which leaves them to the text input's own undo.

use crate::support;

use apinata_app::keymap::{RedoActive, UndoActive};
use apinata_app::request_editor::{RequestEditor, Section};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext};
use support::*;

const FILE: &str = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://x/0\"\n  headers:\n    - name: H1\n      value: one\nruntime:\n  variables:\n    - name: token\n      value: abc\n";

struct Rig {
    _col: tempfile::TempDir,
    _state: tempfile::TempDir,
    ws: Entity<apinata_app::workspace::Workspace>,
    root: std::path::PathBuf,
    win: AnyWindowHandle,
    _ed: Entity<RequestEditor>,
}

fn rig(cx: &mut TestAppContext) -> Rig {
    support::init(cx);
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", FILE)]);
    let (state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    Rig {
        _col: col,
        _state: state,
        ws,
        root,
        win,
        _ed: ed,
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

    // Three keystrokes into the first variable's value: one coalesced step.
    fn type_var(&self, cx: &mut TestAppContext) {
        self.show(cx, Section::Vars);
        for ch in ["x", "y", "z"] {
            type_at_end(cx, self.win, Some("vars"), ("kv-value", 0usize), ch);
            cx.run_until_parked();
        }
        assert_eq!(self.var(cx), "abcxyz");
    }

    fn var(&self, cx: &mut TestAppContext) -> String {
        let v = view_of(cx, &self.ws, &self.root, "a.yml");
        apinata_app::kv_table::var_rows(&v.vars).remove(0).value
    }

    fn header_enabled(&self, cx: &mut TestAppContext) -> bool {
        view_of(cx, &self.ws, &self.root, "a.yml").headers[0].enabled
    }

    fn press(&self, cx: &mut TestAppContext, keys: &str) {
        cx.simulate_keystrokes(self.win, keys);
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn undo_in_a_table_cell_goes_back_one_coalesced_step(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.type_var(cx);
    r.press(cx, &secondary("z"));
    assert_eq!(r.var(cx), "abc");
}

#[gpui_kit::test]
fn redo_in_a_table_cell_on_ctrl_shift_z(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.type_var(cx);
    r.press(cx, &secondary("z"));
    assert_eq!(r.var(cx), "abc");
    r.press(cx, &secondary("shift-z"));
    assert_eq!(r.var(cx), "abcxyz");
}

#[cfg(not(target_os = "macos"))]
#[gpui_kit::test]
fn redo_in_a_table_cell_on_ctrl_y(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.type_var(cx);
    r.press(cx, "ctrl-z");
    assert_eq!(r.var(cx), "abc");
    r.press(cx, "ctrl-y");
    assert_eq!(r.var(cx), "abcxyz");
}

#[gpui_kit::test]
fn undo_in_the_url_input_leaves_the_structured_history(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.type_var(cx);
    cx.update_window(r.win, |_, w, cx| w.click("url", cx))
        .unwrap();
    r.press(cx, &secondary("z"));
    assert_eq!(r.var(cx), "abcxyz");
}

#[gpui_kit::test]
fn undo_with_a_toggle_focused_undoes(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.show(cx, Section::Headers);
    cx.update_window(r.win, |_, w, cx| {
        w.within("headers").click(("kv-enabled", 0usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!r.header_enabled(cx));
    r.press(cx, &secondary("z"));
    assert!(r.header_enabled(cx));
    r.press(cx, &secondary("shift-z"));
    assert!(!r.header_enabled(cx));
}

#[gpui_kit::test]
fn undo_with_an_empty_history_does_nothing(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.show(cx, Section::Vars);
    cx.update_window(r.win, |_, w, cx| {
        w.within("vars").click(("kv-value", 0usize), cx)
    })
    .unwrap();
    r.press(cx, &secondary("z"));
    r.press(cx, &secondary("shift-z"));
    assert_eq!(r.var(cx), "abc");
    assert!(!view_of(cx, &r.ws, &r.root, "a.yml").dirty);
}

// The palette dispatches from the window, so the commands act on the active request tab.
#[gpui_kit::test]
fn the_palette_commands_undo_and_redo_the_active_tab(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", FILE)]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.click(("section", Section::Headers as usize), cx);
    })
    .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.within("headers").click(("kv-enabled", 0usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    let enabled = |cx: &mut TestAppContext| view_of(cx, &s.ws, &s.root, "a.yml").headers[0].enabled;
    assert!(!enabled(cx));
    cx.dispatch_action(s.win, UndoActive);
    cx.run_until_parked();
    assert!(enabled(cx));
    cx.dispatch_action(s.win, RedoActive);
    cx.run_until_parked();
    assert!(!enabled(cx));
}

const JSON_FILE: &str = "info:\n  name: j\n  type: http\n  seq: 2\nhttp:\n  method: POST\n  url: \"http://x/1\"\n  headers:\n    - name: H1\n      value: one\n  body:\n    type: json\n    data: '{}'\n";

// The whole app, so the Shell context binding is live. A header toggle is the structured step
// that text undo in an input must leave alone.
fn full_app(cx: &mut TestAppContext) -> (Started, tempfile::TempDir) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("j.yml", JSON_FILE)]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "j.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.click(("section", Section::Headers as usize), cx);
    })
    .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.within("headers").click(("kv-enabled", 0usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    (s, col)
}

fn j_view(
    cx: &mut TestAppContext,
    s: &Started,
) -> std::sync::Arc<apinata_app::snapshot::RequestView> {
    view_of(cx, &s.ws, &s.root, "j.yml")
}

fn keys(cx: &mut TestAppContext, s: &Started, chord: &str) {
    cx.simulate_keystrokes(s.win, chord);
    cx.run_until_parked();
}

#[gpui_kit::test]
fn undo_and_redo_in_the_url_input_act_on_the_text(cx: &mut TestAppContext) {
    let (s, _col) = full_app(cx);
    type_at_end(cx, s.win, None, "url", "zz");
    cx.run_until_parked();
    assert_eq!(j_view(cx, &s).url, "http://x/1zz");
    keys(cx, &s, &secondary("z"));
    assert_eq!(j_view(cx, &s).url, "http://x/1");
    assert!(!j_view(cx, &s).headers[0].enabled, "the header step stays");
    keys(cx, &s, &secondary("shift-z"));
    assert_eq!(j_view(cx, &s).url, "http://x/1zz");
    keys(cx, &s, &secondary("z"));
    assert_eq!(j_view(cx, &s).url, "http://x/1");
    #[cfg(not(target_os = "macos"))]
    {
        keys(cx, &s, "ctrl-y");
        assert_eq!(j_view(cx, &s).url, "http://x/1zz");
    }
    assert!(!j_view(cx, &s).headers[0].enabled, "the header step stays");
}

#[gpui_kit::test]
fn undo_and_redo_in_a_code_editor_act_on_the_text(cx: &mut TestAppContext) {
    let (s, _col) = full_app(cx);
    cx.update_window(s.win, |_, w, cx| {
        w.click(("section", Section::Body as usize), cx);
    })
    .unwrap();
    frame(cx, s.win);
    replace_code(cx, s.win, Some("body"), "body-code", "[1]");
    cx.run_until_parked();
    let text = |cx: &mut TestAppContext| match &j_view(cx, &s).body {
        apinata_core::BodySpec::Json(t) => t.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(text(cx), "[1]");
    keys(cx, &s, &secondary("z"));
    assert_ne!(text(cx), "[1]", "the editor's own undo ran");
    assert!(!j_view(cx, &s).headers[0].enabled, "the header step stays");
    keys(cx, &s, &secondary("shift-z"));
    assert_eq!(text(cx), "[1]");
    assert!(!j_view(cx, &s).headers[0].enabled, "the header step stays");
}

#[test]
fn the_commands_bind_the_two_contexts_and_the_chords() {
    let cmds = apinata_app::commands::registry();
    let inputs = format!("{} > Input", apinata_app::kv_grid::STRUCTURED_ROWS_CONTEXT);
    for (id, label) in [("request.undo", "Undo edit"), ("request.redo", "Redo edit")] {
        let c = cmds.iter().find(|c| c.id == id).expect(id);
        assert_eq!(c.label, label);
        let contexts: Vec<&str> = c.targets.iter().map(|t| t.context).collect();
        assert_eq!(
            contexts,
            [
                "Shell",
                "RequestEditor && !Input",
                "ConfigEditor && !Input",
                "EnvEditor && !Input",
                inputs.as_str(),
                "EnvTable > Input"
            ]
        );
    }
    let chords = |id: &str| {
        cmds.iter()
            .find(|c| c.id == id)
            .unwrap()
            .default_chords
            .clone()
    };
    assert_eq!(chords("request.undo"), ["secondary-z"]);
    let redo = chords("request.redo");
    assert!(redo.contains(&"secondary-shift-z"));
    assert_eq!(redo.contains(&"ctrl-y"), !cfg!(target_os = "macos"));
}

// A folder's settings tab and an environments tab, each holding one structured step, in the
// whole app so the palette commands and the keymap both reach them.
struct Settings {
    s: Started,
    _col: tempfile::TempDir,
    config: Entity<apinata_app::config_editor::ConfigEditor>,
    env: Entity<apinata_app::env_editor::EnvEditor>,
    config_tab: apinata_app::tabs::TabId,
    env_tab: apinata_app::tabs::TabId,
}

fn settings(cx: &mut TestAppContext) -> Settings {
    use apinata_app::config_editor::ConfigSection;
    use apinata_app::tabs::TabKind;
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("Users")),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: host\n    value: x\n",
        ),
    ]);
    let s = started(cx, &col);
    let config_tab =
        s.ws.update(cx, |w, cx| {
            w.open_settings(&s.root, TabKind::Folder(ip("users")), cx)
        })
        .unwrap();
    let env_tab =
        s.ws.update(cx, |w, cx| {
            w.open_settings(&s.root, TabKind::Environments, cx)
        })
        .unwrap();
    frame(cx, s.win);
    s.ws.update(cx, |w, cx| {
        w.open_settings(&s.root, TabKind::Folder(ip("users")), cx)
    });
    frame(cx, s.win);
    let area = s.shell.read_with(cx, |v, _| v.tab_area().clone());
    let config = area.read_with(cx, |a, _| a.config_editor(config_tab).cloned().unwrap());
    let env = area.read_with(cx, |a, _| a.env_editor(env_tab).cloned().unwrap());
    let row = apinata_app::kv_table::KvRow {
        name: "X-Trace".into(),
        value: "1".into(),
        enabled: true,
        ..Default::default()
    };
    cx.update_window(s.win, |_, w, cx| {
        config.update(cx, |e, cx| {
            e.edit_rows(ConfigSection::Headers, vec![row], w, cx)
        })
    })
    .unwrap();
    env.update(cx, |e, cx| e.set_color("#ff0000", cx));
    cx.run_until_parked();
    Settings {
        s,
        _col: col,
        config,
        env,
        config_tab,
        env_tab,
    }
}

impl Settings {
    fn headers(&self, cx: &mut TestAppContext) -> usize {
        let snap = self
            .s
            .ws
            .read_with(cx, |w, cx| w.snapshot(&self.s.root, cx))
            .unwrap();
        snap.content.folders[&ip("users")].headers.len()
    }

    fn color(&self, cx: &mut TestAppContext) -> Option<String> {
        env_view(cx, &self.s.ws, &self.s.root, "dev").color.clone()
    }

    // Reopening a settings tab brings the open one to the front.
    fn activate(&self, cx: &mut TestAppContext, tab: apinata_app::tabs::TabId) {
        use apinata_app::tabs::TabKind;
        let kind = if tab == self.config_tab {
            TabKind::Folder(ip("users"))
        } else {
            TabKind::Environments
        };
        self.s
            .ws
            .update(cx, |w, cx| w.open_settings(&self.s.root, kind, cx));
        frame(cx, self.s.win);
    }

    fn focus_config(&self, cx: &mut TestAppContext) {
        use gpui_kit::Focusable as _;
        self.activate(cx, self.config_tab);
        cx.update_window(self.s.win, |_, w, cx| {
            let h = self.config.focus_handle(cx);
            w.focus(&h, cx)
        })
        .unwrap();
    }

    fn focus_env(&self, cx: &mut TestAppContext) {
        use gpui_kit::Focusable as _;
        self.activate(cx, self.env_tab);
        cx.update_window(self.s.win, |_, w, cx| {
            let h = self.env.focus_handle(cx);
            w.focus(&h, cx)
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn the_palette_commands_undo_and_redo_a_settings_tab_and_an_environments_tab(
    cx: &mut TestAppContext,
) {
    let t = settings(cx);
    assert_eq!(t.headers(cx), 1);
    t.activate(cx, t.config_tab);
    cx.dispatch_action(t.s.win, UndoActive);
    cx.run_until_parked();
    assert_eq!(t.headers(cx), 0);
    cx.dispatch_action(t.s.win, RedoActive);
    cx.run_until_parked();
    assert_eq!(t.headers(cx), 1);
    assert_eq!(t.color(cx).as_deref(), Some("#ff0000"));
    t.activate(cx, t.env_tab);
    cx.dispatch_action(t.s.win, UndoActive);
    cx.run_until_parked();
    assert_eq!(t.color(cx), None);
    assert_eq!(t.headers(cx), 1, "the other tab keeps its step");
    cx.dispatch_action(t.s.win, RedoActive);
    cx.run_until_parked();
    assert_eq!(t.color(cx).as_deref(), Some("#ff0000"));
}

#[gpui_kit::test]
fn ctrl_z_on_a_settings_editor_outside_any_input_undoes_one_step(cx: &mut TestAppContext) {
    let t = settings(cx);
    t.focus_config(cx);
    keys(cx, &t.s, &secondary("z"));
    assert_eq!(t.headers(cx), 0);
    keys(cx, &t.s, &secondary("shift-z"));
    assert_eq!(t.headers(cx), 1);
}

#[gpui_kit::test]
fn ctrl_z_on_an_environment_editor_outside_any_input_undoes_one_step(cx: &mut TestAppContext) {
    let t = settings(cx);
    t.focus_env(cx);
    keys(cx, &t.s, &secondary("z"));
    assert_eq!(t.color(cx), None);
    keys(cx, &t.s, &secondary("shift-z"));
    assert_eq!(t.color(cx).as_deref(), Some("#ff0000"));
}

#[gpui_kit::test]
fn ctrl_z_in_a_settings_text_input_leaves_the_structured_history(cx: &mut TestAppContext) {
    use apinata_app::config_editor::ConfigSection;
    let t = settings(cx);
    t.activate(cx, t.config_tab);
    t.config
        .update(cx, |e, cx| e.set_section(ConfigSection::Tags, cx));
    frame(cx, t.s.win);
    cx.update_window(t.s.win, |_, w, cx| {
        w.within("config-tags").click("tag-text", cx)
    })
    .unwrap();
    keys(cx, &t.s, &secondary("z"));
    assert_eq!(t.headers(cx), 1);
}

#[gpui_kit::test]
fn ctrl_z_in_the_environment_name_input_leaves_the_structured_history(cx: &mut TestAppContext) {
    let t = settings(cx);
    t.activate(cx, t.env_tab);
    cx.update_window(t.s.win, |_, w, cx| w.click("env-create", cx))
        .unwrap();
    frame(cx, t.s.win);
    cx.update_window(t.s.win, |_, w, cx| w.click("env-name-input", cx))
        .unwrap();
    keys(cx, &t.s, &secondary("z"));
    assert_eq!(t.color(cx).as_deref(), Some("#ff0000"));
}
