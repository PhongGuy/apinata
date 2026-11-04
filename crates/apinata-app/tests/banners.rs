use crate::support;

use apinata_app::banner::{Banner, Choice, banners, commands};
use apinata_app::command::{Command, Edit, Target};
use apinata_app::snapshot::DiskState;
use gpui_kit::AppContext as _;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt;
use support::*;

#[test]
fn each_disk_state_and_held_edits_have_a_banner() {
    assert!(banners(DiskState::Present, 0).is_empty());
    assert_eq!(banners(DiskState::Changed, 0), [Banner::ChangedOnDisk]);
    assert_eq!(
        banners(DiskState::Removed, 2),
        [Banner::DeletedOnDisk, Banner::HeldEdits(2)]
    );
    assert_eq!(banners(DiskState::Broken, 0), [Banner::BrokenOnDisk]);
}

#[test]
fn choices_map_to_commands() {
    let t = Target::Request(ip("a.yml"));
    assert!(matches!(
        commands(Banner::ChangedOnDisk, Choice::Primary, &t, &[])[..],
        [Command::Revert(_)]
    ));
    assert!(matches!(
        commands(Banner::ChangedOnDisk, Choice::Secondary, &t, &[])[..],
        [Command::Overwrite(_)]
    ));
    assert!(matches!(
        commands(Banner::DeletedOnDisk, Choice::Primary, &t, &[])[..],
        [Command::Save(_)]
    ));
    assert!(matches!(
        commands(Banner::DeletedOnDisk, Choice::Secondary, &t, &[])[..],
        [Command::Revert(_)]
    ));
    assert!(matches!(
        commands(Banner::BrokenOnDisk, Choice::Primary, &t, &[])[..],
        [Command::Overwrite(_)]
    ));
    assert!(matches!(
        commands(Banner::BrokenOnDisk, Choice::Secondary, &t, &[])[..],
        [Command::Revert(_)]
    ));
    // Compare opens a dialog and sends nothing itself.
    assert!(commands(Banner::ChangedOnDisk, Choice::Tertiary, &t, &[]).is_empty());
    let held = [Edit::Url("u".into())];
    assert!(matches!(
        commands(Banner::HeldEdits(1), Choice::Primary, &t, &held)[..],
        [
            Command::DiscardStale(_),
            Command::Edit {
                edit: Edit::Url(_),
                ..
            }
        ]
    ));
    assert!(matches!(
        commands(Banner::HeldEdits(1), Choice::Secondary, &t, &held)[..],
        [Command::DiscardStale(_)]
    ));
}

#[gpui_kit::test]
fn a_save_over_a_changed_file_marks_it_changed(cx: &mut TestAppContext) {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
    ]);
    let model = open(cx, &dir);
    send(cx, &model, edit_url("a.yml", "http://x/mine"));
    cx.run_until_parked();
    std::fs::write(dir.path().join("a.yml"), request("a", "http://x/disk")).unwrap();
    send(cx, &model, Command::Save(Target::Request(ip("a.yml"))));
    cx.run_until_parked();
    assert_eq!(
        snap(cx, &model).content.requests[&ip("a.yml")].disk,
        DiskState::Changed
    );
    assert!(notices(cx, &model).is_empty());
}

#[gpui_kit::test]
fn changed_on_disk_offers_overwrite_and_reload(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let path = col.path().join("a.yml");
    for (round, button) in [(1, "banner-overwrite"), (2, "banner-reload")] {
        type_at_end(cx, win, None, "url", "/mine");
        cx.run_until_parked();
        std::fs::write(&path, request("a", &format!("http://x/disk{round}"))).unwrap();
        ws.update(cx, |w, cx| {
            w.send(&root, Command::Reload(vec![path.clone()]), cx)
        });
        cx.run_until_parked();
        assert!(present(cx, win, "banner-changed"));
        cx.update_window(win, |_, w, cx| w.click(button, cx))
            .unwrap();
        cx.run_until_parked();
        assert!(!present(cx, win, "banner-changed"));
    }
    let disk = std::fs::read_to_string(&path).unwrap();
    assert!(disk.contains("http://x/disk2"), "{disk}");
    assert_eq!(ed.read_with(cx, |e, cx| e.url(cx)), "http://x/disk2");
}

