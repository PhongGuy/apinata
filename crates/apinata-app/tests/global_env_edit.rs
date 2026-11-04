//! Creating, renaming, deleting and saving global environments from the app.

use crate::support;

use apinata_app::env_table::{EnvRow, EnvTableEvent};
use apinata_app::global_env_editor::open_global_env_editor;
use apinata_app::global_env_selector::{GlobalEnvSelector, manage_global_environments};
use apinata_app::workspace::{GlobalOp, Workspace};
use apinata_core::{EnvVariable, VarValue};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext, WindowOptions};
use support::*;

struct Blank;
impl gpui_kit::Render for Blank {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        gpui_kit::div()
    }
}

fn start(
    cx: &mut TestAppContext,
    files: &[(&str, &str)],
) -> (
    tempfile::TempDir,
    tempfile::TempDir,
    tempfile::TempDir,
    Entity<Workspace>,
) {
    let col = collection();
    let globals = write_tree(files);
    let (state, ws, _root) = open_dir(cx, &col);
    let dir = globals.path().to_path_buf();
    ws.update(cx, |w, cx| w.set_global_dir(dir, cx));
    cx.run_until_parked();
    (col, globals, state, ws)
}

fn edit(cx: &mut TestAppContext, ws: &Entity<Workspace>, op: GlobalOp) {
    ws.update(cx, |w, cx| w.edit_globals(op, cx));
    cx.run_until_parked();
}

fn names(cx: &mut TestAppContext, ws: &Entity<Workspace>) -> Vec<String> {
    ws.read_with(cx, |w, _| w.global_environments())
}

fn saved_global(cx: &mut TestAppContext, ws: &Entity<Workspace>) -> Option<String> {
    let session = ws.read_with(cx, |w, _| w.session().clone());
    session_value(cx, &session).global_environment
}

#[gpui_kit::test]
fn create_rename_and_delete_carry_the_selection(cx: &mut TestAppContext) {
    let (_c, g, _s, ws) = start(cx, &[("g.yml", "name: g\n")]);
    edit(cx, &ws, GlobalOp::Create("n".into()));
    assert_eq!(names(cx, &ws), ["g", "n"]);
    assert!(g.path().join("n.yml").is_file());
    ws.update(cx, |w, cx| {
        w.select_global_environment(Some("n".into()), cx)
    });
    edit(
        cx,
        &ws,
        GlobalOp::Rename {
            from: "n".into(),
            to: "m".into(),
        },
    );
    assert_eq!(names(cx, &ws), ["g", "m"]);
    assert!(!g.path().join("n.yml").exists() && g.path().join("m.yml").is_file());
    assert_eq!(
        ws.read_with(cx, |w, _| w.selected_global_environment())
            .as_deref(),
        Some("m")
    );
    assert_eq!(saved_global(cx, &ws).as_deref(), Some("m"));
    edit(cx, &ws, GlobalOp::Delete("m".into()));
    assert_eq!(names(cx, &ws), ["g"]);
    assert!(!g.path().join("m.yml").exists());
    assert_eq!(
        ws.read_with(cx, |w, _| w.selected_global_environment()),
        None
    );
    assert_eq!(saved_global(cx, &ws), None);
}

#[gpui_kit::test]
fn rename_points_extending_globals_at_the_new_name(cx: &mut TestAppContext) {
    let (_c, g, _s, ws) = start(
        cx,
        &[
            ("base.yml", "name: base\n"),
            ("dev.yml", "name: dev\nextends: base\n"),
        ],
    );
    edit(
        cx,
        &ws,
        GlobalOp::Rename {
            from: "base".into(),
            to: "root".into(),
        },
    );
    let dev = std::fs::read_to_string(g.path().join("dev.yml")).unwrap();
    assert!(dev.contains("extends: root"), "{dev}");
}

#[gpui_kit::test]
fn a_failed_operation_keeps_the_set_and_reports(cx: &mut TestAppContext) {
    let (_c, _g, _s, ws) = start(cx, &[("g.yml", "name: g\n"), ("h.yml", "name: h\n")]);
    edit(
        cx,
        &ws,
        GlobalOp::Rename {
            from: "g".into(),
            to: "h".into(),
        },
    );
    assert_eq!(names(cx, &ws), ["g", "h"]);
    assert!(ws.read_with(cx, |w, _| w.global_error().is_some()));
    edit(cx, &ws, GlobalOp::Create("k".into()));
    assert!(
        ws.read_with(cx, |w, _| w.global_error().is_none()),
        "a success clears it"
    );
}

