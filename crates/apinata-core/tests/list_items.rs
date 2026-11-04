//! `Collection::list_items`: sibling order, nesting, tags and the refusals.

use crate::common::write_tree;
use apinata_core::{
    Collection, ListItemsError, ListOptions, ListedKind, MAX_PAGE_LIMIT, PageError,
};
use tempfile::TempDir;

const ROOT: &str = "opencollection: 1.0.0\ninfo:\n  name: c\n";

fn request(name: &str, seq: u32, tags: &[&str]) -> String {
    let tags = if tags.is_empty() {
        String::new()
    } else {
        let items: String = tags.iter().map(|t| format!("    - {t}\n")).collect();
        format!("  tags:\n{items}")
    };
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\n{tags}http:\n  method: POST\n  url: \"http://x/{name}\"\n"
    )
}

fn open() -> (TempDir, Collection) {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("b.yml", &request("b", 2, &["smoke", "slow"])),
        ("a.yml", &request("a", 1, &[])),
        ("users/get.yml", &request("get", 1, &["smoke"])),
        ("users/inner/deep.yml", &request("deep", 1, &[])),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    (dir, c)
}

#[test]
fn folders_come_before_requests_and_requests_follow_seq() {
    let (_dir, c) = open();
    let list = c.list_items(None).unwrap();
    let paths: Vec<&str> = list.items().iter().map(|i| i.path()).collect();
    assert_eq!(paths, ["users", "a.yml", "b.yml"]);
    assert_eq!(list.items()[0].kind(), ListedKind::Folder);
    assert_eq!(list.items()[1].kind(), ListedKind::Request);
    assert_eq!(list.items()[1].seq(), Some(1));
    assert_eq!(list.items()[1].method(), Some("POST"));
    assert_eq!(list.items()[1].url(), Some("http://x/a"));
}

#[test]
fn sub_folders_are_nested_in_full() {
    let (_dir, c) = open();
    let list = c.list_items(None).unwrap();
    let users = list.items()[0].children().unwrap();
    assert_eq!(users[0].path(), "users/inner");
    assert_eq!(
        users[0].children().unwrap()[0].path(),
        "users/inner/deep.yml"
    );
    assert_eq!(users[1].path(), "users/get.yml");
    assert!(list.items()[1].children().is_none());
}

#[test]
fn a_path_lists_only_that_folder() {
    let (_dir, c) = open();
    let list = c.list_items(Some("users")).unwrap();
    assert_eq!(list.items().len(), 2);
    assert_eq!(c.list_items(Some("")).unwrap().items().len(), 3);
}

#[test]
fn requests_carry_their_tags() {
    let (_dir, c) = open();
    let list = c.list_items(None).unwrap();
    assert_eq!(list.items()[2].tags(), ["smoke", "slow"]);
    assert!(list.items()[1].tags().is_empty());
    assert!(list.items()[0].tags().is_empty());
}

#[test]
fn bad_paths_are_refused_by_kind() {
    let (_dir, c) = open();
    assert!(matches!(
        c.list_items(Some("../x")),
        Err(ListItemsError::InvalidPath(p)) if p == "../x"
    ));
    assert!(matches!(
        c.list_items(Some("nope")),
        Err(ListItemsError::NoFolder(p)) if p == "nope"
    ));
    let err = c.list_items(Some("a.yml")).unwrap_err();
    assert!(matches!(err, ListItemsError::NoFolder(_)));
    assert_eq!(err.to_string(), "no folder at a.yml");
}

fn folder_file(name: &str) -> String {
    format!("info:\n  name: {name}\n  type: folder\n")
}

fn renamed() -> (TempDir, Collection) {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("Posts/folder.yml", &folder_file("Blog Posts")),
        ("Posts/p.yml", &request("p", 1, &[])),
        ("Posts/Drafts/folder.yml", &folder_file("Old")),
        ("Posts/Drafts/d.yml", &request("d", 1, &[])),
        ("a/folder.yml", &folder_file("Twin")),
        ("b/folder.yml", &folder_file("Twin")),
        ("Twin/t.yml", &request("t", 1, &[])),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    (dir, c)
}

