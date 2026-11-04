//! Undo and redo of the structured edits of a folder's or the collection's settings.

use crate::support;

use apinata_app::command::{Command, Edit};
use apinata_app::config_editor::{ConfigEditor, ConfigSection, ConfigTarget};
use apinata_app::kv_table::KvRow;
use apinata_app::workspace::Workspace;
use apinata_core::KeyValue;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use support::*;

struct Rig {
    _col: tempfile::TempDir,
    _state: tempfile::TempDir,
    ws: Entity<Workspace>,
    root: std::path::PathBuf,
    win: AnyWindowHandle,
    ed: Entity<ConfigEditor>,
    target: ConfigTarget,
}

fn rig(cx: &mut TestAppContext, target: ConfigTarget) -> Rig {
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("Users")),
        ("users/a.yml", &request("a", "http://x")),
    ]);
    let (state, ws, root) = open_dir(cx, &col);
    let (ws2, r, t) = (ws.clone(), root.clone(), target.clone());
    let (win, ed) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ConfigEditor::new(ws2.clone(), r.clone(), t.clone(), w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    Rig {
        _col: col,
        _state: state,
        ws,
        root,
        win,
        ed,
        target,
    }
}

fn row(name: &str, value: &str) -> KvRow {
    KvRow {
        name: name.into(),
        value: value.into(),
        enabled: true,
        ..KvRow::default()
    }
}