#[gpui_kit::test]
fn a_stale_reload_does_not_undo_a_rename(cx: &mut TestAppContext) {
    let (_c, _g, _s, ws) = start(cx, &[("n.yml", "name: n\n")]);
    ws.update(cx, |w, cx| {
        w.select_global_environment(Some("n".into()), cx)
    });
    cx.run_until_parked();
    // Both start before either finishes; the reload began first, so its result is stale.
    ws.update(cx, |w, cx| {
        w.reload_globals(cx);
        w.edit_globals(
            GlobalOp::Rename {
                from: "n".into(),
                to: "m".into(),
            },
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(names(cx, &ws), ["m"]);
    assert_eq!(
        ws.read_with(cx, |w, _| w.selected_global_environment())
            .as_deref(),
        Some("m")
    );
    assert_eq!(saved_global(cx, &ws).as_deref(), Some("m"));
}

#[gpui_kit::test]
fn a_rename_then_an_immediate_save_both_land(cx: &mut TestAppContext) {
    let (_c, g, _s, ws) = start(cx, &[("n.yml", "name: n\n"), ("o.yml", "name: o\n")]);
    ws.update(cx, |w, cx| {
        w.select_global_environment(Some("n".into()), cx)
    });
    cx.run_until_parked();
    // Queued in one update, so the save's reopen must see the rename's write.
    ws.update(cx, |w, cx| {
        w.edit_globals(
            GlobalOp::Rename {
                from: "n".into(),
                to: "m".into(),
            },
            cx,
        );
        let variables = vec![EnvVariable::new("host", VarValue::Text("h".into()))];
        w.edit_globals(
            GlobalOp::Save {
                name: "m".into(),
                variables,
                secrets: Vec::new(),
            },
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(names(cx, &ws), ["m", "o"]);
    assert_eq!(
        ws.read_with(cx, |w, _| w.selected_global_environment())
            .as_deref(),
        Some("m")
    );
    assert_eq!(saved_global(cx, &ws).as_deref(), Some("m"));
    assert!(ws.read_with(cx, |w, _| w.global_error().is_none()));
    let text = std::fs::read_to_string(g.path().join("m.yml")).unwrap();
    assert!(text.contains("value: h"), "{text}");
}

#[gpui_kit::test]
fn save_writes_the_file_and_the_secret_store(cx: &mut TestAppContext) {
    let (_c, g, _s, ws) = start(cx, &[("g.yml", "name: g\n")]);
    let variables = vec![
        EnvVariable::new("host", VarValue::Text("h.example".into())),
        EnvVariable::secret("tok"),
    ];
    let op = GlobalOp::Save {
        name: "g".into(),
        variables,
        secrets: vec![("tok".into(), "s3".into())],
    };
    edit(cx, &ws, op);
    let text = std::fs::read_to_string(g.path().join("g.yml")).unwrap();
    assert!(
        text.contains("h.example") && text.contains("secret: true"),
        "{text}"
    );
    assert!(
        !text.contains("s3"),
        "a secret value never reaches the file"
    );
    let (store, globals) =
        ws.read_with(cx, |w, _| (w.store().clone(), w.globals().expect("loaded")));
    assert_eq!(
        store
            .get(&globals.secret_key("g", "tok"))
            .unwrap()
            .as_deref(),
        Some("s3")
    );
}

#[gpui_kit::test]
fn the_editor_loads_rows_and_saves_them(cx: &mut TestAppContext) {
    support::init(cx);
    let (_c, g, _s, ws) = start(
        cx,
        &[
            (
                "g.yml",
                "name: g\nvariables:\n  - name: host\n    value: a\n",
            ),
            ("broken.yml", "name: [unclosed\n"),
        ],
    );
    let (win, _) = cx
        .update(|cx| gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Blank)))
        .unwrap();
    let shared = ws.clone();
    let editor = cx
        .update_window(win, |_, w, cx| open_global_env_editor(shared, w, cx))
        .unwrap();
    cx.run_until_parked();
    assert!(present(cx, win, "global-env-editor"));
    cx.update_window(win, |_, w, cx| {
        editor.update(cx, |e, cx| e.select("g", w, cx))
    })
    .unwrap();
    assert!(
        present(cx, win, ("global-env-load-error", 0usize)),
        "the broken file is listed"
    );
    let rows = editor.read_with(cx, |e, cx| e.table().read(cx).rows(cx));
    assert_eq!(
        rows.iter()
            .map(|r| (r.name.as_str(), r.value.as_str()))
            .collect::<Vec<_>>(),
        [("host", "a")]
    );
    let mut changed = rows.clone();
    changed[0].value = "b".into();
    cx.update_window(win, |_, w, cx| {
        let table = editor.read(cx).table().clone();
        table.update(cx, |t, cx| t.load(changed, w, cx));
        editor.update(cx, |e, cx| e.save(cx));
    })
    .unwrap();
    cx.run_until_parked();
    let text = std::fs::read_to_string(g.path().join("g.yml")).unwrap();
    assert!(text.contains("value: b"), "{text}");
    // An empty name is invalid, so the rename fails and its message shows in the dialog.
    cx.update_window(win, |_, w, cx| {
        editor.update(cx, |e, cx| e.rename("", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(present(cx, win, "global-env-error"));
}

// A refused operation's message shows with a capital first letter, as other alerts do.
#[gpui_kit::test]
fn a_rename_onto_an_existing_name_shows_a_capitalised_error(cx: &mut TestAppContext) {
    support::init(cx);
    let (_c, _g, _s, ws) = start(cx, &[("g.yml", "name: g\n"), ("h.yml", "name: h\n")]);
    let (win, _editor) = open_editor(cx, &ws, "g");
    edit(
        cx,
        &ws,
        GlobalOp::Rename {
            from: "g".into(),
            to: "h".into(),
        },
    );
    let shown = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find("global-env-error").label().map(str::to_owned)
        })
        .unwrap();
    let shown = shown.expect("the error line is labelled");
    assert!(shown.starts_with("Environment "), "{shown}");
}

#[gpui_kit::test]
fn the_manage_row_opens_the_editor(cx: &mut TestAppContext) {
    support::init(cx);
    let (_c, _g, _s, ws) = start(cx, &[("g.yml", "name: g\n")]);
    let (win, _) = cx
        .update(|cx| gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Blank)))
        .unwrap();
    let shared = ws.clone();
    let sel = cx
        .update_window(win, |_, w, cx| {
            cx.new(|cx| GlobalEnvSelector::new(shared, w, cx))
        })
        .unwrap();
    frame(cx, win);
    let items = sel.read_with(cx, |s, cx| s.items(cx));
    assert_eq!(
        items.last().map(|s| s.as_ref()),
        Some(manage_global_environments())
    );
    cx.update_window(win, |_, w, cx| {
        sel.update(cx, |s, cx| s.choose(manage_global_environments(), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(present(cx, win, "global-env-editor"));
    assert_eq!(
        ws.read_with(cx, |w, _| w.selected_global_environment()),
        None,
        "managing selects nothing"
    );
}

#[gpui_kit::test]
fn toggling_secret_moves_the_value_between_file_and_store(cx: &mut TestAppContext) {
    let (_c, g, _s, ws) = start(
        cx,
        &[(
            "g.yml",
            "name: g\nvariables:\n  - name: tok\n    value: s3\n",
        )],
    );
    let op = |secret| GlobalOp::SetSecret {
        name: "g".into(),
        variable: "tok".into(),
        secret,
    };
    edit(cx, &ws, op(true));
    assert!(ws.read_with(cx, |w, _| w.global_error().is_none()));
    let text = std::fs::read_to_string(g.path().join("g.yml")).unwrap();
    assert!(
        text.contains("secret: true") && !text.contains("s3"),
        "{text}"
    );
    let (store, globals) =
        ws.read_with(cx, |w, _| (w.store().clone(), w.globals().expect("loaded")));
    assert_eq!(
        store
            .get(&globals.secret_key("g", "tok"))
            .unwrap()
            .as_deref(),
        Some("s3")
    );
    edit(cx, &ws, op(false));
    let text = std::fs::read_to_string(g.path().join("g.yml")).unwrap();
    assert!(
        text.contains("value: s3") && !text.contains("secret: true"),
        "{text}"
    );
}

#[gpui_kit::test]
fn the_editor_reveals_a_stored_global_secret(cx: &mut TestAppContext) {
    support::init(cx);
    let (_c, _g, _s, ws) = start(
        cx,
        &[(
            "g.yml",
            "name: g\nvariables:\n  - name: tok\n    secret: true\n",
        )],
    );
    let (store, globals) =
        ws.read_with(cx, |w, _| (w.store().clone(), w.globals().expect("loaded")));
    store.set(&globals.secret_key("g", "tok"), "s3").unwrap();
    let (win, _) = cx
        .update(|cx| gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Blank)))
        .unwrap();
    let shared = ws.clone();
    let editor = cx
        .update_window(win, |_, w, cx| open_global_env_editor(shared, w, cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        editor.update(cx, |e, cx| e.select("g", w, cx))
    })
    .unwrap();
    cx.update_window(win, |_, w, cx| {
        editor.update(cx, |e, cx| e.reveal("tok", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    let table = editor.read_with(cx, |e, _| e.table().clone());
    assert!(table.read_with(cx, |t, _| t.revealed(0)));
    let shown = table.read_with(cx, |t, cx| {
        t.value_input(0).unwrap().read(cx).value().to_string()
    });
    assert_eq!(shown, "s3");
}

// Random scheduling lets the reload's read land before the rename's write in some iterations.
#[gpui_kit::test(iterations = 50)]
fn a_reload_started_during_an_edit_does_not_undo_it(cx: &mut TestAppContext) {
    let (_c, _g, _s, ws) = start(cx, &[("n.yml", "name: n\n")]);
    ws.update(cx, |w, cx| {
        w.select_global_environment(Some("n".into()), cx)
    });
    cx.run_until_parked();
    ws.update(cx, |w, cx| {
        w.edit_globals(
            GlobalOp::Rename {
                from: "n".into(),
                to: "m".into(),
            },
            cx,
        );
        w.reload_globals(cx);
    });
    cx.run_until_parked();
    assert_eq!(names(cx, &ws), ["m"]);
    assert_eq!(
        ws.read_with(cx, |w, _| w.selected_global_environment())
            .as_deref(),
        Some("m")
    );
    assert_eq!(saved_global(cx, &ws).as_deref(), Some("m"));
}

#[test]
fn a_save_op_never_prints_its_secret_values() {
    let op = GlobalOp::Save {
        name: "g".into(),
        variables: vec![EnvVariable::secret("tok")],
        secrets: vec![("tok".into(), "sentinel-s3cr3t".into())],
    };
    let printed = format!("{op:?}");
    assert!(printed.contains("tok"), "{printed}");
    assert!(!printed.contains("sentinel-s3cr3t"), "{printed}");
}

// Opens the editor over a blank window with `env` selected.
fn open_editor(
    cx: &mut TestAppContext,
    ws: &Entity<Workspace>,
    env: &str,
) -> (
    gpui_kit::AnyWindowHandle,
    Entity<apinata_app::global_env_editor::GlobalEnvEditor>,
) {
    let (win, _) = cx
        .update(|cx| gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Blank)))
        .unwrap();
    let shared = ws.clone();
    let editor = cx
        .update_window(win, |_, w, cx| open_global_env_editor(shared, w, cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        editor.update(cx, |e, cx| e.select(env, w, cx))
    })
    .unwrap();
    (win, editor)
}

fn toggle_secret(
    cx: &mut TestAppContext,
    editor: &Entity<apinata_app::global_env_editor::GlobalEnvEditor>,
    name: &str,
) {
    let table = editor.read_with(cx, |e, _| e.table().clone());
    table.update(cx, |_, cx| {
        cx.emit(EnvTableEvent::SetSecret {
            name: name.into(),
            secret: true,
        })
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn toggling_secret_keeps_other_rows_unsaved_edits(cx: &mut TestAppContext) {
    support::init(cx);
    let (_c, g, _s, ws) = start(
        cx,
        &[(
            "g.yml",
            "name: g\nvariables:\n  - name: host\n    value: a\n  - name: tok\n    value: s3\n",
        )],
    );
    let (win, editor) = open_editor(cx, &ws, "g");
    let mut rows = editor.read_with(cx, |e, cx| e.table().read(cx).rows(cx));
    rows[0].value = "b".into();
    cx.update_window(win, |_, w, cx| {
        let table = editor.read(cx).table().clone();
        table.update(cx, |t, cx| t.load(rows, w, cx));
    })
    .unwrap();
    toggle_secret(cx, &editor, "tok");
    assert!(ws.read_with(cx, |w, _| w.global_error().is_none()));
    let rows = editor.read_with(cx, |e, cx| e.table().read(cx).rows(cx));
    assert_eq!(rows[0].value, "b", "the typing in host survives the toggle");
    assert!(rows[1].secret);
    let text = std::fs::read_to_string(g.path().join("g.yml")).unwrap();
    assert!(
        text.contains("value: b") && text.contains("secret: true") && !text.contains("s3"),
        "{text}"
    );
}

#[gpui_kit::test]
fn toggling_secret_on_a_row_not_yet_saved_succeeds(cx: &mut TestAppContext) {
    support::init(cx);
    let (_c, g, _s, ws) = start(
        cx,
        &[(
            "g.yml",
            "name: g\nvariables:\n  - name: host\n    value: a\n",
        )],
    );
    let (win, editor) = open_editor(cx, &ws, "g");
    let mut rows = editor.read_with(cx, |e, cx| e.table().read(cx).rows(cx));
    rows.push(EnvRow {
        name: "tok".into(),
        value: "s3".into(),
        enabled: true,
        ..EnvRow::default()
    });
    cx.update_window(win, |_, w, cx| {
        let table = editor.read(cx).table().clone();
        table.update(cx, |t, cx| t.load(rows, w, cx));
    })
    .unwrap();
    toggle_secret(cx, &editor, "tok");
    assert_eq!(
        ws.read_with(cx, |w, _| w.global_error().map(str::to_owned)),
        None
    );
    let text = std::fs::read_to_string(g.path().join("g.yml")).unwrap();
    assert!(
        text.contains("name: tok") && text.contains("secret: true") && !text.contains("s3"),
        "{text}"
    );
    let (store, globals) =
        ws.read_with(cx, |w, _| (w.store().clone(), w.globals().expect("loaded")));
    assert_eq!(
        store
            .get(&globals.secret_key("g", "tok"))
            .unwrap()
            .as_deref(),
        Some("s3")
    );
}

#[gpui_kit::test]
fn a_failed_save_keeps_the_typed_secrets_for_the_next(cx: &mut TestAppContext) {
    support::init(cx);
    let file = "name: g\nvariables:\n  - name: tok\n    secret: true\n";
    let (_c, g, _s, ws) = start(cx, &[("g.yml", file)]);
    let (win, editor) = open_editor(cx, &ws, "g");
    let table = editor.read_with(cx, |e, _| e.table().clone());
    cx.update_window(win, |_, w, cx| {
        let input = table.read(cx).value_input(0).unwrap().clone();
        input.update(cx, |s, cx| s.set_value("s3", w, cx));
    })
    .unwrap();
    // The file is gone when the save runs, so it fails; it is back for the retry.
    let path = g.path().join("g.yml");
    std::fs::remove_file(&path).unwrap();
    editor.update(cx, |e, cx| e.save(cx));
    cx.run_until_parked();
    assert!(ws.read_with(cx, |w, _| w.global_error().is_some()));
    std::fs::write(&path, file).unwrap();
    editor.update(cx, |e, cx| e.save(cx));
    cx.run_until_parked();
    assert_eq!(
        ws.read_with(cx, |w, _| w.global_error().map(str::to_owned)),
        None
    );
    let (store, globals) =
        ws.read_with(cx, |w, _| (w.store().clone(), w.globals().expect("loaded")));
    assert_eq!(
        store
            .get(&globals.secret_key("g", "tok"))
            .unwrap()
            .as_deref(),
        Some("s3")
    );
}

// Enter in the name input must commit the name, not confirm (and close) the dialog.
#[gpui_kit::test]
fn enter_in_the_name_input_commits_and_keeps_the_dialog(cx: &mut TestAppContext) {
    support::init(cx);
    let (_c, g, _s, ws) = start(cx, &[("staging.yml", "name: staging\n")]);
    let (win, editor) = open_editor(cx, &ws, "staging");
    let name = |cx: &mut TestAppContext, keys: &str| {
        cx.simulate_keystrokes(win, &secondary("a"));
        cx.simulate_input(win, keys);
        cx.simulate_keystrokes(win, "enter");
        cx.run_until_parked();
    };
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click("global-env-rename", cx);
    })
    .unwrap();
    name(cx, "qa");
    assert!(
        present(cx, win, "global-env-editor"),
        "Rename keeps the dialog"
    );
    assert_eq!(names(cx, &ws), ["qa"]);
    assert!(g.path().join("qa.yml").is_file());
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click("global-env-create", cx);
    })
    .unwrap();
    name(cx, "prod");
    assert!(
        present(cx, win, "global-env-editor"),
        "New keeps the dialog"
    );
    assert_eq!(names(cx, &ws), ["prod", "qa"]);
    assert_eq!(
        editor
            .read_with(cx, |e, _| e.selected().map(str::to_owned))
            .as_deref(),
        Some("prod")
    );
}

#[gpui_kit::test]
fn the_manage_row_fits_the_picker(cx: &mut TestAppContext) {
    support::init(cx);
    let (_c, _g, _s, ws) = start(cx, &[("g.yml", "name: g\n")]);
    let shared = ws.clone();
    let (win, _) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| GlobalEnvSelector::new(shared, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click("global-env-select", cx);
    })
    .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let picker = w.find("global-env-selector").bounds();
        let row = w.find("global-env-manage").bounds();
        // The row's box is cut to the menu, so the label's own width, shaped at the menu's
        // small text size, must fit it.
        let style = w.text_style();
        let size = gpui_kit::rems(0.875).to_pixels(w.rem_size());
        let run = style.to_run(manage_global_environments().len());
        let label = w
            .text_system()
            .shape_line(manage_global_environments().into(), size, &[run], None)
            .width;
        assert!(
            row.right() <= picker.right(),
            "row {row:?} picker {picker:?}"
        );
        assert!(label <= row.size.width, "label {label:?} row {row:?}");
    })
    .unwrap();
}

// Committing a name hides its input; focus stays in the dialog so Escape still closes it.
#[gpui_kit::test]
fn escape_closes_the_dialog_after_a_name_is_committed(cx: &mut TestAppContext) {
    support::init(cx);
    let (_c, _g, _s, ws) = start(cx, &[("staging.yml", "name: staging\n")]);
    let (win, _editor) = open_editor(cx, &ws, "staging");
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click("global-env-create", cx);
    })
    .unwrap();
    cx.simulate_input(win, "qa");
    cx.simulate_keystrokes(win, "enter");
    cx.run_until_parked();
    assert_eq!(names(cx, &ws), ["qa", "staging"]);
    assert!(present(cx, win, "global-env-editor"));
    cx.simulate_keystrokes(win, "escape");
    cx.run_until_parked();
    assert!(
        !present(cx, win, "global-env-editor"),
        "Escape closes the dialog"
    );
}

// The global editor shares the table: lock in the value cell, no colour or parent controls.
#[gpui_kit::test]
fn the_global_table_has_the_lock_but_no_colour_or_parent(cx: &mut TestAppContext) {
    support::init(cx);
    let vars =
        "name: g\nvariables:\n  - name: host\n    value: a\n  - secret: true\n    name: token\n";
    let (_c, _g, _s, ws) = start(cx, &[("g.yml", vars)]);
    let (win, _editor) = open_editor(cx, &ws, "g");
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        for (ix, secret) in [(0usize, false), (1, true)] {
            let cell = w.find(("env-value-cell", ix)).bounds();
            let lock = w.find(("env-secret", ix));
            assert_eq!(lock.checked(), Some(secret));
            assert!(lock.bounds().right() <= cell.right());
        }
        assert!(w.try_find("env-swatch-none").is_none() && w.try_find("env-extends").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_duplicate_button_copies_the_selected_global_environment(cx: &mut TestAppContext) {
    support::init(cx);
    let body = "name: g\nvariables:\n  - name: host\n    value: a\n";
    let (_c, g, _s, ws) = start(cx, &[("g.yml", body)]);
    let (win, _) = cx
        .update(|cx| gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Blank)))
        .unwrap();
    let shared = ws.clone();
    let editor = cx
        .update_window(win, |_, w, cx| open_global_env_editor(shared, w, cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        editor.update(cx, |e, cx| e.select("g", w, cx))
    })
    .unwrap();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        assert_eq!(w.find("global-env-duplicate").label(), Some("Duplicate"));
        w.click("global-env-duplicate", cx)
    })
    .unwrap();
    wait_for(cx, "copy selected", |cx| {
        editor
            .read_with(cx, |e, _| e.selected().map(str::to_owned))
            .as_deref()
            == Some("g copy")
    });
    assert_eq!(
        std::fs::read_to_string(g.path().join("g copy.yml")).expect("copy"),
        body.replacen("name: g\n", "name: g copy\n", 1)
    );
}

