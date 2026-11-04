//! Bruno's ignore rule on reload: `is_ignored`, events, list edits and watched directories.

use crate::tree_support;

use apinata_core::{ChangeKind::*, DiskState, IgnorePolicy, ItemKind::*, OpenOptions};
use tree_support::{ROOT, ip, open, reload, request, write};

fn open_as(
    policy: IgnorePolicy,
    files: &[(&str, &str)],
) -> (tempfile::TempDir, apinata_core::Collection) {
    let (dir, _) = open(files);
    let c = apinata_core::Collection::open_with(dir.path(), &OpenOptions::new().ignore(policy))
        .unwrap();
    (dir, c)
}

fn open_app(files: &[(&str, &str)]) -> (tempfile::TempDir, apinata_core::Collection) {
    open_as(IgnorePolicy::BrunoApp, files)
}

fn root_with(entries: &[&str]) -> String {
    let list: String = entries.iter().map(|e| format!("      - '{e}'\n")).collect();
    format!("{ROOT}extensions:\n  bruno:\n    ignore:\n{list}")
}

#[test]
fn is_ignored_follows_brunos_watcher_rule() {
    let root_file = format!(
        "{ROOT}extensions:\n  bruno:\n    ignore:\n      - skip\n      - 'a\\b'\n      - ''\n      - 'trailing/'\n      - 3\n"
    );
    let (dir, c) = open_app(&[("opencollection.yml", &root_file)]);
    let at = |rel: &str| c.is_ignored(&dir.path().join(rel));
    for yes in [
        "skip",
        "skip/x.yml",
        "a/b",
        "a/b/c.yml",
        "x/node_modules/y.yml",
        ".git",
        "mocks",
        "mocks/m.yml",
    ] {
        assert!(at(yes), "{yes} is ignored");
    }
    for no in [
        "skipper",
        "a",
        "trailing",
        "trailing/x.yml",
        "sub/mocks",
        "opencollection.yml",
        ".env",
        "",
    ] {
        assert!(!at(no), "{no} is not ignored");
    }
    assert!(!c.is_ignored(std::path::Path::new("/elsewhere/skip")));
}

#[test]
fn is_ignored_under_the_cli_policy_hides_only_what_bruno_cli_skips() {
    let (dir, c) = open_as(
        IgnorePolicy::BrunoCli,
        &[("opencollection.yml", &root_with(&["skip"]))],
    );
    let at = |rel: &str| c.is_ignored(&dir.path().join(rel));
    for yes in [
        "node_modules",
        "x/.git/y.yml",
        "x_node_modules_y/b.yml",
        "x_node_modules_y/inner",
    ] {
        assert!(at(yes), "{yes} is ignored");
    }
    for no in [
        "skip",
        "mocks/m.yml",
        "x_node_modules_y",
        "x_node_modules_y/folder.yml",
        "node_modules_notes.yml",
    ] {
        assert!(!at(no), "{no} is not ignored");
    }
}

#[test]
fn events_inside_an_ignored_folder_change_nothing() {
    let (dir, mut c) = open_app(&[
        ("opencollection.yml", &root_with(&["skip"])),
        ("skip/a.yml", &request("a", 1)),
    ]);
    write(&dir, "skip/a.yml", &request("a", 2));
    std::fs::create_dir(dir.path().join("skip/sub")).unwrap();
    write(&dir, "skip/sub/b.yml", &request("b", 1));
    for rel in ["skip", "skip/a.yml", "skip/sub", "skip/sub/b.yml"] {
        assert!(reload(&dir, &mut c, rel).is_empty(), "{rel}");
    }
    assert!(c.reload().is_empty());
    assert!(c.request(&ip("skip/a.yml")).is_none());
}

#[test]
fn growing_the_list_drops_the_newly_ignored_folder_and_keeps_its_draft() {
    let (dir, mut c) = open_app(&[
        ("opencollection.yml", ROOT),
        ("skip/a.yml", &request("a", 1)),
        ("skip/b.yml", &request("b", 2)),
        ("c.yml", &request("c", 3)),
    ]);
    c.request_mut(&ip("skip/a.yml"))
        .unwrap()
        .set_url("https://x.test/draft");
    write(&dir, "opencollection.yml", &root_with(&["skip"]));
    let changes = reload(&dir, &mut c, "opencollection.yml");
    assert!(
        changes.contains(&("opencollection.yml".to_owned(), Modified, Collection)),
        "{changes:?}"
    );
    assert!(
        changes.contains(&("skip".to_owned(), Removed, Folder)),
        "{changes:?}"
    );
    let a = c.request(&ip("skip/a.yml")).expect("the draft is kept");
    assert_eq!(
        (a.url(), a.disk_state()),
        ("https://x.test/draft", DiskState::Removed)
    );
    assert_eq!(
        c.folder(&ip("skip")).unwrap().disk_state(),
        DiskState::Removed
    );
    assert!(
        c.request(&ip("skip/b.yml")).is_none(),
        "the clean item leaves"
    );
    assert!(c.request(&ip("c.yml")).is_some());
    assert!(reload(&dir, &mut c, "skip/b.yml").is_empty());
}

