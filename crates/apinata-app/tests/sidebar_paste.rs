use crate::support;

use apinata_app::command::{Command, TreeOp};
use apinata_app::sidebar::Sidebar;
use apinata_app::sidebar::paste::{ClipMode, Clipboard};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use std::path::Path;
use support::*;

fn sidebar(cx: &mut TestAppContext, s: &Started) -> Entity<Sidebar> {
    s.shell.read_with(cx, |sh, _| sh.sidebar().clone())
}

fn toasts(cx: &mut TestAppContext, s: &Started) -> usize {
    cx.update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

fn plain(cx: &mut TestAppContext, s: &Started, path: &str) {
    cx.update_window(s.win, |_, w, cx| w.click(row(path), cx))
        .unwrap();
    frame(cx, s.win);
}

fn copy(cx: &mut TestAppContext, s: &Started) {
    let sb = sidebar(cx, s);
    cx.update_window(s.win, |_, w, cx| {
        sb.update(cx, |sb, cx| sb.copy_selection(w, cx))
    })
    .unwrap();
}

fn cut(cx: &mut TestAppContext, s: &Started) {
    let sb = sidebar(cx, s);
    cx.update_window(s.win, |_, w, cx| {
        sb.update(cx, |sb, cx| sb.cut_selection(w, cx))
    })
    .unwrap();
}

fn paste(cx: &mut TestAppContext, s: &Started, root: &Path, folder: &str) {
    let sb = sidebar(cx, s);
    let (root, folder) = (root.to_path_buf(), ip(folder));
    cx.update_window(s.win, |_, w, cx| {
        sb.update(cx, |sb, cx| sb.paste(&root, folder, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
}

fn root_item() -> String {
    String::new()
}

fn second(cx: &mut TestAppContext, s: &Started, files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = write_tree(files);
    let root = dir.path().to_path_buf();
    s.ws.update(cx, |w, cx| w.open(root, cx));
    cx.run_until_parked();
    frame(cx, s.win);
    dir
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[gpui_kit::test]
fn copy_paste_in_a_collection_adds_a_copy_and_keeps_the_original(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("A", "http://x/a")),
        ("t/folder.yml", &folder("T")),
    ]);
    let s = started(cx, &col);
    plain(cx, &s, "a.yml");
    copy(cx, &s);
    paste(cx, &s, &s.root.clone(), "t");
    paste(cx, &s, &s.root.clone(), "t");
    assert!(col.path().join("a.yml").exists());
    assert_eq!(
        names(&col.path().join("t")),
        ["a (copy).yml", "a.yml", "folder.yml"]
    );
}

#[gpui_kit::test]
fn pasting_beside_the_original_numbers_the_clash(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("A", "http://x/a")),
    ]);
    let s = started(cx, &col);
    plain(cx, &s, "a.yml");
    copy(cx, &s);
    paste(cx, &s, &s.root.clone(), &root_item());
    paste(cx, &s, &s.root.clone(), &root_item());
    assert_eq!(
        names(col.path()),
        [
            "a (copy 2).yml",
            "a (copy).yml",
            "a.yml",
            "opencollection.yml"
        ]
    );
    let text = std::fs::read_to_string(col.path().join("a (copy).yml")).unwrap();
    assert!(text.contains("name: A (copy)"), "{text}");
}

#[gpui_kit::test]
fn copy_paste_across_collections_writes_into_the_other_root(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("A", "http://x/a")),
    ]);
    let s = started(cx, &col);
    let other = second(cx, &s, &[("opencollection.yml", ROOT)]);
    plain(cx, &s, "a.yml");
    copy(cx, &s);
    paste(cx, &s, other.path(), &root_item());
    assert_eq!(names(other.path()), ["a.yml", "opencollection.yml"]);
    assert!(col.path().join("a.yml").exists());
}

#[gpui_kit::test]
fn a_folder_pastes_with_everything_in_it(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("F")),
        ("f/x.yml", &request("X", "http://x/x")),
        ("dest/folder.yml", &folder("Dest")),
    ]);
    let s = started(cx, &col);
    plain(cx, &s, "f");
    copy(cx, &s);
    paste(cx, &s, &s.root.clone(), "dest");
    assert!(col.path().join("dest/f/x.yml").exists());
    assert!(col.path().join("f/x.yml").exists());
}