#[gpui_kit::test]
fn enter_on_the_focused_duplicate_button_copies_the_environment(cx: &mut TestAppContext) {
    support::init(cx);
    let (_c, g, _s, ws) = start(cx, &[("g.yml", "name: g\n")]);
    let (win, editor) = open_editor(cx, &ws, "g");
    cx.update(apinata_app::keymap::init);
    enter_on(cx, win, "global-env-duplicate");
    wait_for(cx, "copy selected", |cx| {
        editor
            .read_with(cx, |e, _| e.selected().map(str::to_owned))
            .as_deref()
            == Some("g copy")
    });
    assert!(g.path().join("g copy.yml").is_file());
    assert!(present(cx, win, "global-env-editor"));
}

#[gpui_kit::test]
fn enter_on_the_focused_new_button_asks_for_a_name(cx: &mut TestAppContext) {
    support::init(cx);
    let (_c, _g, _s, ws) = start(cx, &[("g.yml", "name: g\n")]);
    let (win, _editor) = open_editor(cx, &ws, "g");
    cx.update(apinata_app::keymap::init);
    enter_on(cx, win, "global-env-create");
    frame(cx, win);
    assert!(
        present(cx, win, "global-env-name-input"),
        "Enter pressed New"
    );
}

#[gpui_kit::test]
fn deleting_an_extended_global_environment_offers_only_cancel(cx: &mut TestAppContext) {
    support::init(cx);
    let (_c, g, _s, ws) = start(
        cx,
        &[
            ("base.yml", "name: base\n"),
            ("dev.yml", "name: dev\nextends: base\n"),
        ],
    );
    let (win, _editor) = open_editor(cx, &ws, "base");
    let shown = |cx: &mut TestAppContext, id: &'static str| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find(id).is_some()
        })
        .unwrap()
    };
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click("global-env-delete", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert!(shown(cx, "confirm-cancel"));
    assert!(!shown(cx, "confirm-delete"));
    assert!(shown(cx, "confirm-detail"));
    let title = cx
        .update_window(win, |_, w, _| {
            w.find("confirm-dialog").label().map(str::to_owned)
        })
        .unwrap();
    assert_eq!(
        title.as_deref(),
        Some("Global environment \"base\" is extended by \"dev\"")
    );
    answer_confirm(cx, "confirm-cancel");
    assert!(g.path().join("base.yml").is_file());
}

