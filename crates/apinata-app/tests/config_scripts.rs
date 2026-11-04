//! Folder and collection settings edit their scripts and save what core saves.

use crate::support;

use apinata_app::config_editor::{ConfigEditor, ConfigSection, ConfigTarget};
use apinata_app::workspace::Workspace;
use apinata_core::{Collection, ScriptPhase};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use std::path::Path;
use support::*;

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("Users")),
    ])
}

fn editor(
    cx: &mut TestAppContext,
    ws: &Entity<Workspace>,
    root: &Path,
    target: ConfigTarget,
) -> (AnyWindowHandle, Entity<ConfigEditor>) {
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
    frame(cx, win);
    (win, view)
}

fn click_section(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    ed: &Entity<ConfigEditor>,
    s: ConfigSection,
) {
    let ix = ed
        .read_with(cx, |e, _| e.sections().iter().position(|x| *x == s))
        .expect("offered");
    cx.update_window(win, |_, w, cx| w.click(("config-section", ix), cx))
        .unwrap();
    frame(cx, win);
}

#[gpui_kit::test]
fn folder_scripts_save_what_core_saves(cx: &mut TestAppContext) {
    use ConfigSection::*;
    let (col, twin) = (tree(), tree());
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor(cx, &ws, &root, ConfigTarget::Folder(ip("users")));
    assert_eq!(
        ed.read_with(cx, |e, _| e.sections()),
        [Headers, Auth, Vars, Script, Tests, Docs, Tags]
    );
    click_section(cx, win, &ed, Script);
    replace_code(
        cx,
        win,
        Some("script-section"),
        "script-pre-request-code",
        "let f = 1",
    );
    click_section(cx, win, &ed, Tests);
    replace_code(
        cx,
        win,
        Some("tests-section"),
        "script-tests-code",
        "let t = 2",
    );
    cx.run_until_parked();
    ed.update(cx, |e, cx| e.save(cx));
    cx.run_until_parked();
    let mut c = Collection::open(twin.path()).unwrap();
    let f = c.folder_file_mut(&ip("users")).unwrap();
    f.set_script(ScriptPhase::PreRequest, "let f = 1");
    f.set_script(ScriptPhase::Tests, "let t = 2");
    f.save().unwrap();
    let read =
        |d: &tempfile::TempDir| std::fs::read_to_string(d.path().join("users/folder.yml")).unwrap();
    assert_eq!(read(&col), read(&twin));
}

#[gpui_kit::test]
fn collection_scripts_load_into_the_editor(cx: &mut TestAppContext) {
    let root_yml = format!(
        "{ROOT}request:\n  scripts:\n    - type: after-response\n      code: \"bru.setVar('c', 1)\"\n"
    );
    let col = write_tree(&[("opencollection.yml", root_yml.as_str())]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor(cx, &ws, &root, ConfigTarget::Collection);
    click_section(cx, win, &ed, ConfigSection::Script);
    let post = ed
        .read_with(cx, |e, cx| {
            e.script().read(cx).code(ScriptPhase::PostResponse).cloned()
        })
        .unwrap();
    assert_eq!(
        post.read_with(cx, |c, _| c.value().to_string()),
        "bru.setVar('c', 1)"
    );
}