#[gpui_kit::test]
fn a_cut_moves_only_when_pasted(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("A", "http://x/a")),
        ("t/folder.yml", &folder("T")),
    ]);
    let s = started(cx, &col);
    plain(cx, &s, "a.yml");
    cut(cx, &s);
    assert!(col.path().join("a.yml").exists(), "cut alone moves nothing");
    paste(cx, &s, &s.root.clone(), "t");
    assert!(!col.path().join("a.yml").exists());
    assert!(col.path().join("t/a.yml").exists());
    assert_eq!(names(&col.path().join("t")), ["a.yml", "folder.yml"]);
    // The cut state is spent: a second paste does nothing.
    paste(cx, &s, &s.root.clone(), &root_item());
    assert!(!col.path().join("a.yml").exists());
}

#[gpui_kit::test]
fn a_cut_pasted_where_it_lives_makes_no_copy(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("A", "http://x/a")),
    ]);
    let s = started(cx, &col);
    plain(cx, &s, "a.yml");
    cut(cx, &s);
    paste(cx, &s, &s.root.clone(), &root_item());
    assert_eq!(names(col.path()), ["a.yml", "opencollection.yml"]);
}

#[gpui_kit::test]
fn a_cut_across_collections_deletes_the_source_after_the_paste(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("A", "http://x/a")),
    ]);
    let s = started(cx, &col);
    let other = second(cx, &s, &[("opencollection.yml", ROOT)]);
    plain(cx, &s, "a.yml");
    cut(cx, &s);
    paste(cx, &s, other.path(), &root_item());
    assert!(other.path().join("a.yml").exists());
    assert!(!col.path().join("a.yml").exists());
}

#[gpui_kit::test]
fn a_cancelled_cut_leaves_everything_in_place(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("A", "http://x/a")),
        ("b.yml", &request("B", "http://x/b")),
        ("t/folder.yml", &folder("T")),
    ]);
    let s = started(cx, &col);
    plain(cx, &s, "a.yml");
    cut(cx, &s);
    plain(cx, &s, "b.yml");
    copy(cx, &s);
    paste(cx, &s, &s.root.clone(), "t");
    assert!(col.path().join("a.yml").exists());
    assert!(col.path().join("b.yml").exists());
    assert!(col.path().join("t/b.yml").exists());
    assert!(!col.path().join("t/a.yml").exists());
}

#[gpui_kit::test]
fn a_multi_selection_pastes_every_member(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("A", "http://x/a")),
        ("b.yml", &request("B", "http://x/b")),
        ("t/folder.yml", &folder("T")),
    ]);
    let s = started(cx, &col);
    plain(cx, &s, "a.yml");
    secondary_click(cx, s.win, row("b.yml"));
    copy(cx, &s);
    paste(cx, &s, &s.root.clone(), "t");
    assert!(col.path().join("t/a.yml").exists() && col.path().join("t/b.yml").exists());
}

#[gpui_kit::test]
fn the_system_clipboard_gets_the_paths_one_per_line(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("A", "http://x/a")),
        ("t/folder.yml", &folder("T")),
    ]);
    let s = started(cx, &col);
    plain(cx, &s, "a.yml");
    secondary_click(cx, s.win, row("t"));
    copy(cx, &s);
    let text = cx
        .read_from_clipboard()
        .and_then(|c| c.text())
        .expect("text on the clipboard");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert!(lines.contains(&col.path().join("a.yml").to_str().unwrap()));
    assert!(lines.contains(&col.path().join("t").to_str().unwrap()));
}

#[gpui_kit::test]
fn the_unsaved_edits_notice_appears_only_for_a_dirty_source(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("A", "http://x/a")),
        ("b.yml", &request("B", "http://x/b")),
    ]);
    let s = started(cx, &col);
    plain(cx, &s, "a.yml");
    copy(cx, &s);
    let before = toasts(cx, &s);
    paste(cx, &s, &s.root.clone(), &root_item());
    assert_eq!(toasts(cx, &s), before, "a clean source pushes no notice");

    let root = s.root.clone();
    s.ws.update(cx, |w, cx| {
        w.send(&root, edit_url("b.yml", "http://x/changed"), cx)
    });
    cx.run_until_parked();
    plain(cx, &s, "b.yml");
    copy(cx, &s);
    paste(cx, &s, &s.root.clone(), &root_item());
    assert_eq!(
        toasts(cx, &s),
        before + 1,
        "one notice for the dirty source"
    );
    let text = std::fs::read_to_string(col.path().join("b (copy).yml")).unwrap();
    assert!(text.contains("http://x/b"), "{text}");
}

