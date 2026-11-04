//! Pasting a request or a folder subtree from one collection into another folder.

use crate::tree_support;

use apinata_core::{ItemPath, TreeError};
use tree_support::{ROOT, folder, ip, item, open, read};

/// A request file as the writer lays it out, with a blank line between sections.
fn request(name: &str, seq: u64) -> String {
    tree_support::request(name, seq).replace("\nhttp:", "\n\nhttp:")
}

#[test]
fn no_clash_keeps_the_name_and_matches_a_normal_write() {
    let (_src_dir, src) = open(&[
        ("opencollection.yml", ROOT),
        ("Get users.yml", &request("Get users", 1)),
    ]);
    let (dst_dir, mut dst) = open(&[("opencollection.yml", ROOT)]);
    let source = src.paste_source(&ip("Get users.yml")).expect("source");
    let made = dst.paste(&ItemPath::root(), &source).expect("pastes");
    assert_eq!(made, ip("Get users.yml"));
    // seq is 1 in the empty target, as in the source, so the bytes are the source's.
    assert_eq!(read(&dst_dir, "Get users.yml"), request("Get users", 1));
    let r = dst.request(&made).expect("in the tree");
    assert_eq!(r.name(), "Get users");
    assert!(!r.is_dirty());
}

#[test]
fn a_clash_appends_copy_then_copy_2_to_file_and_name() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request("a", 1))]);
    let source = c.paste_source(&ip("a.yml")).expect("source");
    let first = c.paste(&ItemPath::root(), &source).expect("pastes");
    assert_eq!(first, ip("a (copy).yml"));
    assert_eq!(c.request(&first).expect("there").name(), "a (copy)");
    assert!(read(&dir, "a (copy).yml").contains("name: a (copy)\n"));
    let second = c.paste(&ItemPath::root(), &source).expect("pastes");
    assert_eq!(second, ip("a (copy 2).yml"));
    assert_eq!(c.request(&second).expect("there").name(), "a (copy 2)");
    assert_eq!(c.request(&second).expect("there").seq(), Some(3));
}

#[test]
fn a_name_clash_without_a_file_clash_still_renames() {
    // The request named `a` lives in `other.yml`, so only the display name clashes.
    let (_dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("other.yml", &request("a", 1)),
        ("f/folder.yml", &folder("f", 2)),
        ("f/a.yml", &request("zzz", 1)),
    ]);
    let source = c.paste_source(&ip("f/a.yml")).expect("source");
    let made = c.paste(&ItemPath::root(), &source).expect("pastes");
    assert_eq!(made, ip("a.yml"));
    let again = c.paste(&ItemPath::root(), &source).expect("pastes");
    assert_eq!(again, ip("a (copy).yml"));
}

#[test]
fn a_folder_is_copied_recursively_with_its_folder_file() {
    let (src_dir, src) = open(&[
        ("opencollection.yml", ROOT),
        ("api/folder.yml", &folder("api", 1)),
        ("api/one.yml", &request("one", 1)),
        ("api/deep/folder.yml", &folder("deep", 2)),
        ("api/deep/two.yml", &request("two", 1)),
        (
            "api/g.yml",
            &item("graphql", "g", 3).replace("\ngraphql:", "\n\ngraphql:"),
        ),
    ]);
    let (dst_dir, mut dst) = open(&[("opencollection.yml", ROOT)]);
    let source = src.paste_source(&ip("api")).expect("source");
    let made = dst.paste(&ItemPath::root(), &source).expect("pastes");
    assert_eq!(made, ip("api"));
    for rel in [
        "api/folder.yml",
        "api/one.yml",
        "api/deep/folder.yml",
        "api/deep/two.yml",
        "api/g.yml",
    ] {
        assert_eq!(read(&dst_dir, rel), read(&src_dir, rel), "{rel}");
    }
    assert!(dst.request(&ip("api/deep/two.yml")).is_some());
    assert_eq!(dst.folder(&made).expect("folder").folders().len(), 1);
}

#[test]
fn a_clashing_folder_gets_the_suffix_on_directory_and_name() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("api/folder.yml", &folder("api", 1)),
        ("api/one.yml", &request("one", 1)),
    ]);
    let source = c.paste_source(&ip("api")).expect("source");
    let made = c.paste(&ItemPath::root(), &source).expect("pastes");
    assert_eq!(made, ip("api (copy)"));
    assert_eq!(c.folder(&made).expect("folder").name(), "api (copy)");
    assert!(read(&dir, "api (copy)/folder.yml").contains("name: api (copy)\n"));
    assert!(read(&dir, "api (copy)/folder.yml").contains("seq: 2\n"));
    assert_eq!(read(&dir, "api (copy)/one.yml"), read(&dir, "api/one.yml"));
    let second = c.paste(&ItemPath::root(), &source).expect("pastes");
    assert_eq!(second, ip("api (copy 2)"));
}

