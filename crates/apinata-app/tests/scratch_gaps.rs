//! Scratch pad loose ends: quitting with a draft, and a relaunch with no collection open.

use crate::support;

use apinata_app::command::Edit;
use apinata_app::draft_file::MAX_DRAFT_BYTES;
use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext};
use support::*;

fn scratch_draft(cx: &mut TestAppContext, s: &Started) -> String {
    s.ws.update(cx, |w, cx| w.new_scratch_request(cx));
    cx.run_until_parked();
    let (root, item) = active_draft(cx, &s.ws);
    assert!(s.ws.read_with(cx, |w, _| w.is_scratch(&root)));
    item.as_str().to_owned()
}

#[gpui_kit::test]
fn quitting_with_an_unkept_scratch_draft_prompts(cx: &mut TestAppContext) {
    let s = started_blank(cx, blank_data_dir());
    let item = scratch_draft(cx, &s);
    let (root, _) = active_draft(cx, &s.ws);
    let edit = apinata_app::command::Command::Edit {
        target: apinata_app::command::Target::Request(ip(&item)),
        edit: Edit::Docs("x".repeat(MAX_DRAFT_BYTES)),
    };
    s.ws.update(cx, |w, cx| w.send(&root, edit, cx));
    cx.run_until_parked();
    let closed = VisualTestContext::from_window(s.win, cx).simulate_close();
    assert!(!closed, "the close is held for the prompt");
    assert!(confirm_open(cx), "a prompt asks about the scratch draft");
}

#[gpui_kit::test]
fn a_relaunch_with_no_collection_restores_the_scratch_draft(cx: &mut TestAppContext) {
    let first = started_blank(cx, blank_data_dir());
    let item = scratch_draft(cx, &first);
    let (root, _) = active_draft(cx, &first.ws);
    let edit = apinata_app::command::Command::Edit {
        target: apinata_app::command::Target::Request(ip(&item)),
        edit: Edit::Url("http://127.0.0.1:9/again".into()),
    };
    first.ws.update(cx, |w, cx| w.send(&root, edit, cx));
    cx.run_until_parked();
    assert!(VisualTestContext::from_window(first.win, cx).simulate_close());
    // The first run still holds the drafts lock in this process, so the second starts over a
    // copy of what the first left, as drafts_restore does.
    let kept = apinata_app::draft_file::load_dir(&first.files.path().join("drafts")).drafts;
    assert_eq!(kept.len(), 1, "closing keeps the scratch draft");
    let files = blank_data_dir();
    let drafts = apinata_app::persist::Paths::in_dir(files.path()).drafts;
    for d in &kept {
        // The file names the scratch directory of the first run; the relaunch has its own.
        let mut d = d.clone();
        d.root = apinata_app::persist::Paths::in_dir(files.path()).scratch;
        let name = apinata_app::draft_file::file_name(&d.item_key().unwrap());
        apinata_app::persist::save_json(&drafts.join(name), &d).unwrap();
    }
    let second = started_blank(cx, files);
    cx.run_until_parked();
    let (root, item) = active_draft(cx, &second.ws);
    assert!(second.ws.read_with(cx, |w, _| w.is_scratch(&root)));
    let view = second
        .ws
        .read_with(cx, |w, cx| w.request(&root, &item, cx))
        .unwrap();
    assert_eq!(view.url, "http://127.0.0.1:9/again");
}

fn save_dialog_with_open_entry_chosen(cx: &mut TestAppContext, s: &Started) {
    use apinata_app::keymap::SaveActive;
    use gpui_kit::test::TestWindowExt as _;
    cx.update_window(s.win, |_, w, cx| {
        w.dispatch_action(Box::new(SaveActive), cx)
    })
    .unwrap();
    frame(cx, s.win);
    // With nothing open, the dialog offers its open button in place of the picker.
    cx.update_window(s.win, |_, w, cx| {
        w.click("save-draft-open-collection", cx);
    })
    .unwrap();
    cx.run_until_parked();
}

// Real file I/O runs off the test executor, so give it wall time before asserting.
fn wait_for_file(cx: &mut TestAppContext, file: &std::path::Path) {
    for _ in 0..100 {
        cx.run_until_parked();
        if file.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    cx.run_until_parked();
}

fn pick_through_seam(cx: &mut TestAppContext, dir: &std::path::Path) -> Started {
    use apinata_app::save_draft::FolderPickSeam;
    let s = started_blank(cx, blank_data_dir());
    scratch_draft(cx, &s);
    let chosen = dir.to_path_buf();
    cx.update(|cx| {
        cx.set_global(FolderPickSeam(std::rc::Rc::new(move || {
            Some(chosen.clone())
        })))
    });
    save_dialog_with_open_entry_chosen(cx, &s);
    wait_for_file(cx, &dir.join("opencollection.yml"));
    s
}

fn is_open(cx: &mut TestAppContext, s: &Started, dir: &std::path::Path) -> bool {
    s.ws.read_with(cx, |w, _| {
        w.user_collections().iter().any(|c| c.root() == dir)
    })
}

#[gpui_kit::test]
fn the_open_entry_creates_and_picks_a_collection_through_the_folder_prompt(
    cx: &mut TestAppContext,
) {
    let dir = tempfile::tempdir().unwrap();
    let s = pick_through_seam(cx, dir.path());
    assert!(is_open(cx, &s, dir.path()), "created and opened");
}

#[gpui_kit::test]
fn the_open_entry_takes_a_directory_holding_only_a_git_folder(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".git")).unwrap();
    let s = pick_through_seam(cx, dir.path());
    assert!(dir.path().join("opencollection.yml").is_file());
    assert!(is_open(cx, &s, dir.path()), "created and opened");
}

#[test]
fn the_refusal_says_what_folder_to_choose() {
    assert_eq!(
        apinata_app::save_draft::not_empty_refusal(std::path::Path::new("/tmp/proj")),
        "\"proj\" is not empty. Choose an empty folder, or one that has an opencollection.yml."
    );
}

#[test]
fn the_welcome_screen_refuses_a_non_empty_folder_in_the_save_dialogs_words() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("notes.txt"), "x").unwrap();
    assert_eq!(
        apinata_app::save_draft::create_collection(dir.path()),
        Err(apinata_app::save_draft::not_empty_refusal(dir.path()))
    );
}

#[gpui_kit::test]
fn the_open_entry_refuses_a_folder_of_other_files_with_one_notice(cx: &mut TestAppContext) {
    use apinata_app::save_draft::FolderPickSeam;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("notes.txt"), "x").unwrap();
    let s = started_blank(cx, blank_data_dir());
    scratch_draft(cx, &s);
    let chosen = dir.path().to_path_buf();
    cx.update(|cx| {
        cx.set_global(FolderPickSeam(std::rc::Rc::new(move || {
            Some(chosen.clone())
        })))
    });
    save_dialog_with_open_entry_chosen(cx, &s);
    // The check runs off the test executor, so give it wall time.
    let toasts = |cx: &mut TestAppContext| {
        cx.update_window(s.win, |_, w, cx| {
            use gpui_kit::component::WindowExt as _;
            w.notifications(cx).len()
        })
        .unwrap()
    };
    for _ in 0..100 {
        cx.run_until_parked();
        if toasts(cx) > 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(toasts(cx), 1, "one refusal notice");
    assert!(!is_open(cx, &s, dir.path()));
    assert!(!dir.path().join("opencollection.yml").exists());
}