fn clip(
    cx: &mut TestAppContext,
    s: &Started,
) -> (usize, Option<apinata_app::sidebar::paste::ClipMode>) {
    let sb = sidebar(cx, s);
    sb.read_with(cx, |sb, _| (sb.clipboard_len(), sb.clipboard_mode()))
}

fn lock(path: &Path) -> crate::support::ModeGuard {
    crate::support::ModeGuard::set(path, 0o555)
}

#[gpui_kit::test]
fn a_failed_cross_collection_cut_keeps_the_source_and_the_clipboard(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("A", "http://x/a")),
    ]);
    let s = started(cx, &col);
    let other = second(cx, &s, &[("opencollection.yml", ROOT)]);
    plain(cx, &s, "a.yml");
    cut(cx, &s);
    let _lock = lock(other.path());
    paste(cx, &s, other.path(), &root_item());
    assert!(col.path().join("a.yml").exists(), "the source stays");
    assert!(!other.path().join("a.yml").exists());
    assert_eq!(
        clip(cx, &s),
        (1, Some(apinata_app::sidebar::paste::ClipMode::Cut))
    );
}

#[gpui_kit::test]
fn a_partly_failed_cut_deletes_only_the_sources_that_were_copied(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("A", "http://x/a")),
        ("b.yml", &request("B", "http://x/b")),
    ]);
    let s = started(cx, &col);
    let other = second(cx, &s, &[("opencollection.yml", ROOT)]);
    plain(cx, &s, "a.yml");
    secondary_click(cx, s.win, row("b.yml"));
    cut(cx, &s);
    // b disappears after the cut, so its copy fails while a's succeeds.
    let root = s.root.clone();
    let gone = Command::Tree(TreeOp::Delete { item: ip("b.yml") });
    s.ws.update(cx, |w, cx| w.send(&root, gone, cx));
    cx.run_until_parked();
    paste(cx, &s, other.path(), &root_item());
    assert_eq!(names(other.path()), ["a.yml", "opencollection.yml"]);
    assert!(!col.path().join("a.yml").exists(), "the copied source goes");
    assert_eq!(
        clip(cx, &s),
        (1, Some(apinata_app::sidebar::paste::ClipMode::Cut))
    );
}

#[gpui_kit::test]
fn a_failed_move_within_a_collection_keeps_the_cut(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("A", "http://x/a")),
        ("t/folder.yml", &folder("T")),
    ]);
    let s = started(cx, &col);
    plain(cx, &s, "a.yml");
    cut(cx, &s);
    let _lock = lock(&col.path().join("t"));
    paste(cx, &s, &s.root.clone(), "t");
    assert!(col.path().join("a.yml").exists());
    assert_eq!(
        clip(cx, &s),
        (1, Some(apinata_app::sidebar::paste::ClipMode::Cut))
    );
}

#[gpui_kit::test]
fn a_cut_pasted_into_its_own_folder_stays_cut(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("A", "http://x/a")),
        ("t/folder.yml", &folder("T")),
    ]);
    let s = started(cx, &col);
    plain(cx, &s, "a.yml");
    cut(cx, &s);
    paste(cx, &s, &s.root.clone(), &root_item());
    assert_eq!(
        clip(cx, &s),
        (1, Some(apinata_app::sidebar::paste::ClipMode::Cut))
    );
    paste(cx, &s, &s.root.clone(), "t");
    assert!(col.path().join("t/a.yml").exists(), "it can still move");
}

#[gpui_kit::test]
fn a_cut_of_a_dirty_source_across_collections_keeps_the_source(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("b.yml", &request("B", "http://x/b")),
    ]);
    let s = started(cx, &col);
    let other = second(cx, &s, &[("opencollection.yml", ROOT)]);
    let root = s.root.clone();
    s.ws.update(cx, |w, cx| {
        w.send(&root, edit_url("b.yml", "http://x/changed"), cx)
    });
    cx.run_until_parked();
    plain(cx, &s, "b.yml");
    cut(cx, &s);
    let before = toasts(cx, &s);
    paste(cx, &s, other.path(), &root_item());
    let text = std::fs::read_to_string(other.path().join("b.yml")).unwrap();
    assert!(text.contains("http://x/b"), "the saved version is pasted");
    assert!(col.path().join("b.yml").exists(), "the edits are not lost");
    assert!(toasts(cx, &s) > before, "the user is told");
}