#[test]
fn a_display_name_resolves_to_its_folder() {
    let (_dir, c) = renamed();
    let list = c.list_items(Some("Blog Posts")).unwrap();
    let paths: Vec<&str> = list.items().iter().map(|i| i.path()).collect();
    assert_eq!(paths, ["Posts/Drafts", "Posts/p.yml"]);
}

#[test]
fn display_names_and_directory_names_mix_per_segment() {
    let (_dir, c) = renamed();
    let by_name = c.list_items(Some("Blog Posts/Old")).unwrap();
    let by_dir = c.list_items(Some("Posts/Drafts")).unwrap();
    assert_eq!(by_name.items()[0].path(), "Posts/Drafts/d.yml");
    assert_eq!(by_dir.items()[0].path(), "Posts/Drafts/d.yml");
    assert!(c.list_items(Some("Blog Posts/Drafts")).is_ok());
}

#[test]
fn an_exact_directory_name_beats_a_display_name() {
    let (_dir, c) = renamed();
    let list = c.list_items(Some("Twin")).unwrap();
    assert_eq!(list.items()[0].path(), "Twin/t.yml");
}

#[test]
fn several_display_name_matches_are_ambiguous_and_list_their_paths() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a/folder.yml", &folder_file("Twin")),
        ("b/folder.yml", &folder_file("Twin")),
    ]);
    let c = Collection::open(dir.path()).unwrap();
    let err = c.list_items(Some("Twin")).unwrap_err();
    assert!(matches!(
        &err,
        ListItemsError::AmbiguousFolder { name, paths } if name == "Twin" && paths == &["a", "b"]
    ));
    assert_eq!(err.to_string(), "\"Twin\" matches several folders: a, b");
}

#[test]
fn a_request_file_name_never_resolves_as_a_folder() {
    let (_dir, c) = renamed();
    assert!(matches!(
        c.list_items(Some("Blog Posts/p")),
        Err(ListItemsError::NoFolder(_))
    ));
}

const CONFLICT: &str =
    "info:\n  name: a\n<<<<<<< HEAD\n  type: http\n=======\n  type: ws\n>>>>>>> other\n";
const MSG: &str = "merge conflict markers at line 3; resolve them in git";

#[test]
fn a_file_that_fails_to_load_is_listed_as_broken_after_its_siblings() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("z.yml", &request("z", 1, &[])),
        ("bad2.yml", CONFLICT),
        ("bad1.yml", CONFLICT),
        ("sub/bad.yml", CONFLICT),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let list = c.list_items(None).unwrap();
    let paths: Vec<&str> = list.items().iter().map(|i| i.path()).collect();
    assert_eq!(paths, ["sub", "z.yml", "bad1.yml", "bad2.yml"]);
    let broken = &list.items()[2];
    assert_eq!(broken.kind(), ListedKind::Broken);
    assert_eq!(broken.name(), "bad1");
    assert_eq!(broken.error(), Some(MSG));
    assert!(broken.children().is_none());
    assert_eq!(list.items()[1].error(), None);
    let sub = list.items()[0].children().unwrap();
    assert_eq!(sub[0].path(), "sub/bad.yml");
    assert_eq!(sub[0].kind(), ListedKind::Broken);
}

#[test]
fn a_broken_folder_file_puts_its_message_on_the_folder_item() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", CONFLICT),
        ("f/get.yml", &request("get", 1, &[])),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let list = c.list_items(None).unwrap();
    assert_eq!(list.items().len(), 1);
    assert_eq!(list.items()[0].kind(), ListedKind::Folder);
    assert_eq!(list.items()[0].error(), Some(MSG));
    assert_eq!(list.items()[0].children().unwrap().len(), 1);
}

#[test]
fn an_opaque_file_is_listed_once_and_not_as_broken() {
    let grpc = "info:\n  name: Query\n  type: grpc\n  seq: 3\n";
    let dir = write_tree(&[("opencollection.yml", ROOT), ("query.yml", grpc)]);
    let c = Collection::open(dir.path()).expect("opens");
    let list = c.list_items(None).unwrap();
    assert_eq!(list.items().len(), 1);
    assert_eq!(list.items()[0].kind(), ListedKind::Other);
}