impl Rig {
    fn edit(&self, cx: &mut TestAppContext, section: ConfigSection, rows: Vec<KvRow>) {
        cx.update_window(self.win, |_, w, cx| {
            self.ed
                .update(cx, |e, cx| e.edit_rows(section, rows, w, cx))
        })
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

    fn view(&self, cx: &mut TestAppContext) -> std::sync::Arc<apinata_app::snapshot::ConfigView> {
        let snap = self
            .ws
            .read_with(cx, |w, cx| w.snapshot(&self.root, cx))
            .unwrap();
        match &self.target {
            ConfigTarget::Folder(p) => snap.content.folders[p].clone(),
            ConfigTarget::Collection => snap.content.collection.clone(),
        }
    }

    fn outside(&self, cx: &mut TestAppContext, edit: Edit) {
        let command = Command::Edit {
            target: self.target.target(),
            edit,
        };
        self.ws.update(cx, |w, cx| w.send(&self.root, command, cx));
        cx.run_until_parked();
    }
}

fn header_names(v: &apinata_app::snapshot::ConfigView) -> Vec<String> {
    v.headers.iter().map(|h| h.name.clone()).collect()
}

fn undo_redo_roundtrip(cx: &mut TestAppContext, target: ConfigTarget) {
    let r = rig(cx, target);
    assert!(!r.view(cx).dirty);
    r.edit(cx, ConfigSection::Headers, vec![row("X-Trace", "1")]);
    assert!(r.view(cx).dirty);
    // A second edit makes the first undo take back exactly one step.
    r.edit(
        cx,
        ConfigSection::Headers,
        vec![row("X-Trace", "1"), row("X-Two", "2")],
    );
    r.undo(cx);
    assert_eq!(header_names(&r.view(cx)), ["X-Trace"]);
    r.undo(cx);
    let v = r.view(cx);
    assert!(v.headers.is_empty() && !v.dirty);
    r.redo(cx);
    assert!(r.view(cx).dirty);
    r.redo(cx);
    assert_eq!(header_names(&r.view(cx)), ["X-Trace", "X-Two"]);
}

#[gpui_kit::test]
fn a_folder_setting_change_is_undone_and_redone_one_step_at_a_time(cx: &mut TestAppContext) {
    undo_redo_roundtrip(cx, ConfigTarget::Folder(ip("users")));
}

#[gpui_kit::test]
fn a_collection_setting_change_is_undone_and_redone_one_step_at_a_time(cx: &mut TestAppContext) {
    undo_redo_roundtrip(cx, ConfigTarget::Collection);
}

#[gpui_kit::test]
fn an_outside_change_retires_only_its_own_field(cx: &mut TestAppContext) {
    let r = rig(cx, ConfigTarget::Folder(ip("users")));
    r.edit(cx, ConfigSection::Headers, vec![row("X-Trace", "1")]);
    r.edit(cx, ConfigSection::Vars, vec![row("token", "abc")]);
    r.outside(cx, Edit::Headers(vec![KeyValue::new("X-Outside", "9")]));
    // The vars step is still valid and the headers step went stale.
    r.undo(cx);
    let v = r.view(cx);
    assert!(v.vars.is_empty());
    assert_eq!(header_names(&v), ["X-Outside"]);
    r.undo(cx);
    assert_eq!(header_names(&r.view(cx)), ["X-Outside"]);
    r.redo(cx);
    assert_eq!(r.view(cx).vars.len(), 1);
}

fn show_tags(r: &Rig, cx: &mut TestAppContext) {
    r.ed.update(cx, |e, cx| e.set_section(ConfigSection::Tags, cx));
    frame(cx, r.win);
}

fn add_tag(r: &Rig, cx: &mut TestAppContext, text: &str) {
    use gpui_kit::test::TestWindowExt as _;
    cx.update_window(r.win, |_, w, cx| {
        w.within("config-tags").click("tag-text", cx);
        w.input(text, cx);
        w.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_tag_edit_is_one_step_and_clean_at_the_saved_state(cx: &mut TestAppContext) {
    let r = rig(cx, ConfigTarget::Folder(ip("users")));
    show_tags(&r, cx);
    add_tag(&r, cx, "smoke");
    assert_eq!(r.view(cx).tags, ["smoke"]);
    assert!(r.view(cx).dirty);
    r.undo(cx);
    assert!(r.view(cx).tags.is_empty() && !r.view(cx).dirty);
    r.redo(cx);
    assert_eq!(r.view(cx).tags, ["smoke"]);
    assert!(r.view(cx).dirty);
}

#[gpui_kit::test]
fn an_auth_mode_switch_is_one_step_and_clean_at_the_saved_state(cx: &mut TestAppContext) {
    use apinata_app::auth_section::AuthMode;
    use apinata_core::AuthSpec;
    let r = rig(cx, ConfigTarget::Folder(ip("users")));
    let saved = r.view(cx).auth.clone();
    let auth = r.ed.read_with(cx, |e, _| e.auth().clone());
    cx.update_window(r.win, |_, w, cx| {
        auth.update(cx, |s, cx| s.set_mode(AuthMode::Basic, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(matches!(r.view(cx).auth, AuthSpec::Basic { .. }));
    assert!(r.view(cx).dirty);
    r.undo(cx);
    assert_eq!(r.view(cx).auth, saved);
    assert!(!r.view(cx).dirty);
    r.redo(cx);
    assert!(matches!(r.view(cx).auth, AuthSpec::Basic { .. }));
    assert!(r.view(cx).dirty);
}

// A header name with a line break is refused by core, so the edit fails.
#[gpui_kit::test]
fn a_refused_edit_reverts_its_own_history_change(cx: &mut TestAppContext) {
    let r = rig(cx, ConfigTarget::Folder(ip("users")));
    r.edit(cx, ConfigSection::Headers, vec![row("X-Trace", "1")]);
    r.edit(cx, ConfigSection::Headers, vec![row("bad\nname", "1")]);
    assert_eq!(header_names(&r.view(cx)), ["X-Trace"]);
    // The refused edit left no step of its own, so one undo takes back the good edit.
    r.undo(cx);
    assert!(r.view(cx).headers.is_empty() && !r.view(cx).dirty);
}

// Network and presets edits are only noted: they neither add steps nor disturb the
// headers steps already held.
#[gpui_kit::test]
fn a_network_or_presets_edit_leaves_other_steps_intact(cx: &mut TestAppContext) {
    use apinata_app::section::SectionEdit;
    use apinata_core::{CollectionPresets, CollectionProxy};
    let r = rig(cx, ConfigTarget::Collection);
    r.edit(cx, ConfigSection::Headers, vec![row("X-Trace", "1")]);
    let network = r.ed.read_with(cx, |e, _| e.network().unwrap().clone());
    for edit in [
        Edit::Proxy(CollectionProxy::Off),
        Edit::Presets(CollectionPresets::new("graphql", "https://g.test")),
    ] {
        network.update(cx, |_, cx| cx.emit(SectionEdit::new(edit)));
        cx.run_until_parked();
    }
    assert!(r.view(cx).presets.is_some());
    r.undo(cx);
    assert!(r.view(cx).headers.is_empty());
    assert!(r.view(cx).presets.is_some(), "undo touched only headers");
    r.redo(cx);
    assert_eq!(header_names(&r.view(cx)), ["X-Trace"]);
}

// Undo of a header typed into the empty row drops the row that held focus; focus must stay in
// the table, or the redo key reaches no editor until a click.
#[gpui_kit::test]
fn a_new_header_row_is_redone_by_keys_after_its_undo(cx: &mut TestAppContext) {
    let r = rig(cx, ConfigTarget::Collection);
    r.ed.update(cx, |e, cx| e.set_section(ConfigSection::Headers, cx));
    frame(cx, r.win);
    type_at_end(
        cx,
        r.win,
        Some("config-headers"),
        ("kv-name", 0usize),
        "X-New",
    );
    cx.run_until_parked();
    assert_eq!(header_names(&r.view(cx)), ["X-New"]);
    cx.simulate_keystrokes(r.win, &secondary("z"));
    cx.run_until_parked();
    assert!(r.view(cx).headers.is_empty());
    assert!(
        focus_within(cx, r.win, Some("config-headers")),
        "focus stays in the table after the undo"
    );
    cx.simulate_keystrokes(r.win, &secondary("shift-z"));
    cx.run_until_parked();
    assert_eq!(
        header_names(&r.view(cx)),
        ["X-New"],
        "the redo key brings the row back"
    );
    assert!(
        focus_within(cx, r.win, Some("config-headers")),
        "focus stays in the table after the redo"
    );
}

// Undo of a new header while its Remove button holds focus: focus moves into a cell, so the
// redo key still reaches the editor without a click.
#[gpui_kit::test]
fn a_new_header_row_is_redone_by_keys_after_its_undo_from_remove(cx: &mut TestAppContext) {
    let r = rig(cx, ConfigTarget::Collection);
    r.ed.update(cx, |e, cx| e.set_section(ConfigSection::Headers, cx));
    frame(cx, r.win);
    type_at_end(
        cx,
        r.win,
        Some("config-headers"),
        ("kv-name", 0usize),
        "X-New",
    );
    cx.run_until_parked();
    assert_eq!(header_names(&r.view(cx)), ["X-New"]);
    tab_within_to(cx, r.win, "config-headers", ("kv-delete", 0usize));
    cx.simulate_keystrokes(r.win, &secondary("z"));
    cx.run_until_parked();
    assert!(r.view(cx).headers.is_empty());
    assert!(
        focus_within(cx, r.win, Some("config-headers")),
        "focus moves into a cell after the undo"
    );
    cx.simulate_keystrokes(r.win, &secondary("shift-z"));
    cx.run_until_parked();
    assert_eq!(
        header_names(&r.view(cx)),
        ["X-New"],
        "the redo key brings the row back"
    );
}

const FOLDER_THREE: &str = "info:\n  name: Users\n  type: folder\n  seq: 1\nrequest:\n  headers:\n    - name: h1\n      value: one\n    - name: h2\n      value: two\n    - name: h3\n      value: three\n";

// A folder's header step from before a disk reload must not let a later undo write the
// pre-reload headers back over the outside change.
#[gpui_kit::test]
fn undo_after_a_disk_reload_keeps_the_outside_change(cx: &mut TestAppContext) {
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", FOLDER_THREE),
        ("users/a.yml", &request("a", "http://x")),
    ]);
    let w = watched(cx, &col);
    let target = ConfigTarget::Folder(ip("users"));
    let (ws2, root2, t) = (w.ws.clone(), w.root.clone(), target.clone());
    let (win, ed) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |win, cx| {
                cx.new(|cx| ConfigEditor::new(ws2.clone(), root2.clone(), t.clone(), win, cx))
            })
        })
        .unwrap();
    let r = Rig {
        _col: col,
        _state: w.state,
        ws: w.ws,
        root: w.root,
        win,
        ed,
        target,
    };
    r.ed.update(cx, |e, cx| e.set_section(ConfigSection::Headers, cx));
    frame(cx, r.win);
    let pairs = |r: &Rig, cx: &mut TestAppContext| -> Vec<(String, String)> {
        (r.view(cx).headers.iter())
            .map(|h| (h.name.clone(), h.value.clone()))
            .collect()
    };
    let delete = |r: &Rig, cx: &mut TestAppContext, ix: usize| {
        use gpui_kit::test::TestWindowExt as _;
        cx.update_window(r.win, |_, w, cx| {
            w.within("config-headers").click(("kv-delete", ix), cx)
        })
        .unwrap();
        frame(cx, r.win);
    };
    let save = |r: &Rig, cx: &mut TestAppContext| {
        r.ed.update(cx, |e, cx| e.save(cx));
        cx.run_until_parked();
        assert!(!r.view(cx).dirty, "the save lands");
    };
    delete(&r, cx, 1);
    save(&r, cx);
    settle(cx);
    let outside = "info:\n  name: Users\n  type: folder\n  seq: 1\nrequest:\n  headers:\n    - name: h1\n      value: one\n    - name: h3\n      value: three-x\n";
    std::fs::write(r.root.join("users/folder.yml"), outside).unwrap();
    wait_for(cx, "the reload to show the outside change", |cx| {
        pairs(&r, cx).iter().any(|(_, v)| v == "three-x")
    });
    frame(cx, r.win);
    delete(&r, cx, 0);
    press_key(cx, r.win, &secondary("z"));
    let want = |list: &[(&str, &str)]| -> Vec<(String, String)> {
        list.iter()
            .map(|(n, v)| (n.to_string(), v.to_string()))
            .collect()
    };
    assert_eq!(
        pairs(&r, cx),
        want(&[("h1", "one"), ("h3", "three-x")]),
        "the undo brings h1 back and keeps h3's outside value"
    );
    save(&r, cx);
    let disk = std::fs::read_to_string(r.root.join("users/folder.yml")).unwrap();
    assert!(
        disk.contains("three-x") && disk.contains("name: h1"),
        "{disk}"
    );
    assert!(!disk.contains("name: h2"), "{disk}");
}

/// Runs at each workspace notify whose snapshot shows h3's outside value, given how many such
/// notifies came before it; one registered before the editor runs ahead of its refresh.
type Hook = std::rc::Rc<std::cell::RefCell<Box<dyn FnMut(&mut gpui_kit::App, usize)>>>;

type Slot = std::rc::Rc<
    std::cell::RefCell<
        Option<(
            Entity<ConfigEditor>,
            Entity<Workspace>,
            std::path::PathBuf,
            ConfigTarget,
        )>,
    >,
>;

const COLLECTION_THREE: &str = "opencollection: 1.0.0\ninfo:\n  name: c\nrequest:\n  headers:\n    - name: h1\n      value: one\n    - name: h2\n      value: two\n    - name: h3\n      value: three\n";

fn hook(f: impl FnMut(&mut gpui_kit::App, usize) + 'static) -> Hook {
    std::rc::Rc::new(std::cell::RefCell::new(Box::new(f)))
}

/// The settings file `target` is saved to, relative to the collection root.
fn settings_file(target: &ConfigTarget) -> String {
    match target {
        ConfigTarget::Folder(p) => format!("{}/folder.yml", p.as_str()),
        ConfigTarget::Collection => "opencollection.yml".into(),
    }
}

/// The h1, h2, h3 settings `target` starts a race with.
fn three_for(target: &ConfigTarget) -> &'static str {
    match target {
        ConfigTarget::Folder(_) => FOLDER_THREE,
        ConfigTarget::Collection => COLLECTION_THREE,
    }
}

fn target_headers(
    cx: &gpui_kit::App,
    ws: &Entity<Workspace>,
    root: &std::path::Path,
    target: &ConfigTarget,
) -> Vec<KeyValue> {
    let snap = ws.read(cx).snapshot(root, cx).unwrap();
    let view = match target {
        ConfigTarget::Folder(p) => snap.content.folders.get(p).unwrap().clone(),
        ConfigTarget::Collection => snap.content.collection.clone(),
    };
    view.headers.clone()
}

fn on_outside_h3(
    cx: &mut TestAppContext,
    ws: &Entity<Workspace>,
    root: &std::path::Path,
    target: &ConfigTarget,
    hook: Hook,
) -> gpui_kit::Subscription {
    let (root, target) = (root.to_path_buf(), target.clone());
    let count = std::cell::Cell::new(0);
    cx.update(|cx| {
        cx.observe(ws, move |ws, cx| {
            if target_headers(cx, &ws, &root, &target)
                .iter()
                .any(|h| h.value == "three-x")
            {
                count.set(count.get() + 1);
                (hook.borrow_mut())(cx, count.get());
            }
        })
    })
}

/// The settings of `target` (the users folder or the collection) with its Headers shown,
/// `before` observing the workspace ahead of the editor and `after` behind it; `slot` is
/// filled with the editor, workspace, root and target.
fn race_rig(
    cx: &mut TestAppContext,
    target: ConfigTarget,
    slot: &Slot,
    before: Hook,
    after: Hook,
) -> (Rig, [gpui_kit::Subscription; 2]) {
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let users = folder("Users");
    let (collection, users) = match target {
        ConfigTarget::Folder(_) => (ROOT, FOLDER_THREE),
        ConfigTarget::Collection => (COLLECTION_THREE, users.as_str()),
    };
    let col = write_tree(&[
        ("opencollection.yml", collection),
        ("users/folder.yml", users),
        ("users/a.yml", &request("a", "http://x")),
    ]);
    let (state, ws, root) = open_dir(cx, &col);
    let first = on_outside_h3(cx, &ws, &root, &target, before);
    let (ws2, r, t) = (ws.clone(), root.clone(), target.clone());
    let (win, ed) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ConfigEditor::new(ws2.clone(), r.clone(), t.clone(), w, cx))
            })
        })
        .unwrap();
    let second = on_outside_h3(cx, &ws, &root, &target, after);
    ed.update(cx, |e, cx| e.set_section(ConfigSection::Headers, cx));
    frame(cx, win);
    *slot.borrow_mut() = Some((ed.clone(), ws.clone(), root.clone(), target.clone()));
    let r = Rig {
        _col: col,
        _state: state,
        ws,
        root,
        win,
        ed,
        target,
    };
    (r, [first, second])
}