// Trust is keyed by collection root, so a pasted script needs approval again in its new home.
#[gpui_kit::test]
fn script_trust_does_not_carry_to_the_other_collection(cx: &mut TestAppContext) {
    let code = "console.log('hi')";
    let text = format!(
        "{}runtime:\n  scripts:\n    - type: before-request\n      code: \"{code}\"\n",
        request("S", "http://x/s")
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("s.yml", &text)]);
    let s = started(cx, &col);
    let other = second(cx, &s, &[("opencollection.yml", ROOT)]);
    let trust_dir = tempfile::tempdir().unwrap();
    let mut trust = apinata_core::ScriptTrust::empty(trust_dir.path().join("trust.json"));
    let hash = apinata_core::script_hash(code);
    trust.approve(&s.root, [hash.clone()]);

    plain(cx, &s, "s.yml");
    copy(cx, &s);
    paste(cx, &s, other.path(), &root_item());
    let pasted = std::fs::read_to_string(other.path().join("s.yml")).unwrap();
    assert!(pasted.contains(code), "the script itself is copied");

    let hashes = [hash.clone()];
    let gate = |root: &Path, t: &apinata_core::ScriptTrust| {
        apinata_app::script_gate::approved_only(
            &hashes,
            &t.approved(root),
            apinata_core::ScriptOptions::default(),
        )
        .is_some()
    };
    assert!(gate(&s.root, &trust));
    assert!(!gate(other.path(), &trust), "not approved after the paste");
    trust.approve(other.path(), [hash]);
    assert!(gate(other.path(), &trust), "approved once the target does");
}

#[gpui_kit::test]
fn copy_refuses_the_collection_row(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("A", "http://x/a")),
    ]);
    let s = started(cx, &col);
    plain(cx, &s, &root_item());
    copy(cx, &s);
    let sb = sidebar(cx, &s);
    assert_eq!(sb.read_with(cx, |sb, _| sb.clipboard_len()), 0);
}

fn nested() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("F")),
        ("f/sub/folder.yml", &folder("Sub")),
        ("fb/folder.yml", &folder("Fb")),
    ])
}

fn dirs(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = names(dir);
    v.retain(|n| !n.ends_with(".yml"));
    v
}

// The kit's menu does not expose the disabled flag, so a click on the Paste entry is the check.
#[gpui_kit::test]
fn the_paste_entry_is_disabled_inside_the_copied_folder(cx: &mut TestAppContext) {
    use apinata_app::sidebar::menu::MenuEntry;
    for cut_it in [false, true] {
        let col = nested();
        let s = started(cx, &col);
        plain(cx, &s, "f");
        if cut_it {
            cut(cx, &s);
        } else {
            copy(cx, &s);
        }
        let before = toasts(cx, &s);
        for target in ["f", "f/sub"] {
            cx.update_window(s.win, |_, w, cx| w.right_click(row(target), cx))
                .unwrap();
            frame(cx, s.win);
            cx.update_window(s.win, |_, w, cx| {
                let ix = menu_index(w, MenuEntry::PasteItem.label());
                w.within("popup-menu").click(ix, cx)
            })
            .unwrap();
            cx.run_until_parked();
            frame(cx, s.win);
        }
        assert_eq!(dirs(col.path()), ["f", "fb"], "cut={cut_it}");
        assert_eq!(dirs(&col.path().join("f")), ["sub"], "cut={cut_it}");
        assert_eq!(dirs(&col.path().join("f/sub")), Vec::<String>::new());
        assert_eq!(toasts(cx, &s), before, "a disabled entry does nothing");
    }
}