#[test]
fn an_ignored_file_is_not_listed_as_broken() {
    let root = format!("{ROOT}extensions:\n  bruno:\n    ignore:\n      - 'bad.yml'\n");
    let dir = write_tree(&[("opencollection.yml", &root), ("bad.yml", CONFLICT)]);
    let c = apinata_core::Collection::open_with(
        dir.path(),
        &apinata_core::OpenOptions::new().ignore(apinata_core::IgnorePolicy::BrunoApp),
    )
    .expect("opens");
    assert!(c.list_items(None).unwrap().items().is_empty());
}

#[test]
fn depth_one_lists_direct_children_and_counts_what_a_folder_holds() {
    let (_dir, c) = open();
    let page = c
        .list_items_page(None, &ListOptions::default().with_depth(1))
        .unwrap();
    let paths: Vec<&str> = page.items().iter().map(|i| i.path()).collect();
    assert_eq!(paths, ["users", "a.yml", "b.yml"]);
    let users = &page.items()[0];
    assert!(users.children().is_none());
    // users holds get.yml and the inner folder.
    assert_eq!(users.child_count(), Some(2));
    assert_eq!(page.items()[1].child_count(), None);
    assert_eq!((page.total(), page.has_more()), (3, false));
}

#[test]
fn depth_two_nests_one_more_level_and_counts_at_the_cut() {
    let (_dir, c) = open();
    let page = c
        .list_items_page(None, &ListOptions::default().with_depth(2))
        .unwrap();
    let users = &page.items()[0];
    assert_eq!(users.child_count(), None);
    let inner = &users.children().unwrap()[0];
    assert_eq!(inner.path(), "users/inner");
    assert!(inner.children().is_none());
    assert_eq!(inner.child_count(), Some(1));
}

#[test]
fn without_depth_a_page_nests_in_full_and_the_plain_listing_has_no_counts() {
    let (_dir, c) = open();
    let page = c.list_items_page(None, &ListOptions::default()).unwrap();
    assert!(page.items()[0].children().is_some());
    assert_eq!(page.items()[0].child_count(), None);
    let plain = c.list_items(None).unwrap();
    assert_eq!(plain.items()[0].child_count(), None);
}

#[test]
fn limit_and_offset_slice_the_direct_items_in_tree_order() {
    let (_dir, c) = open();
    let first = c
        .list_items_page(None, &ListOptions::default().with_limit(2))
        .unwrap();
    let paths: Vec<&str> = first.items().iter().map(|i| i.path()).collect();
    assert_eq!(paths, ["users", "a.yml"]);
    assert_eq!((first.total(), first.has_more()), (3, true));
    let rest = c
        .list_items_page(None, &ListOptions::default().with_limit(2).with_offset(2))
        .unwrap();
    assert_eq!(rest.items().len(), 1);
    assert_eq!(rest.items()[0].path(), "b.yml");
    assert!(!rest.has_more());
}

#[test]
fn an_offset_past_the_end_is_an_empty_page_not_an_error() {
    let (_dir, c) = open();
    let page = c
        .list_items_page(None, &ListOptions::default().with_offset(50))
        .unwrap();
    assert!(page.items().is_empty());
    assert_eq!((page.total(), page.has_more()), (3, false));
}

#[test]
fn a_zero_limit_or_depth_is_refused_and_a_large_limit_is_clamped() {
    let (_dir, c) = open();
    let zero = c.list_items_page(None, &ListOptions::default().with_limit(0));
    assert_eq!(
        zero.unwrap_err(),
        ListItemsError::Page(PageError::ZeroLimit)
    );
    let depth = c.list_items_page(None, &ListOptions::default().with_depth(0));
    assert_eq!(
        depth.unwrap_err(),
        ListItemsError::Page(PageError::ZeroDepth)
    );
    let big = c
        .list_items_page(None, &ListOptions::default().with_limit(100_000))
        .unwrap();
    assert_eq!(big.limit(), Some(MAX_PAGE_LIMIT));
    assert_eq!(MAX_PAGE_LIMIT, 500);
}

#[test]
fn paging_a_subfolder_resolves_the_path_like_list_items() {
    let (_dir, c) = open();
    let page = c
        .list_items_page(Some("users"), &ListOptions::default().with_depth(1))
        .unwrap();
    let paths: Vec<&str> = page.items().iter().map(|i| i.path()).collect();
    assert_eq!(paths, ["users/inner", "users/get.yml"]);
    assert_eq!(page.items()[0].child_count(), Some(1));
}