/// Turns h1 off in the Headers table's rows as they stand and emits them, as a click would.
fn table_turns_h1_off(cx: &mut gpui_kit::App, ed: &Entity<ConfigEditor>) {
    let table = ed.read(cx).table(ConfigSection::Headers).cloned().unwrap();
    let mut rows = table.read(cx).rows(cx);
    rows[0].enabled = false;
    table.update(cx, |_, cx| {
        cx.emit(apinata_app::kv_table::KvEvent::Changed(rows))
    });
}

/// Turns h2 off in the snapshot's own headers, an edit built from the reloaded content.
fn view_turns_h2_off(
    cx: &mut gpui_kit::App,
    ws: &Entity<Workspace>,
    root: &std::path::Path,
    target: &ConfigTarget,
) {
    let mut headers = target_headers(cx, ws, root, target);
    headers[1].enabled = false;
    let command = Command::Edit {
        target: target.target(),
        edit: Edit::Headers(headers),
    };
    ws.update(cx, |w, cx| w.send(root, command, cx));
}

impl Rig {
    /// Writes h3's outside value to the settings file and reloads it, as the watcher does.
    fn reload_outside_h3(&self, cx: &mut TestAppContext) {
        let file = self.root.join(settings_file(&self.target));
        std::fs::write(
            &file,
            three_for(&self.target).replace("value: three", "value: three-x"),
        )
        .unwrap();
        let root = self.root.clone();
        self.ws
            .update(cx, |w, cx| w.send(&root, Command::Reload(vec![file]), cx));
        cx.run_until_parked();
        frame(cx, self.win);
        cx.run_until_parked();
    }