// With nothing cut or copied, every row's Paste entry greys out, the collection row's too.
#[gpui_kit::test]
fn paste_greys_out_while_nothing_is_cut_or_copied(cx: &mut TestAppContext) {
    use apinata_app::sidebar::model::{Node, NodeKind};
    use apinata_core::ItemPath;
    let col = nested();
    let s = started(cx, &col);
    let node = |path: ItemPath, kind| Node {
        id: format!("{path:?}").into(),
        collection: 0,
        root: s.root.clone(),
        path,
        kind,
        name: "n".to_owned(),
        method: None,
        dirty: false,
        disk: Default::default(),
        expanded: false,
        children: Vec::new(),
        example: None,
        hint: None,
        unsupported: None,
    };
    let sb = sidebar(cx, &s);
    let rows = [
        (ItemPath::root(), NodeKind::Collection),
        (ip("f"), NodeKind::Folder),
    ];
    for (path, kind) in rows.clone() {
        let n = node(path.clone(), kind);
        assert!(
            sb.read_with(cx, |sb, _| sb.paste_blocked(&n)),
            "{path:?} empty"
        );
    }
    plain(cx, &s, "fb");
    copy(cx, &s);
    for (path, kind) in rows {
        let n = node(path.clone(), kind);
        assert!(
            !sb.read_with(cx, |sb, _| sb.paste_blocked(&n)),
            "{path:?} copied"
        );
    }
}

// The chord and a paste sent past the menu still end in a toast that says why.
#[gpui_kit::test]
fn pasting_a_folder_into_itself_is_refused_with_a_toast(cx: &mut TestAppContext) {
    for cut_it in [false, true] {
        let col = nested();
        let s = started(cx, &col);
        plain(cx, &s, "f");
        if cut_it {
            cut(cx, &s);
        } else {
            copy(cx, &s);
        }
        for target in ["f", "f/sub"] {
            let before = toasts(cx, &s);
            paste(cx, &s, &s.root.clone(), target);
            assert!(toasts(cx, &s) > before, "{target} cut={cut_it}");
        }
        assert_eq!(dirs(col.path()), ["f", "fb"], "cut={cut_it}");
        assert_eq!(dirs(&col.path().join("f")), ["sub"], "cut={cut_it}");
        assert_eq!(dirs(&col.path().join("f/sub")), Vec::<String>::new());
    }
}

fn clipboard_of(mode: ClipMode, root: &Path, items: &[&str]) -> Clipboard {
    Clipboard {
        mode,
        root: root.to_path_buf(),
        items: items.iter().map(|i| ip(i)).collect(),
        generation: 0,
    }
}

#[test]
fn the_clipboard_blocks_a_folder_only_into_itself_or_below() {
    let root = Path::new("/c");
    let cases = [
        ("f", true),
        ("f/sub", true),
        ("f/sub/deep", true),
        ("fb", false),
        ("f2/sub", false),
        ("", false),
    ];
    for mode in [ClipMode::Copy, ClipMode::Cut] {
        let c = clipboard_of(mode, root, &["f"]);
        for (target, blocked) in cases {
            assert_eq!(c.blocks_paste_into(root, &ip(target)), blocked, "{target}");
            let message = c.refusal(root, &ip(target));
            assert_eq!(
                message.as_deref(),
                blocked.then_some("A folder cannot be pasted into itself or one of its subfolders"),
                "{target}"
            );
        }
        let other = Path::new("/other");
        assert!(!c.blocks_paste_into(other, &ip("f")), "another collection");
    }
}

#[gpui_kit::test]
fn the_sidebar_blocks_paste_on_rows_inside_the_copied_folder(cx: &mut TestAppContext) {
    use apinata_app::sidebar::model::{Node, NodeKind};
    let col = nested();
    let s = started(cx, &col);
    let node = |path: &str, kind| Node {
        id: path.to_owned().into(),
        collection: 0,
        root: s.root.clone(),
        path: ip(path),
        kind,
        name: path.to_owned(),
        method: None,
        dirty: false,
        disk: Default::default(),
        expanded: false,
        children: Vec::new(),
        example: None,
        hint: None,
        unsupported: None,
    };
    let sb = sidebar(cx, &s);
    plain(cx, &s, "f");
    copy(cx, &s);
    let rows = [
        ("f", NodeKind::Folder, true),
        ("f/sub", NodeKind::Folder, true),
        ("f/sub/x.yml", NodeKind::Request, true),
        ("fb", NodeKind::Folder, false),
        ("fb/y.yml", NodeKind::Request, false),
    ];
    for (path, kind, blocked) in rows {
        let n = node(path, kind);
        let got = sb.read_with(cx, |sb, _| sb.paste_blocked(&n));
        assert_eq!(got, blocked, "{path}");
    }
}