#[gpui_kit::test]
fn deleting_an_extended_global_environment_reports_the_refusal_and_keeps_the_file(
    cx: &mut TestAppContext,
) {
    let (_c, g, _s, ws) = start(
        cx,
        &[
            ("base.yml", "name: base\n"),
            ("dev.yml", "name: dev\nextends: base\n"),
        ],
    );
    edit(cx, &ws, GlobalOp::Delete("base".into()));
    let error = ws.read_with(cx, |w, _| w.global_error().map(str::to_owned));
    assert!(
        error.as_deref().is_some_and(|e| e.contains("`dev`")),
        "{error:?}"
    );
    assert!(g.path().join("base.yml").is_file());
    assert_eq!(names(cx, &ws), ["base", "dev"]);
}

const TOK: &str = "name: g\nvariables:\n  - name: tok\n    secret: true\n";

// `g` open with `typed` in its secret `tok`, neither committed nor saved, after `files` were
// rewritten outside and the globals reloaded. Returns the editor and the input's text.
fn typed_then_reloaded(
    cx: &mut TestAppContext,
    files: &[(&str, &str)],
) -> (
    tempfile::TempDir,
    Entity<Workspace>,
    Entity<apinata_app::global_env_editor::GlobalEnvEditor>,
    String,
) {
    support::init(cx);
    let (_c, g, _s, ws) = start(cx, &[("g.yml", TOK), ("h.yml", "name: h\n")]);
    let (win, editor) = open_editor(cx, &ws, "g");
    cx.run_until_parked();
    let table = editor.read_with(cx, |e, _| e.table().clone());
    let input = table.read_with(cx, |t, _| t.value_input(0).unwrap().clone());
    cx.update_window(win, |_, w, cx| {
        input.update(cx, |s, cx| s.set_value("typed", w, cx))
    })
    .unwrap();
    for (rel, text) in files {
        std::fs::write(g.path().join(rel), text).unwrap();
    }
    ws.update(cx, |w, cx| w.reload_globals(cx));
    cx.run_until_parked();
    let table = editor.read_with(cx, |e, _| e.table().clone());
    let shown = table.read_with(cx, |t, cx| {
        t.value_input(0).unwrap().read(cx).value().to_string()
    });
    (g, ws, editor, shown)
}