    /// After everything settled: h3 keeps its outside value, the user's h1 edit either landed
    /// on top of it or is held back as stale, and a save leaves the outside value on disk.
    fn assert_outside_h3_kept(&self, cx: &mut TestAppContext) {
        let v = self.view(cx);
        let h3 = v.headers.iter().find(|h| h.name == "h3").unwrap();
        assert_eq!(
            h3.value, "three-x",
            "the outside change survives: {:?}",
            v.headers
        );
        let held = (self
            .ws
            .read_with(cx, |w, cx| w.snapshot(&self.root, cx))
            .unwrap()
            .stale)
            .contains_key(&self.target.target());
        assert!(
            held || !v.headers[0].enabled,
            "the h1 edit landed or is held"
        );
        if v.dirty {
            self.ed.update(cx, |e, cx| e.save(cx));
            cx.run_until_parked();
        }
        let disk = std::fs::read_to_string(self.root.join(settings_file(&self.target))).unwrap();
        assert!(disk.contains("three-x"), "{disk}");
    }
}

fn same_update_race(cx: &mut TestAppContext, target: ConfigTarget, loads: usize) {
    let slot = Slot::default();
    let (s1, s2) = (slot.clone(), slot.clone());
    let before = hook(move |cx, n| {
        if n == 1 {
            let (ed, _, _, _) = s1.borrow().clone().unwrap();
            table_turns_h1_off(cx, &ed);
        }
    });
    let after = hook(move |cx, n| {
        if n < loads {
            let (_, ws, _, _) = s2.borrow().clone().unwrap();
            ws.update(cx, |_, cx| cx.notify());
        }
    });
    let (r, _subs) = race_rig(cx, target, &slot, before, after);
    r.reload_outside_h3(cx);
    r.assert_outside_h3_kept(cx);
}

fn busy_field_race(cx: &mut TestAppContext, target: ConfigTarget, loads: usize) {
    let slot = Slot::default();
    let (s1, s2) = (slot.clone(), slot.clone());
    let before = hook(move |cx, n| {
        if n == 1 {
            let (_, ws, root, target) = s1.borrow().clone().unwrap();
            view_turns_h2_off(cx, &ws, &root, &target);
        }
    });
    let after = hook(move |cx, n| {
        let (ed, ws, _, _) = s2.borrow().clone().unwrap();
        if n < loads {
            ws.update(cx, |_, cx| cx.notify());
        } else if n == loads {
            table_turns_h1_off(cx, &ed);
        }
    });
    let (r, _subs) = race_rig(cx, target, &slot, before, after);
    r.reload_outside_h3(cx);
    r.assert_outside_h3_kept(cx);
}

fn users_target() -> ConfigTarget {
    ConfigTarget::Folder(ip("users"))
}

// Window one: the table emits an edit in the update the reload lands in, before the editor's
// refresh has loaded the reloaded rows.
#[gpui_kit::test]
fn a_table_edit_in_the_update_a_reload_lands_in_keeps_the_outside_change(cx: &mut TestAppContext) {
    same_update_race(cx, users_target(), 1);
}

#[gpui_kit::test]
fn a_table_edit_in_the_update_a_reload_lands_in_keeps_it_across_two_loads(cx: &mut TestAppContext) {
    same_update_race(cx, users_target(), 2);
}

// Window two: the reload lands while Headers has an edit in flight, so the refresh leaves the
// table on its pre-reload rows; the table's next edit is built from them.
#[gpui_kit::test]
fn a_table_edit_while_its_field_is_busy_through_a_reload_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    busy_field_race(cx, users_target(), 1);
}

#[gpui_kit::test]
fn a_table_edit_while_its_field_is_busy_through_two_loads_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    busy_field_race(cx, users_target(), 2);
}

// The same four races on the collection's own settings in opencollection.yml.
#[gpui_kit::test]
fn a_collection_table_edit_in_the_update_a_reload_lands_in_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    same_update_race(cx, ConfigTarget::Collection, 1);
}

#[gpui_kit::test]
fn a_collection_table_edit_in_the_update_a_reload_lands_in_keeps_it_across_two_loads(
    cx: &mut TestAppContext,
) {
    same_update_race(cx, ConfigTarget::Collection, 2);
}

#[gpui_kit::test]
fn a_collection_table_edit_while_its_field_is_busy_through_a_reload_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    busy_field_race(cx, ConfigTarget::Collection, 1);
}

#[gpui_kit::test]
fn a_collection_table_edit_while_its_field_is_busy_through_two_loads_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    busy_field_race(cx, ConfigTarget::Collection, 2);
}

/// Edits a section of an open settings editor, as the user would in its window.
type OnEditor = fn(&mut gpui_kit::App, &Entity<ConfigEditor>, AnyWindowHandle);

/// Whether each edit a watched section sent lagged as it arrived.
type Lagged = std::rc::Rc<std::cell::RefCell<Vec<bool>>>;

type Watch = fn(&mut gpui_kit::App, &Entity<ConfigEditor>, Lagged) -> gpui_kit::Subscription;

/// The settings view of `target` in the current snapshot, if it has one.
fn config_view(
    cx: &gpui_kit::App,
    ws: &Entity<Workspace>,
    root: &std::path::Path,
    target: &ConfigTarget,
) -> Option<std::sync::Arc<apinata_app::snapshot::ConfigView>> {
    let snap = ws.read(cx).snapshot(root, cx)?;
    match target {
        ConfigTarget::Folder(p) => snap.content.folders.get(p).cloned(),
        ConfigTarget::Collection => Some(snap.content.collection.clone()),
    }
}

/// Notes, for each edit `section` emits, what its `lags` says on delivery.
fn watch<T: gpui_kit::EventEmitter<apinata_app::section::SectionEdit>>(
    cx: &mut gpui_kit::App,
    section: Entity<T>,
    lagged: Lagged,
    lags: fn(&T, &Edit, &gpui_kit::App) -> bool,
) -> gpui_kit::Subscription {
    cx.subscribe(
        &section,
        move |s, e: &apinata_app::section::SectionEdit, cx| {
            let l = lags(s.read(cx), &e.edit, cx);
            lagged.borrow_mut().push(l);
        },
    )
}

fn watch_network(
    cx: &mut gpui_kit::App,
    ed: &Entity<ConfigEditor>,
    l: Lagged,
) -> gpui_kit::Subscription {
    let network = ed.read(cx).network().unwrap().clone();
    cx.subscribe(
        &network,
        move |s, e: &apinata_app::section::SectionEdit, cx| {
            let lags = s.read(cx).lags(e, cx);
            l.borrow_mut().push(lags);
        },
    )
}

/// A whole-value section edit of the users folder's or the collection's settings, its file
/// before and after an outside reload, and the text each leaves on disk. The stale edit
/// differs from `outside` only in what the reload changed.
struct ConfigRace {
    target: ConfigTarget,
    text: String,
    outside: String,
    section: ConfigSection,
    landed: fn(&apinata_app::snapshot::ConfigView, &apinata_app::snapshot::ConfigView) -> bool,
    edit: OnEditor,
    /// On disk once the outside change survives the stale edit.
    kept: &'static str,
    /// On disk once the edit lands without a reload.
    edited: &'static str,
    /// Watches the section's edits, noting whether each lags as it reaches the editor;
    /// `None` for a text section, which reads its editor only when the change arrives.
    watch: Option<Watch>,
    /// Whether the stale edit is held; a text section sends the outside text instead.
    held: bool,
}