// The source closes and reopens once the copy is on its way to the other collection, so the
// cut's delete would reach an instance the copy was never taken from.
#[gpui_kit::test]
fn a_cut_source_reopened_before_the_copy_lands_keeps_its_item(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("A", "http://x/a")),
    ]);
    let s = started(cx, &col);
    let other = second(cx, &s, &[("opencollection.yml", ROOT)]);
    plain(cx, &s, "a.yml");
    cut(cx, &s);
    // The workspace notifies before it emits the target's Created, so this observer reopens
    // the source after the paste is sent and before the sidebar sees that notice.
    let (root, landed) = (s.root.clone(), other.path().join("a.yml"));
    let done = std::rc::Rc::new(std::cell::Cell::new(false));
    let flag = done.clone();
    let _reopen = cx.update(|cx| {
        cx.observe(&s.ws, move |ws, cx| {
            if flag.get() || !landed.exists() {
                return;
            }
            flag.set(true);
            ws.update(cx, |ws, cx| {
                ws.close(&root, cx);
                ws.open(root.clone(), cx);
            });
        })
    });
    paste(cx, &s, other.path(), &root_item());
    cx.run_until_parked();
    frame(cx, s.win);
    assert!(done.get(), "the source was reopened mid-cut");
    assert!(other.path().join("a.yml").exists(), "the copy is written");
    assert!(
        col.path().join("a.yml").exists(),
        "the reopened source keeps it"
    );
}

// The source closes and reopens before it answers the copy request; whatever the old
// instance does with it, the reopened one keeps the item.
#[gpui_kit::test]
fn a_cut_source_reopened_before_it_answers_keeps_its_item(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("A", "http://x/a")),
    ]);
    let s = started(cx, &col);
    let other = second(cx, &s, &[("opencollection.yml", ROOT)]);
    plain(cx, &s, "a.yml");
    cut(cx, &s);
    let (sb, root, target) = (sidebar(cx, &s), s.root.clone(), other.path().to_path_buf());
    cx.update_window(s.win, |_, w, cx| {
        sb.update(cx, |sb, cx| {
            sb.paste(&target, apinata_core::ItemPath::root(), w, cx)
        });
        s.ws.update(cx, |ws, cx| {
            ws.close(&root, cx);
            ws.open(root.clone(), cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    assert!(
        col.path().join("a.yml").exists(),
        "the reopened source keeps it"
    );
}

#[gpui_kit::test]
fn a_failed_paste_of_a_dirty_source_shows_no_unsaved_edits_notice(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("A", "http://x/a")),
        ("b.yml", &request("B", "http://x/b")),
    ]);
    let s = started(cx, &col);
    let other = second(cx, &s, &[("opencollection.yml", ROOT)]);
    let root = s.root.clone();
    s.ws.update(cx, |w, cx| {
        w.send(&root, edit_url("b.yml", "http://x/changed"), cx)
    });
    cx.run_until_parked();
    let _lock = lock(other.path());
    let before = toasts(cx, &s);
    plain(cx, &s, "a.yml");
    copy(cx, &s);
    paste(cx, &s, other.path(), &root_item());
    let clean = toasts(cx, &s) - before;
    assert!(clean >= 1, "the failure is reported");
    plain(cx, &s, "b.yml");
    copy(cx, &s);
    paste(cx, &s, other.path(), &root_item());
    assert_eq!(names(other.path()), ["opencollection.yml"]);
    assert_eq!(
        toasts(cx, &s) - before - clean,
        clean,
        "a dirty source fails with the same toasts as a clean one, no unsaved notice"
    );
}

/// Whether an error toast and a warning toast are on screen, by the marker each one holds.
fn failure_toasts(cx: &mut TestAppContext, s: &Started) -> (bool, bool) {
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        (
            w.try_find("toast-error").is_some(),
            w.try_find("toast-warning").is_some(),
        )
    })
    .unwrap()
}

fn two_requests() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("A", "http://x/a")),
        ("b.yml", &request("B", "http://x/b")),
    ])
}

// Every write fails in a read-only target, so the paste failed outright.
#[gpui_kit::test]
fn a_paste_whose_every_write_fails_shows_an_error_toast(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let other = second(cx, &s, &[("opencollection.yml", ROOT)]);
    plain(cx, &s, "a.yml");
    secondary_click(cx, s.win, row("b.yml"));
    copy(cx, &s);
    let _lock = lock(other.path());
    paste(cx, &s, other.path(), &root_item());
    assert_eq!(names(other.path()), ["opencollection.yml"]);
    assert_eq!(failure_toasts(cx, &s), (true, false));
}

