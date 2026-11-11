//! The environment list's name field, in a collection's editor and the global one: New refuses
//! a name as Rename does, the reason stays inside the list column, and the Rename label reads
//! on one line.

use crate::support;

use apinata_app::env_editor::{ENV_MENU_RENAME, EnvEditor};
use apinata_app::global_env_editor::{GlobalEnvEditor, open_global_env_editor};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, WindowOptions, px};
use support::*;

const NAME: &str = "env-name-input";
const LONG: &str = "production-eu-west";

fn files() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", "name: dev\n"),
        (
            &format!("environments/{LONG}.yml"),
            &format!("name: {LONG}\n"),
        ),
    ])
}

struct Opened {
    col: tempfile::TempDir,
    win: AnyWindowHandle,
    ed: Entity<EnvEditor>,
    _state: tempfile::TempDir,
}

// An editor over dev and the long-named environment with dev selected.
fn opened(cx: &mut TestAppContext) -> Opened {
    let col = files();
    let (state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.select("dev", w, cx))
    })
    .unwrap();
    frame(cx, win);
    Opened {
        col,
        win,
        ed,
        _state: state,
    }
}

// Opens New, types `name` and presses Enter.
fn create(cx: &mut TestAppContext, win: AnyWindowHandle, name: &str) {
    cx.update_window(win, |_, w, cx| w.click("env-create", cx))
        .unwrap();
    cx.run_until_parked();
    cx.simulate_input(win, name);
    cx.simulate_keystrokes(win, "enter");
    cx.run_until_parked();
}

fn env_files(col: &tempfile::TempDir) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(col.path().join("environments"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

// The field as one frame draws it: its value and the reason under it, if any.
fn drawn(cx: &mut TestAppContext, win: AnyWindowHandle) -> (Option<String>, Option<String>) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let value = w
            .try_find(NAME)
            .map(|e| e.value().unwrap_or_default().to_owned());
        let error = w
            .try_find("env-name-error")
            .and_then(|e| e.label().map(str::to_owned));
        (value, error)
    })
    .unwrap()
}

// A taken name is refused under the field, as Rename refuses it, and no copy is made.
#[gpui_kit::test]
fn new_refuses_a_taken_name_under_the_field(cx: &mut TestAppContext) {
    let Opened { col, win, ed, .. } = opened(cx);
    create(cx, win, "dev");
    assert_eq!(
        drawn(cx, win),
        (
            Some("dev".into()),
            Some("An environment named \"dev\" already exists".into())
        )
    );
    assert!(present(cx, win, "env-name-invalid-icon"), "marked invalid");
    assert!(!present(cx, win, "env-error"), "no line under the pane");
    assert_eq!(env_files(&col), ["dev.yml", &format!("{LONG}.yml")]);
    assert_eq!(
        ed.read_with(cx, |e, _| e.selected().map(str::to_owned))
            .as_deref(),
        Some("dev")
    );
}

// A name core would change to fit a file name is refused with the typed text, not changed.
#[gpui_kit::test]
fn new_refuses_a_name_it_would_change(cx: &mut TestAppContext) {
    let Opened { col, win, .. } = opened(cx);
    create(cx, win, "a/b");
    assert_eq!(
        drawn(cx, win),
        (
            Some("a/b".into()),
            Some("\"a/b\" is not a valid environment name".into())
        )
    );
    assert_eq!(env_files(&col), ["dev.yml", &format!("{LONG}.yml")]);
}

// A free, valid name still creates the environment and closes the field.
#[gpui_kit::test]
fn new_creates_a_free_name(cx: &mut TestAppContext) {
    let Opened { col, win, ed, .. } = opened(cx);
    create(cx, win, "qa");
    assert_eq!(drawn(cx, win), (None, None));
    assert!(env_files(&col).contains(&"qa.yml".to_owned()));
    assert_eq!(
        ed.read_with(cx, |e, _| e.selected().map(str::to_owned))
            .as_deref(),
        Some("qa")
    );
}

// The reason wraps inside the list column, clear of the divider and the variables table.
#[gpui_kit::test]
fn the_reason_stays_inside_the_list(cx: &mut TestAppContext) {
    for opener in ["rename", "new"] {
        let Opened { win, .. } = opened(cx);
        match opener {
            "new" => create(cx, win, LONG),
            _ => {
                env_menu(cx, win, ENV_MENU_RENAME);
                cx.simulate_keystrokes(win, &secondary("a"));
                cx.simulate_input(win, LONG);
                cx.simulate_keystrokes(win, "enter");
                cx.run_until_parked();
            }
        }
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let (list, divider, field, error) = (
                w.find("env-list").bounds(),
                w.find("env-divider").bounds(),
                w.find(NAME).bounds(),
                w.find("env-name-error").bounds(),
            );
            assert!(
                error.left() >= list.left() - px(1.) && error.right() <= list.right() + px(1.),
                "{opener}: {error:?} inside the list {list:?}"
            );
            assert!(
                error.right() <= divider.left(),
                "{opener}: clear of the divider"
            );
            assert!(
                error.top() >= field.bottom() - px(1.),
                "{opener}: under the field"
            );
        })
        .unwrap();
    }
}