impl ConfigRace {
    // The settings under test in `text`, the other file plain.
    fn tree(&self, text: &str) -> tempfile::TempDir {
        let users = folder("Users");
        let (collection, users) = match self.target {
            ConfigTarget::Folder(_) => (ROOT, text),
            ConfigTarget::Collection => (text, users.as_str()),
        };
        write_tree(&[
            ("opencollection.yml", collection),
            ("users/folder.yml", users),
            ("users/a.yml", &request("a", "http://x")),
        ])
    }

    // Opens the settings editor on `self.text` with `section` shown; `ahead`, given the editor
    // slot, observes the workspace before the editor does, so it runs ahead of its refresh.
    fn open(
        &self,
        cx: &mut TestAppContext,
        ahead: impl FnOnce(
            &mut TestAppContext,
            &Entity<Workspace>,
            &std::path::Path,
            EdSlot,
        ) -> Option<gpui_kit::Subscription>,
    ) -> (Rig, Option<gpui_kit::Subscription>) {
        support::init(cx);
        cx.update(apinata_app::keymap::init);
        let col = self.tree(&self.text);
        let (state, ws, root) = open_dir(cx, &col);
        let slot = EdSlot::default();
        let sub = ahead(cx, &ws, &root, slot.clone());
        let (ws2, r, t) = (ws.clone(), root.clone(), self.target.clone());
        let (win, ed) = cx
            .update(|cx| {
                gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                    cx.new(|cx| ConfigEditor::new(ws2.clone(), r.clone(), t.clone(), w, cx))
                })
            })
            .unwrap();
        *slot.borrow_mut() = Some((ed.clone(), win));
        ed.update(cx, |e, cx| e.set_section(self.section, cx));
        frame(cx, win);
        cx.run_until_parked();
        let rig = Rig {
            _col: col,
            _state: state,
            ws,
            root,
            win,
            ed,
            target: self.target.clone(),
        };
        (rig, sub)
    }

    // Writes `text` over the settings file and reloads it, as the watcher does.
    fn reload(&self, cx: &mut TestAppContext, r: &Rig, text: &str) {
        let file = r.root.join(settings_file(&self.target));
        std::fs::write(&file, text).unwrap();
        let root = r.root.clone();
        r.ws.update(cx, |w, cx| w.send(&root, Command::Reload(vec![file]), cx));
        cx.run_until_parked();
        frame(cx, r.win);
        cx.run_until_parked();
    }

    fn disk(&self, r: &Rig) -> String {
        std::fs::read_to_string(r.root.join(settings_file(&self.target))).unwrap()
    }

    fn held(r: &Rig, cx: &mut TestAppContext) -> bool {
        (r.ws
            .read_with(cx, |w, cx| w.snapshot(&r.root, cx))
            .unwrap()
            .stale)
            .contains_key(&r.target.target())
    }

    /// Makes the edit in the window before the editor's refresh of the reload, saves, and
    /// asserts the outside change survives and whether the edit is held.
    fn race(self, cx: &mut TestAppContext) {
        let (target, landed, edit) = (self.target.clone(), self.landed, self.edit);
        let (r, _ahead) = self.open(cx, move |cx, ws, root, slot| {
            let opened = cx.read(|cx| config_view(cx, ws, root, &target)).unwrap();
            let (root, fired) = (root.to_path_buf(), std::cell::Cell::new(false));
            Some(cx.update(|cx| {
                cx.observe(ws, move |ws, cx| {
                    let done = config_view(cx, &ws, &root, &target)
                        .is_some_and(|now| landed(&opened, &now));
                    if done && !fired.replace(true) {
                        let (ed, win) = slot.borrow().clone().unwrap();
                        edit(cx, &ed, win);
                    }
                })
            }))
        });
        self.reload(cx, &r, &self.outside);
        if r.view(cx).dirty {
            r.ed.update(cx, |e, cx| e.save(cx));
            cx.run_until_parked();
        }
        let disk = self.disk(&r);
        assert!(disk.contains(self.kept), "{disk}");
        assert_eq!(
            Self::held(&r, cx),
            self.held,
            "whether the stale edit is held"
        );
    }

    /// Makes the edit once an unrelated reload has settled, saves, and asserts it lands,
    /// nothing is held and the section did not count it as lagging.
    fn plain(self, cx: &mut TestAppContext) {
        let (r, _) = self.open(cx, |_, _, _, _| None);
        let renamed = self.text.replacen("info:\n  name: ", "info:\n  name: Z", 1);
        assert_ne!(renamed, self.text);
        let before = r.view(cx).name.clone();
        self.reload(cx, &r, &renamed);
        assert_ne!(r.view(cx).name, before, "the reload settled");
        let lagged = Lagged::default();
        let _watch = (self.watch).map(|w| cx.update(|cx| w(cx, &r.ed, lagged.clone())));
        cx.update(|cx| (self.edit)(cx, &r.ed, r.win));
        cx.run_until_parked();
        if self.watch.is_some() {
            let lagged = lagged.borrow();
            assert!(!lagged.is_empty(), "the section sent an edit");
            assert!(!lagged.iter().any(|&l| l), "an ordinary edit does not lag");
        }
        assert!(!Self::held(&r, cx), "nothing is held");
        r.ed.update(cx, |e, cx| e.save(cx));
        cx.run_until_parked();
        let disk = self.disk(&r);
        assert!(disk.contains(self.edited), "{disk}");
    }
}

type EdSlot = std::rc::Rc<std::cell::RefCell<Option<(Entity<ConfigEditor>, AnyWindowHandle)>>>;

/// The settings file `target` starts with: its head, then `body`.
fn settings_with(target: &ConfigTarget, body: &str) -> String {
    match target {
        ConfigTarget::Folder(_) => format!("{}{body}", folder("Users")),
        ConfigTarget::Collection => format!("{ROOT}{body}"),
    }
}

const APIKEY_OLD: &str =
    "request:\n  auth:\n    type: apikey\n    key: k\n    value: old\n    placement: header\n";
const APIKEY_NEW: &str =
    "request:\n  auth:\n    type: apikey\n    key: k\n    value: new\n    placement: query\n";

// The user moves the API key to the query; the reload moves it too and changes its value.
fn auth_race(target: ConfigTarget) -> ConfigRace {
    ConfigRace {
        text: settings_with(&target, APIKEY_OLD),
        outside: settings_with(&target, APIKEY_NEW),
        target,
        section: ConfigSection::Auth,
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
fn a_folder_auth_edit_from_values_a_reload_replaced_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    auth_race(users_target()).race(cx);
}

#[gpui_kit::test]
fn a_collection_auth_edit_from_values_a_reload_replaced_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    auth_race(ConfigTarget::Collection).race(cx);
}

#[gpui_kit::test]
fn a_folder_auth_edit_after_a_settled_reload_lands(cx: &mut TestAppContext) {
    auth_race(users_target()).plain(cx);
}