// The only source is gone before the paste, so nothing is sent at all.
#[gpui_kit::test]
fn a_paste_whose_every_source_vanished_shows_an_error_toast(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let other = second(cx, &s, &[("opencollection.yml", ROOT)]);
    plain(cx, &s, "a.yml");
    copy(cx, &s);
    let root = s.root.clone();
    let gone = Command::Tree(TreeOp::Delete { item: ip("a.yml") });
    s.ws.update(cx, |w, cx| w.send(&root, gone, cx));
    cx.run_until_parked();
    paste(cx, &s, other.path(), &root_item());
    assert_eq!(failure_toasts(cx, &s), (true, false));
}

// b is gone before the paste while a pastes, so the paste partly failed.
#[gpui_kit::test]
fn a_two_item_paste_with_one_failure_shows_a_warning_toast(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let other = second(cx, &s, &[("opencollection.yml", ROOT)]);
    plain(cx, &s, "a.yml");
    secondary_click(cx, s.win, row("b.yml"));
    copy(cx, &s);
    let root = s.root.clone();
    let gone = Command::Tree(TreeOp::Delete { item: ip("b.yml") });
    s.ws.update(cx, |w, cx| w.send(&root, gone, cx));
    cx.run_until_parked();
    paste(cx, &s, other.path(), &root_item());
    assert_eq!(names(other.path()), ["a.yml", "opencollection.yml"]);
    assert_eq!(failure_toasts(cx, &s), (false, true));
}

#[gpui_kit::test]
fn a_clean_paste_shows_no_failure_toast(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let other = second(cx, &s, &[("opencollection.yml", ROOT)]);
    plain(cx, &s, "a.yml");
    secondary_click(cx, s.win, row("b.yml"));
    copy(cx, &s);
    paste(cx, &s, other.path(), &root_item());
    assert_eq!(
        names(other.path()),
        ["a.yml", "b.yml", "opencollection.yml"]
    );
    assert_eq!(failure_toasts(cx, &s), (false, false));
}

// Both sources are dirty; the target's ignore rule refuses b's write after copy-out while a
// lands, so the batch is partial and only a counts toward the unsaved notice.
#[gpui_kit::test]
fn a_partial_paste_of_two_dirty_sources_notes_only_the_written_one(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let ignoring = format!("{ROOT}extensions:\n  bruno:\n    ignore:\n      - 'b.yml'\n");
    let other = second(cx, &s, &[("opencollection.yml", &ignoring)]);
    let root = s.root.clone();
    for path in ["a.yml", "b.yml"] {
        s.ws.update(cx, |w, cx| {
            w.send(&root, edit_url(path, "http://x/changed"), cx)
        });
    }
    cx.run_until_parked();
    plain(cx, &s, "a.yml");
    secondary_click(cx, s.win, row("b.yml"));
    copy(cx, &s);
    let before = toasts(cx, &s);
    paste(cx, &s, other.path(), &root_item());
    assert_eq!(names(other.path()), ["a.yml", "opencollection.yml"]);
    assert_eq!(failure_toasts(cx, &s), (false, true));
    assert_eq!(
        toasts(cx, &s),
        before + 2,
        "one unsaved notice for the written copy and one warning for the refused one"
    );
}

