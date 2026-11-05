//! Opening a legacy collection: the conversion dialog, both targets and the failure alert.

use crate::support;

use apinata_app::legacy_convert::{
    conversion_failure, failure_text, in_place_detail, offer_detail,
};
use apinata_app::session::Session;
use apinata_app::shell::Shell;
use apinata_core::legacy::{ConvertError, plan_conversion};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, WindowOptions};
use std::fs;
use std::path::PathBuf;
use support::*;

const REQUEST: &str =
    "meta {\n  name: a\n  type: http\n  seq: 1\n}\n\nget {\n  url: https://example.com\n}\n";
const MALFORMED: &str = "meta {\n  name: a\n";

fn legacy() -> tempfile::TempDir {
    write_tree(&[
        (
            "bruno.json",
            r#"{"version": "1", "name": "Legacy", "type": "collection"}"#,
        ),
        ("a.bru", REQUEST),
        (".env", "T=x\n"),
    ])
}

fn shell(cx: &mut TestAppContext) -> (tempfile::TempDir, AnyWindowHandle, Entity<Shell>) {
    support::init(cx);
    let files = tempfile::tempdir().unwrap();
    let session = session_at(cx, &files, Session::default());
    let ws = workspace(cx, &session);
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

fn open_root(cx: &mut TestAppContext, view: &Entity<Shell>, root: PathBuf) {
    let ws = view.read_with(cx, |v, _| v.workspace().clone());
    ws.update(cx, |w, cx| w.open(root, cx));
    cx.run_until_parked();
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

fn alert(cx: &mut TestAppContext, win: AnyWindowHandle) -> bool {
    cx.update_window(win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap()
}

#[test]
fn the_offer_names_the_folder_it_is_about() {
    let detail = offer_detail(std::path::Path::new("/work/shop api"));
    assert_eq!(
        detail,
        "/work/shop api uses the legacy .bru format. Apinata stores collections as YAML files."
    );
}

#[gpui_kit::test]
fn opening_a_legacy_root_offers_conversion_and_cancel_changes_nothing(cx: &mut TestAppContext) {
    let (_files, _win, view) = shell(cx);
    let src = legacy();
    open_root(cx, &view, src.path().to_path_buf());
    let win = confirm_window(cx).expect("the conversion dialog");
    cx.update_window(win, |_, w, _| {
        for id in ["legacy-new-folder", "legacy-in-place", "legacy-cancel"] {
            assert!(w.try_find(id).is_some(), "{id} missing");
        }
    })
    .unwrap();
    answer_confirm(cx, "legacy-cancel");
    assert!(opened(cx, &view).is_empty());
    assert!(!cx.did_prompt_for_paths());
    assert!(src.path().join("a.bru").is_file());
}

#[gpui_kit::test]
fn new_folder_asks_for_a_parent_then_opens_the_copy(cx: &mut TestAppContext) {
    let (_files, _win, view) = shell(cx);
    let src = legacy();
    let parent = tempfile::tempdir().unwrap();
    open_root(cx, &view, src.path().to_path_buf());
    answer_confirm(cx, "legacy-new-folder");
    assert!(cx.did_prompt_for_paths());
    let chosen = parent.path().to_path_buf();
    cx.simulate_path_prompt_response(move |o| {
        assert!(o.directories && !o.files && !o.multiple);
        Some(vec![chosen])
    });
    cx.run_until_parked();
    let new_root = parent.path().join("Legacy");
    assert_eq!(opened(cx, &view), std::slice::from_ref(&new_root));
    assert!(new_root.join("a.yml").is_file() && src.path().join("a.bru").is_file());
}

#[gpui_kit::test]
fn in_place_asks_again_then_converts_and_opens_the_folder(cx: &mut TestAppContext) {
    let (_files, _win, view) = shell(cx);
    let src = legacy();
    open_root(cx, &view, src.path().to_path_buf());
    answer_confirm(cx, "legacy-in-place");
    assert!(confirm_open(cx), "the second confirmation");
    answer_confirm(cx, "legacy-convert");
    assert_eq!(opened(cx, &view), [src.path().to_path_buf()]);
    assert!(src.path().join("opencollection.yml").is_file() && !src.path().join("a.bru").exists());
}

#[gpui_kit::test]
fn declining_the_second_confirmation_changes_nothing(cx: &mut TestAppContext) {
    let (_files, _win, view) = shell(cx);
    let src = legacy();
    open_root(cx, &view, src.path().to_path_buf());
    answer_confirm(cx, "legacy-in-place");
    answer_confirm(cx, "legacy-cancel");
    assert!(opened(cx, &view).is_empty());
    assert!(src.path().join("a.bru").is_file() && !src.path().join("opencollection.yml").exists());
}

#[gpui_kit::test]
fn a_failed_conversion_shows_an_alert_and_opens_nothing(cx: &mut TestAppContext) {
    let (_files, win, view) = shell(cx);
    let src = legacy();
    fs::write(src.path().join("bad.bru"), MALFORMED).unwrap();
    open_root(cx, &view, src.path().to_path_buf());
    answer_confirm(cx, "legacy-in-place");
    assert!(
        !confirm_open(cx),
        "no second confirmation for a plan that failed"
    );
    assert!(alert(cx, win));
    assert!(opened(cx, &view).is_empty());
}

fn shows(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> bool {
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id).is_some()
    })
    .unwrap()
}

#[gpui_kit::test]
fn two_legacy_roots_are_offered_one_at_a_time(cx: &mut TestAppContext) {
    let (_files, win, view) = shell(cx);
    let (first, second) = (legacy(), legacy());
    open_root(cx, &view, first.path().to_path_buf());
    open_root(cx, &view, second.path().to_path_buf());
    answer_confirm(cx, "legacy-in-place");
    assert!(shows(cx, win, "legacy-convert"), "the second confirmation");
    assert!(
        !shows(cx, win, "legacy-new-folder"),
        "the next offer waits until this one is resolved"
    );
    answer_confirm(cx, "legacy-convert");
    assert!(shows(cx, win, "legacy-new-folder"), "then the next offer");
    answer_confirm(cx, "legacy-cancel");
    assert!(!confirm_open(cx));
    assert_eq!(opened(cx, &view).len(), 1);
    assert!(second.path().join("a.bru").is_file());
}

#[gpui_kit::test]
fn reopening_a_root_whose_offer_waits_queues_it_once(cx: &mut TestAppContext) {
    let (_files, win, view) = shell(cx);
    let src = legacy();
    open_root(cx, &view, src.path().to_path_buf());
    open_root(cx, &view, src.path().to_path_buf());
    answer_confirm(cx, "legacy-in-place");
    answer_confirm(cx, "legacy-convert");
    assert_eq!(opened(cx, &view), [src.path().to_path_buf()]);
    assert!(
        !confirm_open(cx),
        "a second offer for the converted root is queued"
    );
    assert!(!alert(cx, win), "a 'not legacy' alert");
}

#[gpui_kit::test]
fn another_spelling_of_a_waiting_root_queues_no_second_offer(cx: &mut TestAppContext) {
    let (_files, _win, view) = shell(cx);
    let src = legacy();
    let mut trailing = src.path().as_os_str().to_owned();
    trailing.push("/");
    open_root(cx, &view, src.path().to_path_buf());
    open_root(cx, &view, PathBuf::from(trailing));
    open_root(cx, &view, src.path().join("."));
    answer_confirm(cx, "legacy-cancel");
    assert!(!confirm_open(cx), "a second offer for the same directory");
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_symlink_to_a_waiting_root_queues_no_second_offer(cx: &mut TestAppContext) {
    let (_files, _win, view) = shell(cx);
    let src = legacy();
    let links = tempfile::tempdir().unwrap();
    let link = links.path().join("link");
    std::os::unix::fs::symlink(src.path(), &link).unwrap();
    open_root(cx, &view, src.path().to_path_buf());
    open_root(cx, &view, link);
    answer_confirm(cx, "legacy-cancel");
    assert!(!confirm_open(cx), "a second offer through the symlink");
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_root_opened_through_a_symlink_is_converted_and_reopened_under_that_path(
    cx: &mut TestAppContext,
) {
    let (_files, _win, view) = shell(cx);
    let src = legacy();
    let links = tempfile::tempdir().unwrap();
    let link = links.path().join("link");
    std::os::unix::fs::symlink(src.path(), &link).unwrap();
    open_root(cx, &view, link.clone());
    answer_confirm(cx, "legacy-in-place");
    answer_confirm(cx, "legacy-convert");
    assert_eq!(opened(cx, &view), [link]);
}

#[gpui_kit::test]
fn the_next_offer_waits_for_a_failure_alert_to_close(cx: &mut TestAppContext) {
    let (_files, win, view) = shell(cx);
    let (bad, good) = (legacy(), legacy());
    fs::write(bad.path().join("bad.bru"), MALFORMED).unwrap();
    open_root(cx, &view, bad.path().to_path_buf());
    open_root(cx, &view, good.path().to_path_buf());
    answer_confirm(cx, "legacy-in-place");
    assert!(alert(cx, win));
    assert!(!confirm_open(cx), "no offer over the alert");
    cx.update_window(win, |_, w, cx| w.close_dialog(cx))
        .unwrap();
    assert!(shows(cx, win, "legacy-new-folder"), "the next offer");
    answer_confirm(cx, "legacy-cancel");
    assert!(!confirm_open(cx) && !alert(cx, win));
}

#[test]
fn the_second_confirmation_counts_the_files() {
    assert_eq!(
        in_place_detail(3, 2),
        "3 YAML files will be created. 2 .bru files and the legacy collection file will be deleted. Other files are left untouched."
    );
    assert_eq!(
        in_place_detail(1, 1),
        "1 YAML file will be created. 1 .bru file and the legacy collection file will be deleted. Other files are left untouched."
    );
    assert!(!in_place_detail(2, 2).to_lowercase().contains("bruno"));
}

#[test]
fn failure_text_lists_the_first_ten_errors() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("bruno.json"),
        r#"{"version": "1", "name": "x", "type": "collection"}"#,
    )
    .unwrap();
    for n in 0..12 {
        fs::write(dir.path().join(format!("m{n:02}.bru")), MALFORMED).unwrap();
    }
    let text = failure_text(&plan_conversion(dir.path()).unwrap_err());
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 12, "{text}");
    assert_eq!(lines[0], "12 of the legacy files could not be read");
    for (i, line) in lines[1..11].iter().enumerate() {
        assert!(line.starts_with(&format!("m{i:02}.bru:")), "{line}");
    }
    assert_eq!(lines[11], "and 2 more");
}

#[test]
fn a_failed_delete_says_the_yaml_is_complete_and_names_the_leftovers() {
    let src = legacy();
    let plan = plan_conversion(src.path()).unwrap();
    let io = |rel: &str| ConvertError::Io {
        path: src.path().join(rel),
        message: "denied".into(),
    };
    let text = conversion_failure(&plan, &io("a.bru"));
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].ends_with("a.bru: denied"), "{text}");
    assert!(
        text.contains("The YAML collection is complete, but some legacy files were not deleted."),
        "{text}"
    );
    assert!(
        text.contains("Remove these leftover legacy files by hand"),
        "{text}"
    );
    assert_eq!(lines[lines.len() - 2..], ["a.bru", "bruno.json"], "{text}");
    assert_eq!(
        conversion_failure(&plan, &io("a.yml")),
        failure_text(&io("a.yml")),
        "a failed write rolls back"
    );
}