#[gpui_kit::test]
fn a_collection_auth_edit_after_a_settled_reload_lands(cx: &mut TestAppContext) {
    auth_race(ConfigTarget::Collection).plain(cx);
}

const OAUTH1_OLD: &str = "request:\n  auth:\n    type: oauth1\n    consumerKey: old\n    signatureMethod: PLAINTEXT\n    placement: header\n";
const OAUTH1_NEW: &str = "request:\n  auth:\n    type: oauth1\n    consumerKey: new\n    signatureMethod: PLAINTEXT\n    placement: query\n";

// The user moves the OAuth 1.0 parameters to the query in the sub-editor; the reload moves
// them too and changes the consumer key, and the sub-editor's event arrives after the refresh.
fn oauth1_race(target: ConfigTarget) -> ConfigRace {
    ConfigRace {
        text: settings_with(&target, OAUTH1_OLD),
        outside: settings_with(&target, OAUTH1_NEW),
        target,
        section: ConfigSection::Auth,
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
fn a_folder_oauth1_edit_from_a_spec_a_reload_replaced_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    oauth1_race(users_target()).race(cx);
}

#[gpui_kit::test]
fn a_collection_oauth1_edit_from_a_spec_a_reload_replaced_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    oauth1_race(ConfigTarget::Collection).race(cx);
}

#[gpui_kit::test]
fn a_folder_oauth1_edit_after_a_settled_reload_lands(cx: &mut TestAppContext) {
    oauth1_race(users_target()).plain(cx);
}

#[gpui_kit::test]
fn a_collection_oauth1_edit_after_a_settled_reload_lands(cx: &mut TestAppContext) {
    oauth1_race(ConfigTarget::Collection).plain(cx);
}

const OAUTH2_OLD: &str = "request:\n  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: http://id/token\n";
const OAUTH2_NEW: &str = "request:\n  auth:\n    type: oauth2\n    flow: resource_owner_password_credentials\n    accessTokenUrl: http://id/new\n";

// As the OAuth 1.0 race, for the OAuth 2.0 sub-editor: the user switches the grant and the
// reload switches it too and changes the token URL.
fn oauth2_race(target: ConfigTarget) -> ConfigRace {
    ConfigRace {
        text: settings_with(&target, OAUTH2_OLD),
        outside: settings_with(&target, OAUTH2_NEW),
        target,
        section: ConfigSection::Auth,
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

#[gpui_kit::test]
fn a_folder_oauth2_edit_from_a_spec_a_reload_replaced_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    oauth2_race(users_target()).race(cx);
}

#[gpui_kit::test]
fn a_collection_oauth2_edit_from_a_spec_a_reload_replaced_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    oauth2_race(ConfigTarget::Collection).race(cx);
}

#[gpui_kit::test]
fn a_folder_oauth2_edit_after_a_settled_reload_lands(cx: &mut TestAppContext) {
    oauth2_race(users_target()).plain(cx);
}

#[gpui_kit::test]
fn a_collection_oauth2_edit_after_a_settled_reload_lands(cx: &mut TestAppContext) {
    oauth2_race(ConfigTarget::Collection).plain(cx);
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
fn docs_race(target: ConfigTarget) -> ConfigRace {
    ConfigRace {
        text: settings_with(&target, "docs: old notes\n"),
        outside: settings_with(&target, "docs: new notes!\n"),
        target,
        section: ConfigSection::Docs,
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
fn a_folder_docs_edit_from_text_a_reload_replaced_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    docs_race(users_target()).race(cx);
}

#[gpui_kit::test]
fn a_collection_docs_edit_from_text_a_reload_replaced_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    docs_race(ConfigTarget::Collection).race(cx);
}

#[gpui_kit::test]
fn a_folder_docs_edit_after_a_settled_reload_lands(cx: &mut TestAppContext) {
    docs_race(users_target()).plain(cx);
}

#[gpui_kit::test]
fn a_collection_docs_edit_after_a_settled_reload_lands(cx: &mut TestAppContext) {
    docs_race(ConfigTarget::Collection).plain(cx);
}

/// The three phases' scripts, `x()\nb()` in `changed` and `a()` in the others.
fn scripts_body(changed: Option<apinata_core::ScriptPhase>) -> String {
    use apinata_core::ScriptPhase::*;
    let code = |phase| {
        if changed == Some(phase) {
            "|-\n        x()\n        b()"
        } else {
            "a()"
        }
    };
    format!(
        "request:\n  scripts:\n    - type: before-request\n      code: {}\n    - type: after-response\n      code: {}\n    - type: tests\n      code: {}\n",
        code(PreRequest),
        code(PostResponse),
        code(Tests)
    )
}

// The user appends `b()` to one phase's script; the reload appends it too and rewrites the
// first call, in that phase only.
fn script_race(target: ConfigTarget, phase: apinata_core::ScriptPhase) -> ConfigRace {
    use apinata_core::ScriptPhase::*;
    type Landed =
        fn(&apinata_app::snapshot::ConfigView, &apinata_app::snapshot::ConfigView) -> bool;
    let (section, landed, edit): (_, Landed, OnEditor) = match phase {
        PreRequest => (
            ConfigSection::Script,
            |o, now| o.scripts.pre_request != now.scripts.pre_request,
            |cx, ed, win| script_edit(cx, ed.read(cx).script().clone(), PreRequest, win),
        ),
        PostResponse => (
            ConfigSection::Script,
            |o, now| o.scripts.post_response != now.scripts.post_response,
            |cx, ed, win| script_edit(cx, ed.read(cx).script().clone(), PostResponse, win),
        ),
        _ => (
            ConfigSection::Tests,
            |o, now| o.scripts.tests != now.scripts.tests,
            |cx, ed, win| script_edit(cx, ed.read(cx).tests_script().clone(), Tests, win),
        ),
    };
    ConfigRace {
        text: settings_with(&target, &scripts_body(None)),
        outside: settings_with(&target, &scripts_body(Some(phase))),
        target,
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
fn a_folder_script_edit_from_text_a_reload_replaced_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    use apinata_core::ScriptPhase::*;
    for phase in [PreRequest, PostResponse, Tests] {
        script_race(users_target(), phase).race(cx);
    }
}

#[gpui_kit::test]
fn a_collection_script_edit_from_text_a_reload_replaced_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    use apinata_core::ScriptPhase::*;
    for phase in [PreRequest, PostResponse, Tests] {
        script_race(ConfigTarget::Collection, phase).race(cx);
    }
}

#[gpui_kit::test]
fn a_folder_script_edit_after_a_settled_reload_lands(cx: &mut TestAppContext) {
    use apinata_core::ScriptPhase::*;
    for phase in [PreRequest, Tests] {
        script_race(users_target(), phase).plain(cx);
    }
}

#[gpui_kit::test]
fn a_collection_script_edit_after_a_settled_reload_lands(cx: &mut TestAppContext) {
    use apinata_core::ScriptPhase::*;
    for phase in [PreRequest, Tests] {
        script_race(ConfigTarget::Collection, phase).plain(cx);
    }
}

// The user removes tag a; the reload removes it too and renames b.
fn tags_race() -> ConfigRace {
    let target = users_target();
    ConfigRace {
        text: "info:\n  name: Users\n  type: folder\n  seq: 1\n  tags:\n    - a\n    - b\n".into(),
        outside: "info:\n  name: Users\n  type: folder\n  seq: 1\n  tags:\n    - y\n".into(),
        target,
        section: ConfigSection::Tags,
        landed: |o, now| o.tags != now.tags,
        edit: |cx, ed, _| {
            let tags = ed.read(cx).tags().unwrap().clone();
            tags.update(cx, |t, cx| t.remove(0, cx));
        },
        kept: "- y",
        edited: "tags:\n    - b\n",
        watch: Some(|cx, ed, lagged| {
            let tags = ed.read(cx).tags().unwrap().clone();
            cx.subscribe(
                &tags,
                move |t, e: &apinata_app::tag_input::TagsChanged, cx| {
                    let l = t.read(cx).lags(&e.0);
                    lagged.borrow_mut().push(l);
                },
            )
        }),
        held: true,
    }
}

#[gpui_kit::test]
fn a_folder_tags_edit_from_values_a_reload_replaced_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    tags_race().race(cx);
}