// Closes and reopens the collection at `root` inside one update, so the sidebar first sees
// the root open again as a new instance.
fn reopen(cx: &mut TestAppContext, s: &Started, root: &Path) {
    let root = root.to_path_buf();
    cx.update_window(s.win, |_, _, cx| {
        s.ws.update(cx, |ws, cx| {
            ws.close(&root, cx);
            ws.open(root.clone(), cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
}

// A cut is bound to the instance it was taken from, so closing its source empties the
// clipboard and a later paste has nothing to move.
#[gpui_kit::test]
fn closing_a_cuts_source_drops_the_cut(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let other = second(cx, &s, &[("opencollection.yml", ROOT)]);
    plain(cx, &s, "a.yml");
    cut(cx, &s);
    let root = s.root.clone();
    s.ws.update(cx, |w, cx| w.close(&root, cx));
    cx.run_until_parked();
    frame(cx, s.win);
    assert_eq!(clip(cx, &s), (0, None));
    paste(cx, &s, other.path(), &root_item());
    assert_eq!(names(other.path()), ["opencollection.yml"]);
    assert!(col.path().join("a.yml").exists());
}

// A reopened source is a new instance the cut never looked at, so pasting it elsewhere
// neither copies nor deletes.
#[gpui_kit::test]
fn a_cut_whose_source_reopened_pastes_nothing_into_another_collection(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let other = second(cx, &s, &[("opencollection.yml", ROOT)]);
    plain(cx, &s, "a.yml");
    cut(cx, &s);
    reopen(cx, &s, &s.root.clone());
    paste(cx, &s, other.path(), &root_item());
    assert_eq!(names(other.path()), ["opencollection.yml"]);
    assert_eq!(names(col.path()), ["a.yml", "b.yml", "opencollection.yml"]);
    assert_eq!(clip(cx, &s), (0, None));
}

// The same for a cut pasted back into its reopened collection: nothing moves.
#[gpui_kit::test]
fn a_cut_whose_source_reopened_does_not_move_within_it(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("A", "http://x/a")),
        ("t/folder.yml", &folder("T")),
    ]);
    let s = started(cx, &col);
    plain(cx, &s, "a.yml");
    cut(cx, &s);
    reopen(cx, &s, &s.root.clone());
    paste(cx, &s, &s.root.clone(), "t");
    assert!(col.path().join("a.yml").exists());
    assert_eq!(names(&col.path().join("t")), ["folder.yml"]);
}

// A copy only reads its source, so it survives a reopen and still pastes.
#[gpui_kit::test]
fn a_copy_survives_its_source_reopening_and_still_pastes(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let other = second(cx, &s, &[("opencollection.yml", ROOT)]);
    plain(cx, &s, "a.yml");
    copy(cx, &s);
    reopen(cx, &s, &s.root.clone());
    assert_eq!(clip(cx, &s), (1, Some(ClipMode::Copy)));
    paste(cx, &s, other.path(), &root_item());
    assert_eq!(names(other.path()), ["a.yml", "opencollection.yml"]);
    assert!(col.path().join("a.yml").exists());
}

// The source answers the copy request and then closes before the sidebar sees the answer,
// so the copies come back for a cut that no longer exists; writing them would leave a
// duplicate beside the kept source.
#[gpui_kit::test]
fn a_cut_whose_source_closed_after_answering_writes_no_copy(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let other = second(cx, &s, &[("opencollection.yml", ROOT)]);
    plain(cx, &s, "a.yml");
    cut(cx, &s);
    let (sb, root, target) = (sidebar(cx, &s), s.root.clone(), other.path().to_path_buf());
    cx.update_window(s.win, |_, w, cx| {
        sb.update(cx, |sb, cx| {
            sb.paste(&target, apinata_core::ItemPath::root(), w, cx)
        })
    })
    .unwrap();
    // Only background work runs: the actor replies, the waiting paste does not resume yet.
    while cx.dispatcher.tick(true) {}
    s.ws.update(cx, |ws, cx| ws.close(&root, cx));
    cx.run_until_parked();
    frame(cx, s.win);
    assert_eq!(names(other.path()), ["opencollection.yml"]);
    assert!(col.path().join("a.yml").exists());
}

// The reopen and the paste share one update, so the sidebar has not yet seen the close when
// it pastes; the paste itself must notice the cut's instance is gone, or the reopened
// instance would move the item.
#[gpui_kit::test]
fn a_paste_in_the_update_that_reopened_the_cuts_source_moves_nothing(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("A", "http://x/a")),
        ("t/folder.yml", &folder("T")),
    ]);
    let s = started(cx, &col);
    plain(cx, &s, "a.yml");
    cut(cx, &s);
    let (sb, root) = (sidebar(cx, &s), s.root.clone());
    cx.update_window(s.win, |_, w, cx| {
        s.ws.update(cx, |ws, cx| {
            ws.close(&root, cx);
            ws.open(root.clone(), cx);
        });
        sb.update(cx, |sb, cx| sb.paste(&root, ip("t"), w, cx));
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    assert!(col.path().join("a.yml").exists());
    assert_eq!(names(&col.path().join("t")), ["folder.yml"]);
}