// The Rename label names the environment on one line, the height of New's label.
#[gpui_kit::test]
fn the_rename_label_reads_on_one_line(cx: &mut TestAppContext) {
    let Opened { win, ed, .. } = opened(cx);
    cx.update_window(win, |_, w, cx| w.click("env-create", cx))
        .unwrap();
    cx.run_until_parked();
    let one_line = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find("env-name-label").bounds().size.height
        })
        .unwrap();
    cx.simulate_keystrokes(win, "escape");
    cx.update_window(win, |_, w, cx| ed.update(cx, |e, cx| e.select(LONG, w, cx)))
        .unwrap();
    frame(cx, win);
    env_menu(cx, win, ENV_MENU_RENAME);
    assert_eq!(
        ed.read_with(cx, |e, _| e.naming_label()).as_deref(),
        Some(format!("Rename \"{LONG}\"").as_str())
    );
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let label = w.find("env-name-label");
        assert_eq!(label.label(), Some(format!("Rename \"{LONG}\"").as_str()));
        let height = label.bounds().size.height;
        assert!(
            height <= one_line + px(1.),
            "{height:?} is one line of {one_line:?}"
        );
    })
    .unwrap();
}

const GLOBAL_NAME: &str = "global-env-name-input";

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

struct Globals {
    dir: tempfile::TempDir,
    win: AnyWindowHandle,
    ed: Entity<GlobalEnvEditor>,
    _keep: (tempfile::TempDir, tempfile::TempDir),
}

// The global editor over dev and the long-named environment, holding one variable each, with
// dev selected.
fn globals(cx: &mut TestAppContext) -> Globals {
    support::init(cx);
    let body = |n: &str| format!("name: {n}\nvariables:\n  - name: host\n    value: a\n");
    let (dev, long) = (body("dev"), body(LONG));
    let long_file = format!("{LONG}.yml");
    let dir = write_tree(&[("dev.yml", &dev), (&long_file, &long)]);
    let col = collection();
    let (state, ws, _root) = open_dir(cx, &col);
    let path = dir.path().to_path_buf();
    ws.update(cx, |w, cx| w.set_global_dir(path, cx));
    cx.run_until_parked();
    let (win, _) = cx
        .update(|cx| gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Blank)))
        .unwrap();
    let ed = cx
        .update_window(win, |_, w, cx| open_global_env_editor(ws.clone(), w, cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.select("dev", w, cx))
    })
    .unwrap();
    frame(cx, win);
    Globals {
        dir,
        win,
        ed,
        _keep: (col, state),
    }
}

// Opens the global New field, types `name` and presses Enter.
fn create_global(cx: &mut TestAppContext, win: AnyWindowHandle, name: &str) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click("global-env-create", cx)
    })
    .unwrap();
    cx.run_until_parked();
    cx.simulate_input(win, name);
    cx.simulate_keystrokes(win, "enter");
    cx.run_until_parked();
}

fn global_files(g: &Globals) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(g.dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

// The global field as one frame draws it: its value and the reason under it, if any.
fn drawn_global(cx: &mut TestAppContext, win: AnyWindowHandle) -> (Option<String>, Option<String>) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let value = w
            .try_find(GLOBAL_NAME)
            .map(|e| e.value().unwrap_or_default().to_owned());
        let error = w
            .try_find("global-env-name-error")
            .and_then(|e| e.label().map(str::to_owned));
        (value, error)
    })
    .unwrap()
}

fn global_selected(cx: &mut TestAppContext, g: &Globals) -> Option<String> {
    g.ed.read_with(cx, |e, _| e.selected().map(str::to_owned))
}

// A taken global name is refused under the field, and no copy is made.
#[gpui_kit::test]
fn global_new_refuses_a_taken_name_under_the_field(cx: &mut TestAppContext) {
    let g = globals(cx);
    create_global(cx, g.win, "dev");
    assert_eq!(
        drawn_global(cx, g.win),
        (
            Some("dev".into()),
            Some("An environment named \"dev\" already exists".into())
        )
    );
    assert!(
        present(cx, g.win, "global-env-name-invalid-icon"),
        "marked invalid"
    );
    assert!(
        !present(cx, g.win, "global-env-error"),
        "no line above the list"
    );
    assert_eq!(global_files(&g), ["dev.yml", &format!("{LONG}.yml")]);
    assert_eq!(global_selected(cx, &g).as_deref(), Some("dev"));
}