#[gpui_kit::test]
fn a_folder_tags_edit_after_a_settled_reload_lands(cx: &mut TestAppContext) {
    tags_race().plain(cx);
}

const PROXY_OFF_OLD: &str = "config:\n  proxy:\n    inherit: false\n    disabled: true\n    config:\n      protocol: http\n      hostname: old.local\n      port: \"3128\"\n      bypassProxy: \"\"\n";
const PROXY_ON_NEW: &str = "config:\n  proxy:\n    inherit: false\n    config:\n      protocol: http\n      hostname: new.local\n      port: \"3128\"\n      bypassProxy: \"\"\n";

fn network_of(v: &apinata_app::snapshot::ConfigView) -> &apinata_core::CollectionNetwork {
    v.network.as_ref().unwrap()
}

// The user turns the stored proxy on; the reload turns it on too and changes its host.
fn proxy_race() -> ConfigRace {
    ConfigRace {
        text: format!("{ROOT}{PROXY_OFF_OLD}"),
        outside: format!("{ROOT}{PROXY_ON_NEW}"),
        target: ConfigTarget::Collection,
        section: ConfigSection::Network,
        landed: |o, now| network_of(o).proxy != network_of(now).proxy,
        edit: |cx, ed, win| {
            let network = ed.read(cx).network().unwrap().clone();
            cx.update_window(win, |_, w, cx| {
                network.update(cx, |n, cx| {
                    n.set_choice(apinata_app::network_section::ProxyChoice::Custom, w, cx)
                })
            })
            .unwrap();
        },
        kept: "hostname: new.local",
        edited: "inherit: false\n    config:",
        watch: Some(watch_network),
        held: true,
    }
}

#[gpui_kit::test]
fn a_proxy_edit_from_values_a_reload_replaced_keeps_the_outside_change(cx: &mut TestAppContext) {
    proxy_race().race(cx);
}

#[gpui_kit::test]
fn a_proxy_edit_after_a_settled_reload_lands(cx: &mut TestAppContext) {
    proxy_race().plain(cx);
}

// The user removes host a.test; the reload removes it too and changes b.test's address.
fn host_override_race() -> ConfigRace {
    let hosts = |rows: &str| format!("{ROOT}extensions:\n  apinata:\n    hostOverrides:\n{rows}");
    ConfigRace {
        text: hosts(
            "      - host: a.test\n        address: 127.0.0.1\n      - host: b.test\n        address: 127.0.0.2\n",
        ),
        outside: hosts("      - host: b.test\n        address: 127.0.0.9\n"),
        target: ConfigTarget::Collection,
        section: ConfigSection::Network,
        landed: |o, now| network_of(o).host_overrides != network_of(now).host_overrides,
        edit: |cx, ed, win| {
            let network = ed.read(cx).network().unwrap().clone();
            cx.update_window(win, |_, w, cx| {
                network.update(cx, |n, cx| n.remove_host_override(0, w, cx))
            })
            .unwrap();
        },
        kept: "127.0.0.9",
        edited: "hostOverrides:\n      - host: b.test",
        watch: Some(watch_network),
        held: true,
    }
}

#[gpui_kit::test]
fn a_host_override_edit_from_rows_a_reload_replaced_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    host_override_race().race(cx);
}

#[gpui_kit::test]
fn a_host_override_edit_after_a_settled_reload_lands(cx: &mut TestAppContext) {
    host_override_race().plain(cx);
}

// The user removes the a.test certificate; the reload removes it too and changes b.test's file.
fn certificate_race() -> ConfigRace {
    let cert = |domain: &str, file: &str| {
        format!(
            "    - domain: {domain}\n      type: pem\n      certificateFilePath: {file}.pem\n      privateKeyFilePath: {file}.key\n"
        )
    };
    ConfigRace {
        text: format!(
            "{ROOT}config:\n  clientCertificates:\n{}{}",
            cert("a.test", "a"),
            cert("b.test", "b")
        ),
        outside: format!(
            "{ROOT}config:\n  clientCertificates:\n{}",
            cert("b.test", "b2")
        ),
        target: ConfigTarget::Collection,
        section: ConfigSection::Network,
        landed: |o, now| network_of(o).certificates != network_of(now).certificates,
        edit: |cx, ed, win| {
            let network = ed.read(cx).network().unwrap().clone();
            cx.update_window(win, |_, w, cx| {
                network.update(cx, |n, cx| n.remove_certificate(0, w, cx))
            })
            .unwrap();
        },
        kept: "b2.pem",
        edited: "clientCertificates:\n    - domain: b.test",
        watch: Some(watch_network),
        held: true,
    }
}

#[gpui_kit::test]
fn a_certificate_edit_from_rows_a_reload_replaced_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    certificate_race().race(cx);
}

#[gpui_kit::test]
fn a_certificate_edit_after_a_settled_reload_lands(cx: &mut TestAppContext) {
    certificate_race().plain(cx);
}

fn presets(request_type: &str, url: &str) -> String {
    format!(
        "{ROOT}extensions:\n  bruno:\n    presets:\n      request:\n        type: {request_type}\n        url: {url}\n"
    )
}

// The user picks GraphQL; the reload picks it too and changes the URL.
fn presets_race() -> ConfigRace {
    ConfigRace {
        text: presets("http", "https://old.test"),
        outside: presets("graphql", "https://new.test"),
        target: ConfigTarget::Collection,
        section: ConfigSection::Presets,
        landed: |o, now| o.presets != now.presets,
        edit: |cx, _, win| {
            use gpui_kit::test::TestWindowExt as _;
            cx.update_window(win, |_, w, cx| w.within("presets-type").click(1usize, cx))
                .unwrap();
        },
        kept: "https://new.test",
        edited: "type: graphql",
        watch: Some(|cx, ed, l| {
            let presets = ed.read(cx).presets().unwrap().clone();
            watch(cx, presets, l, |s, e, cx| s.lags(e, cx))
        }),
        held: true,
    }
}