fn var_count(cx: &mut TestAppContext, ws: &Entity<Workspace>, env: &str) -> usize {
    ws.read_with(cx, |w, _| {
        let globals = w.globals().expect("loaded");
        globals.environment(env).map_or(0, |e| e.variables().len())
    })
}

fn stored_tok(cx: &mut TestAppContext, ws: &Entity<Workspace>) -> Option<String> {
    let (store, globals) =
        ws.read_with(cx, |w, _| (w.store().clone(), w.globals().expect("loaded")));
    store.get(&globals.secret_key("g", "tok")).unwrap()
}

#[gpui_kit::test]
fn unsent_global_secret_typing_survives_a_change_to_another_variable(cx: &mut TestAppContext) {
    let more = format!("{TOK}  - name: host\n    value: a\n");
    let (_g, ws, editor, shown) = typed_then_reloaded(cx, &[("g.yml", &more)]);
    assert_eq!(var_count(cx, &ws, "g"), 2, "the reload landed");
    assert_eq!(shown, "typed", "the reload replaced the typing");
    editor.update(cx, |e, cx| e.save(cx));
    cx.run_until_parked();
    assert_eq!(stored_tok(cx, &ws).as_deref(), Some("typed"));
}

#[gpui_kit::test]
fn unsent_global_secret_typing_survives_a_change_to_another_file(cx: &mut TestAppContext) {
    let h = "name: h\nvariables:\n  - name: host\n    value: b\n";
    let (_g, ws, editor, shown) = typed_then_reloaded(cx, &[("h.yml", h)]);
    assert_eq!(var_count(cx, &ws, "h"), 1, "the reload landed");
    assert_eq!(shown, "typed", "the reload replaced the typing");
    editor.update(cx, |e, cx| e.save(cx));
    cx.run_until_parked();
    assert_eq!(stored_tok(cx, &ws).as_deref(), Some("typed"));
}