#[gpui_kit::test]
fn deleted_on_disk_offers_save(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let path = col.path().join("a.yml");
    type_at_end(cx, win, None, "url", "/mine");
    cx.run_until_parked();
    std::fs::remove_file(&path).unwrap();
    ws.update(cx, |w, cx| {
        w.send(&root, Command::Reload(vec![path.clone()]), cx)
    });
    cx.run_until_parked();
    assert!(present(cx, win, "banner-deleted"));
    cx.update_window(win, |_, w, cx| w.click("banner-save", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("http://x/0/mine")
    );
    assert!(!present(cx, win, "banner-deleted"));
}

#[gpui_kit::test]
fn broken_on_disk_offers_save_over_it(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let path = col.path().join("a.yml");
    type_at_end(cx, win, None, "url", "/mine");
    cx.run_until_parked();
    std::fs::write(&path, "info: [\n").unwrap();
    ws.update(cx, |w, cx| {
        w.send(&root, Command::Reload(vec![path.clone()]), cx)
    });
    cx.run_until_parked();
    assert!(present(cx, win, "banner-broken"));
    cx.update_window(win, |_, w, cx| w.click("banner-save", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("http://x/0/mine")
    );
    assert!(!present(cx, win, "banner-broken"));
}

#[gpui_kit::test]
fn held_edits_can_be_dropped(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let path = col.path().join("a.yml");
    std::fs::write(&path, request("a", "http://x/disk")).unwrap();
    ws.update(cx, |w, cx| {
        w.send(&root, Command::Reload(vec![path.clone()]), cx)
    });
    type_at_end(cx, win, None, "url", "Z");
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.click("banner-drop", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(!present(cx, win, "banner-held"));
    assert_eq!(view_of(cx, &ws, &root, "a.yml").url, "http://x/disk");
    assert_eq!(ed.read_with(cx, |e, cx| e.url(cx)), "http://x/disk");
}

#[gpui_kit::test]
fn a_failed_save_on_close_shows_the_banner_in_the_kept_tab(cx: &mut TestAppContext) {
    let a = open_area(cx);
    open_tab(cx, &a.ws, &a.root, "a.yml");
    a.ws.update(cx, |w, cx| {
        w.send(&a.root, edit_url("a.yml", "http://x/1"), cx)
    });
    cx.run_until_parked();
    std::fs::write(a.col.path().join("a.yml"), request("a", "http://x/disk")).unwrap();
    cx.update_window(a.win, |_, w, cx| w.click(("tab-close", 0usize), cx))
        .unwrap();
    cx.run_until_parked();
    answer_confirm(cx, "confirm-save");
    cx.run_until_parked();
    assert_eq!(tab_names(cx, &a.ws), ["a.yml"]);
    assert!(present(cx, a.win, "banner-changed"));
}

#[gpui_kit::test]
fn held_edits_offer_the_users_value(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let path = col.path().join("a.yml");
    std::fs::write(&path, request("a", "http://x/disk")).unwrap();
    ws.update(cx, |w, cx| {
        w.send(&root, Command::Reload(vec![path.clone()]), cx)
    });
    type_at_end(cx, win, None, "url", "Z");
    cx.run_until_parked();
    assert!(present(cx, win, "banner-held"));
    assert_eq!(ed.read_with(cx, |e, cx| e.url(cx)), "http://x/disk");
    cx.update_window(win, |_, w, cx| w.click("banner-apply", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(view_of(cx, &ws, &root, "a.yml").url, "http://x/0Z");
    assert!(!present(cx, win, "banner-held"));
    let stale = ws
        .read_with(cx, |w, cx| w.snapshot(&root, cx))
        .unwrap()
        .stale
        .len();
    assert_eq!(stale, 0);
}

#[gpui_kit::test]
fn broken_on_disk_can_be_discarded(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let path = col.path().join("a.yml");
    type_at_end(cx, win, None, "url", "/mine");
    cx.run_until_parked();
    std::fs::write(&path, "info: [\n").unwrap();
    ws.update(cx, |w, cx| {
        w.send(&root, Command::Reload(vec![path.clone()]), cx)
    });
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.click("banner-discard", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "info: [\n");
    assert!(!present(cx, win, "banner-broken"));
}

#[gpui_kit::test]
fn apply_mine_racing_a_newer_reload_holds_the_edit_again(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let path = col.path().join("a.yml");
    std::fs::write(&path, request("a", "http://x/disk")).unwrap();
    ws.update(cx, |w, cx| {
        w.send(&root, Command::Reload(vec![path.clone()]), cx)
    });
    type_at_end(cx, win, None, "url", "Z");
    cx.run_until_parked();
    assert!(present(cx, win, "banner-held"));
    // A second reload the editor has not seen yet reaches the actor before the click.
    std::fs::write(&path, request("a", "http://x/disk2")).unwrap();
    ws.update(cx, |w, cx| {
        w.send(&root, Command::Reload(vec![path.clone()]), cx)
    });
    cx.update_window(win, |_, w, cx| w.click("banner-apply", cx))
        .unwrap();
    cx.run_until_parked();
    let snapshot = ws.read_with(cx, |w, cx| w.snapshot(&root, cx)).unwrap();
    let held = snapshot.stale.get(&Target::Request(ip("a.yml")));
    assert!(
        matches!(held.map(|h| &h[..]), Some([Edit::Url(u)]) if u == "http://x/0Z"),
        "the edit is held again, {} held",
        held.map_or(0, Vec::len)
    );
    assert!(present(cx, win, "banner-held"));
}

// An outside change of only `info.seq` joins the draft and the saved copy, so it raises no
// banner; the folder's unsaved edit survives and its Save lands both.
#[gpui_kit::test]
fn a_seq_only_outside_change_to_a_dirty_folder_raises_no_banner(cx: &mut TestAppContext) {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("f")),
        ("f/a.yml", &request("a", "http://x/0")),
    ]);
    let model = open(cx, &dir);
    let target = Target::Folder(ip("f"));
    send(
        cx,
        &model,
        Command::Edit {
            target: target.clone(),
            edit: Edit::Docs("mine".into()),
        },
    );
    cx.run_until_parked();
    let path = dir.path().join("f/folder.yml");
    std::fs::write(&path, folder("f").replace("seq: 1", "seq: 7")).unwrap();
    send(cx, &model, Command::Reload(vec![path.clone()]));
    cx.run_until_parked();
    let view = snap(cx, &model).content.folders[&ip("f")].clone();
    assert_eq!(view.disk, DiskState::Present, "no changed-on-disk state");
    assert!(view.dirty && view.docs == "mine", "the draft is kept");
    send(cx, &model, Command::Save(target));
    cx.run_until_parked();
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(
        saved.contains("seq: 7") && saved.contains("mine"),
        "{saved}"
    );
}