#[gpui_kit::test]
fn a_presets_edit_from_values_a_reload_replaced_keeps_the_outside_change(cx: &mut TestAppContext) {
    presets_race().race(cx);
}

#[gpui_kit::test]
fn a_presets_edit_after_a_settled_reload_lands(cx: &mut TestAppContext) {
    presets_race().plain(cx);
}

// A collection-settings edit that only the plain test exercises: it lands after a settled
// reload, sending `edited`, and no edit it sends counts as lagging.
fn plain_only(
    text: String,
    section: ConfigSection,
    edit: OnEditor,
    edited: &'static str,
    watch: Watch,
) -> ConfigRace {
    ConfigRace {
        outside: text.clone(),
        text,
        target: ConfigTarget::Collection,
        section,
        landed: |_, _| false,
        edit,
        kept: "",
        edited,
        watch: Some(watch),
        held: false,
    }
}

const PROXY_CUSTOM: &str = "config:\n  proxy:\n    inherit: false\n    config:\n      protocol: http\n      hostname: h.local\n      port: \"3128\"\n      auth:\n        username: u\n        password: \"\"\n      bypassProxy: \"\"\n";

// Sets the text of a network input without the Enter or blur that would commit it.
fn set_uncommitted(
    cx: &mut gpui_kit::App,
    win: AnyWindowHandle,
    input: Entity<gpui_kit::component::input::InputState>,
    text: &str,
) {
    cx.update_window(win, |_, w, cx| {
        input.update(cx, |i, cx| i.set_value(text.to_string(), w, cx))
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_proxy_choice_after_an_uncommitted_password_lands_after_a_settled_reload(
    cx: &mut TestAppContext,
) {
    plain_only(
        format!("{ROOT}{PROXY_CUSTOM}"),
        ConfigSection::Network,
        |cx, ed, win| {
            use apinata_app::network_section::{NetworkField, ProxyChoice};
            let network = ed.read(cx).network().unwrap().clone();
            let password = network.read(cx).input(NetworkField::Password).clone();
            set_uncommitted(cx, win, password, "{{pw}}");
            cx.update_window(win, |_, w, cx| {
                network.update(cx, |n, cx| n.set_choice(ProxyChoice::Off, w, cx))
            })
            .unwrap();
        },
        "{{pw}}",
        watch_network,
    )
    .plain(cx);
}

#[gpui_kit::test]
fn moving_secrets_with_two_uncommitted_passphrases_lands_after_a_settled_reload(
    cx: &mut TestAppContext,
) {
    let cert = |domain: &str| {
        format!(
            "    - domain: {domain}\n      type: pem\n      certificateFilePath: {domain}.pem\n      privateKeyFilePath: {domain}.key\n      passphrase: \"{{{{old}}}}\"\n"
        )
    };
    // The literal proxy password offers the move; the passphrases are templates already.
    let proxy = PROXY_CUSTOM.replace("password: \"\"", "password: hunter2");
    let text = format!(
        "{ROOT}{proxy}  clientCertificates:\n{}{}",
        cert("a.test"),
        cert("b.test")
    );
    plain_only(
        text,
        ConfigSection::Network,
        |cx, ed, win| {
            use apinata_app::network_section::CertField;
            use gpui_kit::test::TestWindowExt as _;
            let network = ed.read(cx).network().unwrap().clone();
            for (row, text) in [(0, "{{p1}}"), (1, "{{p2}}")] {
                let input = network
                    .read(cx)
                    .cert_input(row, CertField::Passphrase)
                    .cloned();
                set_uncommitted(cx, win, input.unwrap(), text);
            }
            cx.update_window(win, |_, w, cx| w.click("network-move-secrets", cx))
                .unwrap();
        },
        "{{p2}}",
        watch_network,
    )
    .plain(cx);
}

#[gpui_kit::test]
fn typing_a_proxy_host_lands_after_a_settled_reload(cx: &mut TestAppContext) {
    plain_only(
        format!("{ROOT}{PROXY_CUSTOM}"),
        ConfigSection::Network,
        |cx, _, win| {
            use gpui_kit::test::TestWindowExt as _;
            cx.update_window(win, |_, w, cx| {
                w.click("proxy-host", cx);
                w.press("end", cx);
                w.input("x", cx);
            })
            .unwrap();
        },
        "hostname: h.localx",
        watch_network,
    )
    .plain(cx);
}

#[gpui_kit::test]
fn typing_a_presets_url_lands_after_a_settled_reload(cx: &mut TestAppContext) {
    plain_only(
        presets("http", "https://old.test"),
        ConfigSection::Presets,
        |cx, _, win| {
            use gpui_kit::test::TestWindowExt as _;
            cx.update_window(win, |_, w, cx| {
                w.click("presets-url", cx);
                w.press("end", cx);
                w.input("x", cx);
            })
            .unwrap();
        },
        "https://old.testx",
        |cx, ed, l| {
            let presets = ed.read(cx).presets().unwrap().clone();
            watch(cx, presets, l, |s, e, cx| s.lags(e, cx))
        },
    )
    .plain(cx);
}

/// Notes what the auth section's `lags`, which reads the edit's load stamp, says on delivery.
fn watch_auth(
    cx: &mut gpui_kit::App,
    ed: &Entity<ConfigEditor>,
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

#[gpui_kit::test]
fn moving_oauth2_secrets_with_two_uncommitted_secrets_lands_after_a_settled_reload(
    cx: &mut TestAppContext,
) {
    let auth = "request:\n  auth:\n    type: oauth2\n    flow: resource_owner_password_credentials\n    accessTokenUrl: http://id/token\n    credentials:\n      clientId: svc\n      clientSecret: old-plain\n    resourceOwner:\n      username: u\n      password: old-pw\n";
    plain_only(
        format!("{ROOT}{auth}"),
        ConfigSection::Auth,
        |cx, ed, win| {
            use apinata_app::oauth2_editor::OAuth2Field;
            use gpui_kit::test::TestWindowExt as _;
            let oauth2 = ed.read(cx).auth().read(cx).oauth2().clone();
            for (f, text) in [
                (OAuth2Field::ClientSecret, "new-plain"),
                (OAuth2Field::Password, "new-pw"),
            ] {
                let input = oauth2.read(cx).input(f).clone();
                set_uncommitted(cx, win, input, text);
            }
            cx.update_window(win, |_, w, cx| w.click("oauth2-move-secrets", cx))
                .unwrap();
        },
        "{{process.env.APINATA_OAUTH2_CLIENT_SECRET}}",
        watch_auth,
    )
    .plain(cx);
}
