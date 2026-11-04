//! New collection through a save prompt, and environment files imported into the open collection.

use crate::support;

use apinata_app::env_selector::{
    IMPORT_ENVIRONMENT, MANAGE_ENVIRONMENTS, NO_ENVIRONMENT, option_id,
};
use apinata_app::session::Session;
use apinata_app::shell::Shell;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext,
    WindowOptions,
};
use std::fs;
use std::path::PathBuf;
use support::*;

const DEV: &str = r#"{"name":"dev","values":[{"key":"host","value":"x.test","enabled":true},{"key":"token","value":"s3","type":"secret","enabled":true}],"_postman_variable_scope":"environment"}"#;
const STAGE: &str = r#"{"name":"stage","values":[{"key":"host","value":"y.test"}]}"#;

/// A shell over a fresh workspace, with `col` open when given.
fn shell(
    cx: &mut TestAppContext,
    col: Option<&tempfile::TempDir>,
) -> (tempfile::TempDir, AnyWindowHandle, Entity<Shell>) {
    support::init(cx);
    let files = tempfile::tempdir().unwrap();
    let session = session_at(cx, &files, Session::default());
    let ws = workspace(cx, &session);
    if let Some(col) = col {
        ws.update(cx, |w, cx| w.open(col.path().to_path_buf(), cx));
    }
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| Shell::new(ws, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    (files, win, view)
}

fn opened(cx: &mut TestAppContext, view: &Entity<Shell>) -> Vec<PathBuf> {
    view.read_with(cx, |v, cx| {
        v.workspace()
            .read(cx)
            .collections()
            .iter()
            .map(|c| c.root().to_path_buf())
            .collect()
    })
}

fn dialog(cx: &mut TestAppContext, win: AnyWindowHandle) -> bool {
    cx.update_window(win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap()
}

/// Chooses the environment dropdown's import entry, as a click on it does.
fn import_env(cx: &mut TestAppContext, win: AnyWindowHandle, view: &Entity<Shell>) {
    let sel = view.read_with(cx, |v, _| v.env_selector().clone());
    cx.update_window(win, |_, w, cx| {
        sel.update(cx, |s, cx| s.choose(IMPORT_ENVIRONMENT, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

fn choose_files(cx: &mut TestAppContext, files: Vec<PathBuf>) {
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(move |o| {
        assert!(o.files && o.multiple && !o.directories);
        Some(files)
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn new_collection_creates_and_opens_it(cx: &mut TestAppContext) {
    let (_files, win, view) = shell(cx, None);
    let parent = tempfile::tempdir().unwrap();
    let chosen = parent.path().join("Fresh");
    switcher_click(cx, win, "switcher-new-collection");
    assert!(cx.did_prompt_for_new_path());
    let answer = chosen.clone();
    cx.simulate_new_path_selection(move |_| Some(answer));
    cx.run_until_parked();
    assert_eq!(opened(cx, &view), std::slice::from_ref(&chosen));
    let root = fs::read_to_string(chosen.join("opencollection.yml")).unwrap();
    assert!(root.contains("name: Fresh"), "{root}");
}

#[gpui_kit::test]
fn new_collection_over_an_existing_directory_is_refused(cx: &mut TestAppContext) {
    let (_files, win, view) = shell(cx, None);
    let existing = tempfile::tempdir().unwrap();
    fs::write(existing.path().join("keep.txt"), "x").unwrap();
    let answer = existing.path().to_path_buf();
    switcher_click(cx, win, "switcher-new-collection");
    cx.simulate_new_path_selection(move |_| Some(answer));
    cx.run_until_parked();
    assert!(opened(cx, &view).is_empty());
    assert!(dialog(cx, win));
}

#[gpui_kit::test]
fn new_collection_into_an_empty_folder_opens_it(cx: &mut TestAppContext) {
    let (_files, win, view) = shell(cx, None);
    let parent = tempfile::tempdir().unwrap();
    let chosen = parent.path().join("Empty");
    fs::create_dir(&chosen).unwrap();
    let answer = chosen.clone();
    switcher_click(cx, win, "switcher-new-collection");
    cx.simulate_new_path_selection(move |_| Some(answer));
    cx.run_until_parked();
    assert_eq!(opened(cx, &view), std::slice::from_ref(&chosen));
    assert!(chosen.join("opencollection.yml").is_file());
    assert!(!dialog(cx, win));
}

#[gpui_kit::test]
fn the_key_starts_a_new_collection(cx: &mut TestAppContext) {
    let (_files, win, view) = shell(cx, None);
    cx.update(apinata_app::keymap::init);
    {
        let mut v = VisualTestContext::from_window(win, cx);
        v.update(|w, cx| view.read(cx).focus_handle(cx).focus(w, cx));
        v.simulate_keystrokes(&key("shift-n"));
        v.run_until_parked();
    }
    assert!(cx.did_prompt_for_new_path());
}

#[gpui_kit::test]
fn chosen_environment_files_are_imported(cx: &mut TestAppContext) {
    let col = collection();
    let (files, win, view) = shell(cx, Some(&col));
    let dev = files.path().join("dev.json");
    let stage = files.path().join("stage.json");
    fs::write(&dev, DEV).unwrap();
    fs::write(&stage, STAGE).unwrap();
    import_env(cx, win, &view);
    choose_files(cx, vec![dev, stage]);
    for name in ["dev", "stage"] {
        assert!(
            col.path()
                .join(format!("environments/{name}.yml"))
                .is_file(),
            "{name}"
        );
    }
    let sel = view.read_with(cx, |v, _| v.env_selector().clone());
    let report = sel
        .read_with(cx, |s, _| s.last_import().cloned())
        .expect("imported");
    assert_eq!(report.environments, ["dev", "stage"]);
    assert_eq!(sel.read_with(cx, |s, cx| s.items(cx)).len(), 3);
    assert!(!dialog(cx, win), "no notes, no report");
}

#[gpui_kit::test]
fn a_file_that_is_not_an_environment_imports_nothing(cx: &mut TestAppContext) {
    let col = collection();
    let (files, win, view) = shell(cx, Some(&col));
    let bad = files.path().join("bad.json");
    let stage = files.path().join("stage.json");
    fs::write(&bad, r#"{"info":{}}"#).unwrap();
    fs::write(&stage, STAGE).unwrap();
    import_env(cx, win, &view);
    choose_files(cx, vec![bad, stage]);
    assert!(dialog(cx, win));
    assert!(!col.path().join("environments/stage.yml").exists());
    let sel = view.read_with(cx, |v, _| v.env_selector().clone());
    assert!(sel.read_with(cx, |s, _| s.last_import().is_none()));
}

#[gpui_kit::test]
fn a_write_failing_after_another_keeps_what_was_written(cx: &mut TestAppContext) {
    let col = collection();
    let (files, win, view) = shell(cx, Some(&col));
    let stage = files.path().join("stage.json");
    let dots = files.path().join("dots.json");
    fs::write(&stage, STAGE).unwrap();
    fs::write(&dots, r#"{"name":"...","values":[]}"#).unwrap();
    import_env(cx, win, &view);
    choose_files(cx, vec![stage, dots]);
    assert!(col.path().join("environments/stage.yml").is_file());
    assert!(dialog(cx, win));
    let sel = view.read_with(cx, |v, _| v.env_selector().clone());
    let report = sel
        .read_with(cx, |s, _| s.last_import().cloned())
        .expect("stage was written");
    assert_eq!(report.environments, ["stage"]);
}

#[gpui_kit::test]
fn a_failed_environment_write_reads_like_other_import_failures(cx: &mut TestAppContext) {
    support::init(cx);
    let col = collection();
    let model = open(cx, &col);
    // A file where the environments directory belongs makes the write fail.
    let blocker = col.path().join("environments");
    fs::write(&blocker, "").unwrap();
    let env = apinata_core::import::parse_environments(None, STAGE.as_bytes())
        .unwrap()
        .remove(0);
    let (reply, replies) = async_channel::bounded(1);
    send(
        cx,
        &model,
        apinata_app::command::Command::ImportEnvironment { env, reply },
    );
    cx.run_until_parked();
    let message = replies
        .try_recv()
        .expect("replied")
        .expect_err("the write fails");
    assert!(message.starts_with(char::is_uppercase), "{message}");
    let shown = blocker.display().to_string();
    assert_eq!(message.matches(&shown).count(), 1, "{message}");
}

#[gpui_kit::test]
fn import_environment_without_a_collection_does_nothing(cx: &mut TestAppContext) {
    let (_files, win, view) = shell(cx, None);
    import_env(cx, win, &view);
    assert!(!cx.did_prompt_for_paths());
}

#[gpui_kit::test]
fn the_environment_dropdown_ends_with_import(cx: &mut TestAppContext) {
    let col = collection();
    let (_files, _win, view) = shell(cx, Some(&col));
    let sel = view.read_with(cx, |v, _| v.env_selector().clone());
    let listed = sel.read_with(cx, |s, _| s.listed().to_vec());
    let items = sel.read_with(cx, |s, cx| s.items(cx));
    assert_eq!(listed.last().map(|s| s.as_ref()), Some(IMPORT_ENVIRONMENT));
    assert_eq!(listed[listed.len() - 2].as_ref(), MANAGE_ENVIRONMENTS);
    assert_eq!(&listed[..listed.len() - 2], &items[..]);
    assert!(!IMPORT_ENVIRONMENT.to_lowercase().contains("bruno"));
}

#[gpui_kit::test]
fn without_a_collection_the_dropdown_offers_no_import(cx: &mut TestAppContext) {
    let (_files, _win, view) = shell(cx, None);
    let sel = view.read_with(cx, |v, _| v.env_selector().clone());
    let listed = sel.read_with(cx, |s, _| s.listed().to_vec());
    assert!(
        listed.iter().all(|i| i.as_ref() != IMPORT_ENVIRONMENT),
        "{listed:?}"
    );
    assert!(
        listed.iter().all(|i| i.as_ref() != MANAGE_ENVIRONMENTS),
        "{listed:?}"
    );
}

#[gpui_kit::test]
fn choosing_import_keeps_the_selected_environment(cx: &mut TestAppContext) {
    let col = collection();
    let (files, win, view) = shell(cx, Some(&col));
    let stage = files.path().join("stage.json");
    fs::write(&stage, STAGE).unwrap();
    import_env(cx, win, &view);
    choose_files(cx, vec![stage]);
    assert!(col.path().join("environments/stage.yml").is_file());
    let sel = view.read_with(cx, |v, _| v.env_selector().clone());
    assert_eq!(
        sel.read_with(cx, |s, _| s.shown()),
        Some(gpui_kit::SharedString::from(NO_ENVIRONMENT)),
        "the import entry is never shown as selected"
    );
    let selected = view.read_with(cx, |v, cx| {
        v.workspace().read(cx).selected_environment(col.path(), cx)
    });
    assert_eq!(selected, None);
}

#[gpui_kit::test]
fn picking_import_in_the_select_keeps_the_previous_environment(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: host\n    value: x\n",
        ),
    ]);
    let (_files, win, view) = shell(cx, Some(&col));
    let ws = view.read_with(cx, |v, _| v.workspace().clone());
    ws.update(cx, |w, cx| {
        w.select_environment(col.path(), Some("dev".into()), cx)
    });
    frame(cx, win);
    let sel = view.read_with(cx, |v, _| v.env_selector().clone());
    assert_eq!(sel.read_with(cx, |s, _| s.shown()).as_deref(), Some("dev"));
    let select = sel.read_with(cx, |s, _| s.select().clone());
    // What a click on the entry does: the kit moves its cursor there, then confirms it.
    cx.update_window(win, |_, w, cx| {
        select.update(cx, |s, cx| {
            s.set_selected_value(&gpui_kit::SharedString::from(IMPORT_ENVIRONMENT), w, cx)
        })
    })
    .unwrap();
    cx.update_window(win, |_, w, cx| w.click("env-select", cx))
        .unwrap();
    frame(cx, win);
    cx.simulate_keystrokes(win, "enter");
    cx.run_until_parked();
    assert!(cx.did_prompt_for_paths(), "the import prompt opened");
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    frame(cx, win);
    let listed = sel.read_with(cx, |s, _| s.listed().to_vec());
    let dev = listed.iter().position(|i| i.as_ref() == "dev").unwrap();
    let (value, index) = select.read_with(cx, |s, cx| {
        (s.selected_value().cloned(), s.selected_index(cx))
    });
    assert_eq!(
        value.as_deref(),
        Some("dev"),
        "the select's value is dev again"
    );
    assert_eq!(
        index.map(|ix| ix.row),
        Some(dev),
        "and so is the row it shows"
    );
    assert_eq!(sel.read_with(cx, |s, _| s.shown()).as_deref(), Some("dev"));
    let chosen = ws.read_with(cx, |w, cx| w.selected_environment(col.path(), cx));
    assert_eq!(chosen.as_deref(), Some("dev"));
}

#[test]
fn dropdown_row_ids_never_collide() {
    assert_eq!(option_id(NO_ENVIRONMENT), "env-none");
    assert_eq!(option_id(IMPORT_ENVIRONMENT), "env-import");
    assert_eq!(option_id(MANAGE_ENVIRONMENTS), "env-manage");
    assert_eq!(option_id("none"), "env-option-none");
    assert_eq!(option_id("dev"), "env-option-dev");
}

#[gpui_kit::test]
fn dropdown_rows_are_observed_and_a_click_chooses_one(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", "name: dev\n"),
        ("environments/prod.yml", "name: prod\n"),
    ]);
    let (_files, win, view) = shell(cx, Some(&col));
    assert!(
        !present(cx, win, "env-option-dev"),
        "a closed dropdown lists nothing"
    );
    cx.update_window(win, |_, w, cx| w.click("env-select", cx))
        .unwrap();
    frame(cx, win);
    for id in [
        "env-none",
        "env-option-dev",
        "env-option-prod",
        "env-import",
    ] {
        assert!(present(cx, win, id), "{id} is in the tree");
    }
    cx.update_window(win, |_, w, cx| w.click("env-option-prod", cx))
        .unwrap();
    frame(cx, win);
    let sel = view.read_with(cx, |v, _| v.env_selector().clone());
    assert_eq!(sel.read_with(cx, |s, _| s.shown()).as_deref(), Some("prod"));
    let ws = view.read_with(cx, |v, _| v.workspace().clone());
    let chosen = ws.read_with(cx, |w, cx| {
        w.current_root()
            .and_then(|r| w.selected_environment(&r, cx))
    });
    assert_eq!(chosen.as_deref(), Some("prod"));
}

// The first-launch screen offers New collection beside Open and Import; it asks for a new
// directory and opens the collection made there.
#[gpui_kit::test]
fn the_empty_window_offers_new_collection(cx: &mut TestAppContext) {
    let (_files, win, view) = shell(cx, None);
    let parent = tempfile::tempdir().unwrap();
    let chosen = parent.path().join("Fresh");
    let offered = cx
        .update_window(win, |_, w, _| {
            let welcome = w.within("no-collection");
            (1..4usize)
                .filter(|ix| welcome.try_find(("empty-action", *ix)).is_some())
                .count()
        })
        .unwrap();
    assert_eq!(offered, 3, "Open, New and Import collection");
    cx.update_window(win, |_, w, cx| {
        w.within("no-collection")
            .click(("empty-action", 2usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        cx.did_prompt_for_new_path(),
        "the second action is New collection"
    );
    let answer = chosen.clone();
    cx.simulate_new_path_selection(move |_| Some(answer));
    cx.run_until_parked();
    assert_eq!(opened(cx, &view), std::slice::from_ref(&chosen));
}

// The welcome screen's buttons and the switcher's actions open a dialog, so they read as the
// commands they duplicate, ellipsis included.
#[gpui_kit::test]
fn the_welcome_and_switcher_collection_actions_end_in_an_ellipsis(cx: &mut TestAppContext) {
    let (_files, win, _view) = shell(cx, None);
    let welcome = cx
        .update_window(win, |_, w, _| {
            let scope = w.within("no-collection-actions");
            (1..4usize)
                .map(|ix| scope.find(("empty-action", ix)).label().map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap();
    let expected = ["Open collection…", "New collection…", "Import collection…"];
    assert_eq!(welcome, expected.map(|l| Some(l.to_owned())));
    cx.update_window(win, |_, w, cx| {
        w.within("title-bar").click("collection-switcher", cx)
    })
    .unwrap();
    frame(cx, win);
    let switcher = cx
        .update_window(win, |_, w, _| {
            [
                "switcher-open-collection",
                "switcher-import-collection",
                "switcher-new-collection",
            ]
            .map(|id| w.find(id).label().map(str::to_owned))
        })
        .unwrap();
    let expected = ["Open collection…", "Import collection…", "New collection…"];
    assert_eq!(switcher, expected.map(|l| Some(l.to_owned())));
}