// Renamed on disk, the secret the typing belonged to is gone, so the typing goes with it.
#[gpui_kit::test]
fn an_outside_rename_of_the_global_secret_replaces_the_typing(cx: &mut TestAppContext) {
    let renamed = TOK.replace("name: tok", "name: key");
    let (_g, _ws, _editor, shown) = typed_then_reloaded(cx, &[("g.yml", &renamed)]);
    assert_eq!(shown, "");
}

// A global environment's value in the open editor's first row.
fn first_value(
    cx: &mut TestAppContext,
    editor: &Entity<apinata_app::global_env_editor::GlobalEnvEditor>,
) -> String {
    let rows = editor.read_with(cx, |e, cx| e.table().read(cx).rows(cx));
    rows.first().map(|r| r.value.clone()).unwrap_or_default()
}

const HOST_A: &str = "name: g\nvariables:\n  - name: host\n    value: a\n";
const HOST_OUTSIDE: &str = "name: g\nvariables:\n  - name: host\n    value: outside\n";

// The watched global directory carries another program's rewrite into the open editor.
#[gpui_kit::test]
fn an_outside_edit_shows_in_the_open_global_editor_without_a_click(cx: &mut TestAppContext) {
    support::init(cx);
    cx.executor().allow_parking();
    let (_c, g, _s, ws) = start(cx, &[("g.yml", HOST_A)]);
    ws.update(cx, |w, cx| {
        w.set_watch_factory(Some(apinata_app::watch::set::notify_factory()), cx)
    });
    let (_win, editor) = open_editor(cx, &ws, "g");
    assert_eq!(first_value(cx, &editor), "a");
    settle(cx);
    std::fs::write(g.path().join("g.yml"), HOST_OUTSIDE).unwrap();
    wait_for(cx, "the outside value in the global editor", |cx| {
        first_value(cx, &editor) == "outside"
    });
}

// The watcher's reload keeps a secret's unsent typing through a change to another variable.
#[gpui_kit::test]
fn a_watched_outside_edit_keeps_unsent_global_secret_typing(cx: &mut TestAppContext) {
    support::init(cx);
    cx.executor().allow_parking();
    let (_c, g, _s, ws) = start(cx, &[("g.yml", TOK)]);
    ws.update(cx, |w, cx| {
        w.set_watch_factory(Some(apinata_app::watch::set::notify_factory()), cx)
    });
    let (win, editor) = open_editor(cx, &ws, "g");
    settle(cx);
    let table = editor.read_with(cx, |e, _| e.table().clone());
    let input = table.read_with(cx, |t, _| t.value_input(0).unwrap().clone());
    cx.update_window(win, |_, w, cx| {
        input.update(cx, |s, cx| s.set_value("typed", w, cx))
    })
    .unwrap();
    let more = format!("{TOK}  - name: host\n    value: a\n");
    std::fs::write(g.path().join("g.yml"), more).unwrap();
    wait_for(cx, "the outside variable in the global editor", |cx| {
        table.read_with(cx, |t, cx| t.rows(cx).len()) == 2
    });
    let shown = input.read_with(cx, |i, _| i.value().to_string());
    assert_eq!(shown, "typed");
}