#[test]
fn shrinking_the_list_walks_the_folder_in() {
    let (dir, mut c) = open_app(&[
        ("opencollection.yml", &root_with(&["skip"])),
        ("skip/a.yml", &request("a", 1)),
    ]);
    write(&dir, "opencollection.yml", ROOT);
    let changes = reload(&dir, &mut c, "opencollection.yml");
    assert!(
        changes.contains(&("skip".to_owned(), Added, Folder)),
        "{changes:?}"
    );
    assert!(c.request(&ip("skip/a.yml")).is_some());
    assert!(!c.is_ignored(&dir.path().join("skip")));
}

#[test]
fn a_broken_root_file_keeps_the_last_list() {
    let (dir, mut c) = open_app(&[
        ("opencollection.yml", &root_with(&["skip"])),
        ("skip/a.yml", &request("a", 1)),
    ]);
    write(&dir, "opencollection.yml", "info: [");
    reload(&dir, &mut c, "opencollection.yml");
    assert!(c.is_ignored(&dir.path().join("skip")));
    assert!(c.request(&ip("skip/a.yml")).is_none());
}
#[test]
fn watch_dirs_are_the_root_live_folders_and_environments_unless_ignored() {
    let (dir, mut c) = open_app(&[
        ("opencollection.yml", &root_with(&["skip"])),
        ("skip/a.yml", &request("a", 1)),
        ("mocks/m.yml", &request("m", 1)),
        ("users/u.yml", &request("u", 1)),
        ("users/deep/d.yml", &request("d", 1)),
        ("gone/g.yml", &request("g", 1)),
    ]);
    let root = dir.path().to_path_buf();
    c.request_mut(&ip("gone/g.yml"))
        .unwrap()
        .set_url("https://x.test/draft");
    std::fs::remove_dir_all(root.join("gone")).unwrap();
    c.reload_path(&root.join("gone"));
    assert!(
        c.folder(&ip("gone")).is_some(),
        "kept for its draft, but not watched"
    );
    let expected = vec![
        root.clone(),
        root.join("environments"),
        root.join("users"),
        root.join("users/deep"),
    ];
    assert_eq!(c.watch_dirs(), expected);
    write(
        &dir,
        "opencollection.yml",
        &root_with(&["skip", "environments"]),
    );
    reload(&dir, &mut c, "opencollection.yml");
    assert!(!c.watch_dirs().contains(&root.join("environments")));
}

#[test]
fn a_listed_folder_file_is_not_loaded_so_a_rescan_finds_nothing() {
    let (_dir, mut c) = open_app(&[
        ("opencollection.yml", &root_with(&["x/folder.yml"])),
        ("x/folder.yml", &tree_support::folder("x", 1)),
        ("x/a.yml", &request("a", 1)),
    ]);
    assert!(c.folder(&ip("x")).unwrap().file().is_none());
    assert!(c.reload().is_empty());
}

#[test]
fn tree_edits_refuse_a_destination_the_rule_hides() {
    use apinata_core::{DropTarget, ItemPath, TreeError};
    let files = [
        ("opencollection.yml", root_with(&["new.yml", "hid"])),
        ("a.yml", request("a", 1)),
        ("f/b.yml", request("b", 1)),
    ];
    let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
    let (dir, mut app) = open_app(&files);
    let (_cli_dir, mut cli) = open_as(IgnorePolicy::BrunoCli, &files);
    let root = ItemPath::root();
    let name = |r: Result<ItemPath, TreeError>| matches!(r, Err(TreeError::InvalidName { .. }));
    assert!(name(app.create_folder(&root, "mocks")));
    assert!(name(app.create_request(&root, "new")));
    assert!(name(app.clone_request(&ip("a.yml"), "new")));
    assert!(name(app.rename(&ip("a.yml"), "new", true)));
    assert!(name(app.rename(&ip("f"), "hid", true)));
    assert!(name(cli.create_folder(&root, "node_modules")));
    app.move_item(&ip("a.yml"), DropTarget::Inside(ip("f")))
        .expect("a visible destination is allowed");
    write(
        &dir,
        "opencollection.yml",
        &root_with(&["new.yml", "f/b.yml"]),
    );
    reload(&dir, &mut app, "opencollection.yml");
    std::fs::create_dir(dir.path().join("g")).unwrap();
    write(&dir, "g/b.yml", &request("b", 1));
    reload(&dir, &mut app, "g");
    assert!(matches!(
        app.move_item(&ip("g/b.yml"), DropTarget::Inside(ip("f"))),
        Err(TreeError::InvalidMove { .. })
    ));
    assert!(app.reload().is_empty());
}