// A global name core would change to fit a file name is refused with the typed text.
#[gpui_kit::test]
fn global_new_refuses_a_name_it_would_change(cx: &mut TestAppContext) {
    let g = globals(cx);
    create_global(cx, g.win, "a/b");
    assert_eq!(
        drawn_global(cx, g.win),
        (
            Some("a/b".into()),
            Some("\"a/b\" is not a valid environment name".into())
        )
    );
    assert_eq!(global_files(&g), ["dev.yml", &format!("{LONG}.yml")]);
}

// A free, valid global name still creates the environment and closes the field.
#[gpui_kit::test]
fn global_new_creates_a_free_name(cx: &mut TestAppContext) {
    let g = globals(cx);
    create_global(cx, g.win, "qa");
    assert_eq!(drawn_global(cx, g.win), (None, None));
    assert!(global_files(&g).contains(&"qa.yml".to_owned()));
    assert_eq!(global_selected(cx, &g).as_deref(), Some("qa"));
}

// The global reason wraps inside the list column, under the field.
#[gpui_kit::test]
fn global_the_reason_stays_inside_the_list(cx: &mut TestAppContext) {
    let g = globals(cx);
    create_global(cx, g.win, LONG);
    cx.update_window(g.win, |_, w, cx| {
        w.render_frame(cx);
        let (list, field, error) = (
            w.find("global-env-list").bounds(),
            w.find(GLOBAL_NAME).bounds(),
            w.find("global-env-name-error").bounds(),
        );
        assert!(
            error.left() >= list.left() - px(1.) && error.right() <= list.right() + px(1.),
            "{error:?} inside the list {list:?}"
        );
        assert!(
            field.left() >= list.left() - px(1.) && field.right() <= list.right() + px(1.),
            "the field {field:?} is in the list {list:?}"
        );
        assert!(error.top() >= field.bottom() - px(1.), "under the field");
    })
    .unwrap();
}

// The global Rename label names the environment on one line, the height of New's label.
#[gpui_kit::test]
fn global_the_rename_label_reads_on_one_line(cx: &mut TestAppContext) {
    let g = globals(cx);
    cx.update_window(g.win, |_, w, cx| {
        w.render_frame(cx);
        w.click("global-env-create", cx)
    })
    .unwrap();
    cx.run_until_parked();
    let one_line = cx
        .update_window(g.win, |_, w, cx| {
            w.render_frame(cx);
            w.find("global-env-name-label").bounds().size.height
        })
        .unwrap();
    cx.simulate_keystrokes(g.win, "escape");
    let ed = g.ed.clone();
    cx.update_window(g.win, |_, w, cx| {
        ed.update(cx, |e, cx| e.select(LONG, w, cx))
    })
    .unwrap();
    cx.update_window(g.win, |_, w, cx| {
        w.render_frame(cx);
        w.click("global-env-rename", cx)
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(g.win, |_, w, cx| {
        w.render_frame(cx);
        let label = w.find("global-env-name-label");
        assert_eq!(label.label(), Some(format!("Rename \"{LONG}\"").as_str()));
        let height = label.bounds().size.height;
        assert!(
            height <= one_line + px(1.),
            "{height:?} is one line of {one_line:?}"
        );
    })
    .unwrap();
}

// The global Save is plain with nothing to save and primary once there is an edit.
#[gpui_kit::test]
fn global_save_is_primary_only_with_edits(cx: &mut TestAppContext) {
    use gpui_kit::component::ActiveTheme as _;
    let g = globals(cx);
    let primary: gpui_kit::Hsla = cx.update(|cx| cx.theme().tokens.button_primary.into());
    assert!(
        !save_filled(cx, g.win, primary),
        "nothing to save: Save is plain"
    );
    let ed = g.ed.clone();
    cx.update_window(g.win, |_, w, cx| {
        let table = ed.read(cx).table().clone();
        let mut rows = table.read(cx).rows(cx);
        rows[0].value = "b".into();
        table.update(cx, |t, cx| t.load(rows, w, cx));
    })
    .unwrap();
    frame(cx, g.win);
    assert!(
        save_filled(cx, g.win, primary),
        "an edit to save: Save is primary"
    );
}

// Whether a quad of `primary` is painted under the global Save's centre in the next frame.
fn save_filled(cx: &mut TestAppContext, win: AnyWindowHandle, primary: gpui_kit::Hsla) -> bool {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let save = w.find("global-env-save").bounds();
        let scale = w.scale_factor();
        let (cx_, cy) = (f32::from(save.center().x), f32::from(save.center().y));
        w.painted_quads().into_iter().any(|q| {
            let (x, y) = (q.bounds.origin.x.0 / scale, q.bounds.origin.y.0 / scale);
            let r = x + q.bounds.size.width.0 / scale;
            let b = y + q.bounds.size.height.0 / scale;
            q.background.as_solid() == Some(primary) && x <= cx_ && r >= cx_ && y <= cy && b >= cy
        })
    })
    .unwrap()
}