#[test]
fn a_request_pastes_into_a_folder_of_another_collection() {
    let (_s, src) = open(&[("opencollection.yml", ROOT), ("a.yml", &request("a", 1))]);
    let (dir, mut dst) = open(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("f", 1)),
        ("f/x.yml", &request("x", 1)),
    ]);
    let source = src.paste_source(&ip("a.yml")).expect("source");
    let made = dst.paste(&ip("f"), &source).expect("pastes");
    assert_eq!(made, ip("f/a.yml"));
    assert_eq!(dst.request(&made).expect("there").seq(), Some(2));
    assert!(read(&dir, "f/a.yml").contains("seq: 2\n"));
}

#[test]
fn the_source_is_the_saved_content_and_unsaved_edits_are_flagged() {
    let (_s, mut src) = open(&[("opencollection.yml", ROOT), ("a.yml", &request("a", 1))]);
    assert!(
        !src.paste_source(&ip("a.yml"))
            .expect("source")
            .has_unsaved_edits()
    );
    src.request_mut(&ip("a.yml"))
        .expect("there")
        .set_method("DELETE");
    let source = src.paste_source(&ip("a.yml")).expect("source");
    assert!(source.has_unsaved_edits());
    let (dir, mut dst) = open(&[("opencollection.yml", ROOT)]);
    dst.paste(&ItemPath::root(), &source).expect("pastes");
    assert_eq!(read(&dir, "a.yml"), request("a", 1));
}

#[test]
fn an_unsaved_item_inside_a_folder_flags_the_folder() {
    let (_s, mut src) = open(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("f", 1)),
        ("f/a.yml", &request("a", 1)),
    ]);
    src.request_mut(&ip("f/a.yml"))
        .expect("there")
        .set_method("PUT");
    assert!(
        src.paste_source(&ip("f"))
            .expect("source")
            .has_unsaved_edits()
    );
}

#[test]
fn a_folder_cannot_be_pasted_into_itself_or_below() {
    let (_d, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("f", 1)),
        ("f/sub/folder.yml", &folder("sub", 1)),
        ("fb/folder.yml", &folder("fb", 2)),
    ]);
    let source = c.paste_source(&ip("f")).expect("source");
    // A sibling whose name only starts with the same letters is not below the folder.
    c.paste(&ip("fb"), &source)
        .expect("a prefix sibling is outside f");
    for target in ["f", "f/sub"] {
        let err = c.paste(&ip(target), &source).expect_err("refused");
        assert!(matches!(err, TreeError::IntoItself { .. }), "{target}");
        assert_eq!(
            err.to_string(),
            "A folder cannot be pasted into itself or one of its subfolders"
        );
    }
}

#[test]
fn the_root_and_unknown_items_are_not_sources() {
    let (_d, c) = open(&[("opencollection.yml", ROOT)]);
    assert!(matches!(
        c.paste_source(&ItemPath::root()),
        Err(TreeError::InvalidMove { .. })
    ));
    assert!(matches!(
        c.paste_source(&ip("nope.yml")),
        Err(TreeError::NotFound { .. })
    ));
}

#[test]
fn a_missing_target_folder_is_not_found() {
    let (_s, src) = open(&[("opencollection.yml", ROOT), ("a.yml", &request("a", 1))]);
    let (_d, mut dst) = open(&[("opencollection.yml", ROOT)]);
    let source = src.paste_source(&ip("a.yml")).expect("source");
    assert!(matches!(
        dst.paste(&ip("nope"), &source),
        Err(TreeError::NotFound { .. })
    ));
}

// The pre-chosen directory name is taken by the time the write runs. A dangling symlink
// stands in for the racing writer: the free-name check sees nothing there, the create does.
// The copy then numbers its directory and display name alike, as a folder clone does.
#[cfg(unix)]
#[test]
fn a_folder_whose_directory_is_taken_at_write_time_numbers_directory_and_name() {
    let (_src_dir, src) = open(&[
        ("opencollection.yml", ROOT),
        ("api/folder.yml", &folder("api", 1)),
        ("api/one.yml", &request("one", 1)),
    ]);
    let (dst_dir, mut dst) = open(&[("opencollection.yml", ROOT)]);
    std::os::unix::fs::symlink(dst_dir.path().join("gone"), dst_dir.path().join("api"))
        .expect("symlink");
    let source = src.paste_source(&ip("api")).expect("source");
    let made = dst.paste(&ItemPath::root(), &source).expect("pastes");
    assert_eq!(made, ip("api 1"));
    assert_eq!(dst.folder(&made).expect("folder").name(), "api 1");
    assert!(read(&dst_dir, "api 1/folder.yml").contains("name: api 1\n"));
}