// `g` open under the watched global directory with `typed` over its plain `host`, after
// another program rewrote the file to `outside` and the banner rose.
fn plain_typing_under_a_watched_edit(
    cx: &mut TestAppContext,
) -> (
    tempfile::TempDir,
    gpui_kit::AnyWindowHandle,
    Entity<apinata_app::global_env_editor::GlobalEnvEditor>,
) {
    support::init(cx);
    cx.executor().allow_parking();
    let (_c, g, _s, ws) = start(cx, &[("g.yml", HOST_A)]);
    ws.update(cx, |w, cx| {
        w.set_watch_factory(Some(apinata_app::watch::set::notify_factory()), cx)
    });
    let (win, editor) = open_editor(cx, &ws, "g");
    settle(cx);
    let table = editor.read_with(cx, |e, _| e.table().clone());
    let input = table.read_with(cx, |t, _| t.value_input(0).unwrap().clone());
    cx.update_window(win, |_, w, cx| {
        input.update(cx, |s, cx| s.set_value("typed", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    std::fs::write(g.path().join("g.yml"), HOST_OUTSIDE).unwrap();
    wait_for(cx, "the changed-on-disk banner", |cx| {
        present(cx, win, "banner-changed")
    });
    (g, win, editor)
}

// The watcher's reload under plain typing keeps the draft and says the file changed.
#[gpui_kit::test]
fn a_watched_outside_edit_keeps_plain_global_typing_under_the_banner(cx: &mut TestAppContext) {
    let (_g, _win, editor) = plain_typing_under_a_watched_edit(cx);
    assert_eq!(first_value(cx, &editor), "typed");
    assert!(editor.read_with(cx, |e, cx| e.has_edits(cx)));
}

// The banner's Reload takes the file's values and drops the typing.
#[gpui_kit::test]
fn the_global_banner_reload_takes_the_outside_values(cx: &mut TestAppContext) {
    let (_g, win, editor) = plain_typing_under_a_watched_edit(cx);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click("banner-reload", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(first_value(cx, &editor), "outside");
    assert!(!present(cx, win, "banner-changed"));
}

// The banner's Overwrite writes the typing over the outside edit.
#[gpui_kit::test]
fn the_global_banner_overwrite_saves_the_typing(cx: &mut TestAppContext) {
    let (g, win, editor) = plain_typing_under_a_watched_edit(cx);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click("banner-overwrite", cx);
    })
    .unwrap();
    wait_for(cx, "the typing on disk", |_| {
        std::fs::read_to_string(g.path().join("g.yml")).is_ok_and(|t| t.contains("typed"))
    });
    settle(cx);
    assert_eq!(first_value(cx, &editor), "typed");
    assert!(!present(cx, win, "banner-changed"));
}

// A global directory that cannot be watched says so, as the collection beside it does.
#[gpui_kit::test]
fn a_global_watch_that_cannot_start_is_a_watch_failure(cx: &mut TestAppContext) {
    let (_c, g, _s, ws) = start(cx, &[("g.yml", HOST_A)]);
    let seen = events(cx, &ws);
    let broken: apinata_app::watch::set::WatchFactory =
        std::sync::Arc::new(|_| Err("no inotify".to_string()));
    ws.update(cx, |w, cx| w.set_watch_factory(Some(broken), cx));
    cx.run_until_parked();
    let failed: Vec<_> = seen
        .borrow()
        .iter()
        .filter(|e| {
            matches!(e, apinata_app::workspace::WorkspaceEvent::WatchFailed { root, .. }
                if root == g.path())
        })
        .cloned()
        .collect();
    assert_eq!(
        failed,
        [apinata_app::workspace::WorkspaceEvent::WatchFailed {
            root: g.path().to_path_buf(),
            message: "no inotify".into()
        }]
    );
}

// An edit between the first load and the watch being placed has no event, so placing the
// watch reads the directory once.
#[gpui_kit::test]
fn placing_the_global_watch_reads_an_edit_made_before_it(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (_c, g, _s, ws) = start(cx, &[("g.yml", HOST_A)]);
    std::fs::write(g.path().join("g.yml"), HOST_OUTSIDE).unwrap();
    ws.update(cx, |w, cx| {
        w.set_watch_factory(Some(apinata_app::watch::set::notify_factory()), cx)
    });
    cx.run_until_parked();
    let value = ws.read_with(cx, |w, _| {
        let globals = w.globals().expect("loaded");
        let vars = globals.environment("g").map(|e| e.variables()).unwrap();
        vars[0].value.clone()
    });
    assert!(
        value == VarValue::Text("outside".into()),
        "the outside value was read"
    );
}

// Another program's rewrite reaches the open global editor well inside the watcher's budget.
#[gpui_kit::test]
fn an_outside_edit_reaches_the_open_global_editor_promptly(cx: &mut TestAppContext) {
    support::init(cx);
    cx.executor().allow_parking();
    let (_c, g, _s, ws) = start(cx, &[("g.yml", HOST_A)]);
    ws.update(cx, |w, cx| {
        w.set_watch_factory(Some(apinata_app::watch::set::notify_factory()), cx)
    });
    let (_win, editor) = open_editor(cx, &ws, "g");
    settle(cx);
    let written = std::time::Instant::now();
    std::fs::write(g.path().join("g.yml"), HOST_OUTSIDE).unwrap();
    wait_for(cx, "the outside value in the global editor", |cx| {
        first_value(cx, &editor) == "outside"
    });
    assert!(written.elapsed() < std::time::Duration::from_secs(3));
}

// The app's own reads of the watched global directory never set off another reload, so an
// idle workspace publishes nothing and each collection keeps its snapshot.
#[gpui_kit::test]
fn an_idle_watched_global_directory_reloads_nothing(cx: &mut TestAppContext) {
    support::init(cx);
    cx.executor().allow_parking();
    let (c, _g, _s, ws) = start(cx, &[("g.yml", HOST_A)]);
    ws.update(cx, |w, cx| {
        w.set_watch_factory(Some(apinata_app::watch::set::notify_factory()), cx)
    });
    let root = c.path().to_path_buf();
    settle(cx);
    let before = ws_snap(cx, &ws, &root).seq;
    settle(cx);
    assert_eq!(ws_snap(cx, &ws, &root).seq, before);
}

// A watch that hands each handler to the test, so it can deliver any batch it likes.
struct Quiet;
impl apinata_app::watch::set::Watch for Quiet {
    fn watch(&mut self, _: &std::path::Path) -> Result<(), apinata_app::watch::set::WatchError> {
        Ok(())
    }
    fn unwatch(&mut self, _: &std::path::Path) {}
}

type Handlers = std::sync::Arc<std::sync::Mutex<Vec<apinata_app::watch::set::EventHandler>>>;

fn capturing() -> (apinata_app::watch::set::WatchFactory, Handlers) {
    let handlers: Handlers = Default::default();
    let kept = handlers.clone();
    let factory: apinata_app::watch::set::WatchFactory = std::sync::Arc::new(move |handler| {
        kept.lock().unwrap().push(handler);
        Ok(Box::new(Quiet) as Box<dyn apinata_app::watch::set::Watch>)
    });
    (factory, handlers)
}

fn global_failures(
    seen: &std::cell::RefCell<Vec<apinata_app::workspace::WorkspaceEvent>>,
    dir: &std::path::Path,
) -> Vec<apinata_app::workspace::WorkspaceEvent> {
    (seen.borrow().iter())
        .filter(|e| {
            matches!(e, apinata_app::workspace::WorkspaceEvent::WatchFailed { root, .. }
                if root == dir)
        })
        .cloned()
        .collect()
}

// The watch limit reached after the global watch started stops it and says so, once.
#[gpui_kit::test]
fn a_global_watch_that_fails_later_is_a_watch_failure(cx: &mut TestAppContext) {
    use notify_debouncer_full::notify::{Error, ErrorKind};
    let (_c, g, _s, ws) = start(cx, &[("g.yml", HOST_A)]);
    let seen = events(cx, &ws);
    let (factory, handlers) = capturing();
    ws.update(cx, |w, cx| w.set_watch_factory(Some(factory), cx));
    cx.run_until_parked();
    assert!(global_failures(&seen, g.path()).is_empty());
    let limit = Error::new(ErrorKind::MaxFilesWatch);
    let message = limit.to_string();
    for handler in handlers.lock().unwrap().iter_mut() {
        handler(Err(vec![Error::new(ErrorKind::MaxFilesWatch)]));
        handler(Err(vec![Error::new(ErrorKind::MaxFilesWatch)]));
    }
    cx.run_until_parked();
    assert_eq!(
        global_failures(&seen, g.path()),
        [apinata_app::workspace::WorkspaceEvent::WatchFailed {
            root: g.path().to_path_buf(),
            message
        }]
    );
}

// Once the file is back to what the editor loaded, the changed-on-disk banner goes away.
#[gpui_kit::test]
fn the_global_banner_clears_when_the_file_returns_to_base(cx: &mut TestAppContext) {
    let (g, win, editor) = plain_typing_under_a_watched_edit(cx);
    std::fs::write(g.path().join("g.yml"), HOST_A).unwrap();
    wait_for(cx, "the banner to clear", |cx| {
        !present(cx, win, "banner-changed")
    });
    assert_eq!(first_value(cx, &editor), "typed");
}

// The notify message ends its own sentence before the advice, in both arms.
#[test]
fn a_watch_failure_reads_as_two_sentences() {
    use apinata_app::shell::watch_failed_text;
    assert_eq!(
        watch_failed_text(None, "no inotify"),
        "Stopped watching global environments: no inotify. \
         Reopen Global environments to pick up changes."
    );
    assert_eq!(
        watch_failed_text(Some("col"), "no inotify"),
        "Stopped watching col: no inotify. Use Reload collection to pick up changes."
    );
    assert_eq!(
        watch_failed_text(Some("col"), "OS file watch limit reached."),
        "Stopped watching col: OS file watch limit reached. Use Reload collection to pick up changes."
    );
}

// Watching never makes a missing global directory; the first saved environment does, and
// from then on another program's file there is picked up.
#[gpui_kit::test]
fn the_global_directory_is_made_by_the_first_save_not_by_watching(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (_c, base, _s, ws) = start(cx, &[]);
    let dir = base.path().join("environments");
    let seen = events(cx, &ws);
    let set = dir.clone();
    ws.update(cx, |w, cx| w.set_global_dir(set, cx));
    ws.update(cx, |w, cx| {
        w.set_watch_factory(Some(apinata_app::watch::set::notify_factory()), cx)
    });
    cx.run_until_parked();
    assert!(!dir.exists(), "watching made the directory");
    assert!(global_failures(&seen, &dir).is_empty());
    edit(cx, &ws, GlobalOp::Create("g".into()));
    assert!(dir.exists());
    settle(cx);
    std::fs::write(dir.join("h.yml"), HOST_A.replace("name: g", "name: h")).unwrap();
    wait_for(cx, "the outside environment", |cx| {
        names(cx, &ws).contains(&"h".to_string())
    });
}
