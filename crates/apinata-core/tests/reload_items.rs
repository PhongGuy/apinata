//! Reloading request and opaque item files: hash first, then clean, dirty and seq-only merges.

use crate::tree_support;

use apinata_core::{ChangeKind::*, ItemKind::*, SaveError};
use tree_support::{
    BROKEN, ROOT, changed_url, errors_for, ip, item, one, open, reload, request, write,
};

#[test]
fn own_writes_untouched_files_and_format_only_rewrites_produce_no_change() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request("a", 1))]);
    assert!(reload(&dir, &mut c, "a.yml").is_empty());
    let a = c.request_mut(&ip("a.yml")).expect("a");
    a.set_method("POST");
    a.save().expect("saves");
    assert!(reload(&dir, &mut c, "a.yml").is_empty());
    let reformatted = request("a", 1).replace("method: GET", "method: 'POST'");
    write(&dir, "a.yml", &format!("# a comment\n{reformatted}"));
    assert!(
        reload(&dir, &mut c, "a.yml").is_empty(),
        "same content, other format"
    );
}

#[test]
fn a_clean_request_is_replaced() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request("a", 1))]);
    write(&dir, "a.yml", &changed_url("a", 1));
    assert_eq!(
        reload(&dir, &mut c, "a.yml"),
        one("a.yml", Modified, Request)
    );
    assert_eq!(
        c.request(&ip("a.yml")).expect("a").url(),
        "https://x.test/changed"
    );
}

#[test]
fn a_dirty_request_keeps_its_draft_and_its_next_save_conflicts() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request("a", 1))]);
    c.request_mut(&ip("a.yml")).expect("a").set_method("POST");
    write(&dir, "a.yml", &changed_url("a", 1));
    assert_eq!(
        reload(&dir, &mut c, "a.yml"),
        one("a.yml", Conflict, Request)
    );
    let a = c.request_mut(&ip("a.yml")).expect("a");
    assert_eq!((a.method(), a.url()), ("POST", "https://x.test/a"));
    assert!(matches!(a.save(), Err(SaveError::Conflict { .. })));
}

#[test]
fn a_seq_only_change_reaches_the_draft() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request("a", 1))]);
    c.request_mut(&ip("a.yml")).expect("a").set_method("POST");
    write(&dir, "a.yml", &request("a", 5));
    assert_eq!(
        reload(&dir, &mut c, "a.yml"),
        one("a.yml", SeqOnly, Request)
    );
    let a = c.request_mut(&ip("a.yml")).expect("a");
    assert_eq!((a.seq(), a.method(), a.is_dirty()), (Some(5), "POST", true));
    a.save().expect("no conflict after a seq-only reload");
    let disk = std::fs::read_to_string(dir.path().join("a.yml")).expect("read");
    assert!(
        disk.contains("seq: 5") && disk.contains("method: POST"),
        "{disk}"
    );
}

#[test]
fn a_seq_only_change_keeps_an_unsaved_seq_edit() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request("a", 1))]);
    c.request_mut(&ip("a.yml")).expect("a").set_seq(5);
    write(&dir, "a.yml", &request("a", 2));
    assert_eq!(
        reload(&dir, &mut c, "a.yml"),
        one("a.yml", SeqOnly, Request)
    );
    let a = c.request_mut(&ip("a.yml")).expect("a");
    assert_eq!((a.seq(), a.is_dirty()), (Some(5), true));
}

#[test]
fn files_are_added_removed_and_moved_through_load_errors() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    write(&dir, "b.yml", &request("b", 1));
    assert_eq!(reload(&dir, &mut c, "b.yml"), one("b.yml", Added, Request));
    write(&dir, "b.yml", BROKEN);
    assert_eq!(
        reload(&dir, &mut c, "b.yml"),
        one("b.yml", Removed, Request)
    );
    assert!(c.request(&ip("b.yml")).is_none());
    assert_eq!(errors_for(&dir, &c, "b.yml"), 1);
    write(&dir, "b.yml", &request("b", 1));
    assert_eq!(reload(&dir, &mut c, "b.yml"), one("b.yml", Added, Request));
    assert!(c.load_errors().is_empty());
    std::fs::remove_file(dir.path().join("b.yml")).expect("remove");
    assert_eq!(
        reload(&dir, &mut c, "b.yml"),
        one("b.yml", Removed, Request)
    );
    assert!(reload(&dir, &mut c, "b.yml").is_empty());
}

#[test]
fn a_known_broken_file_reloaded_unchanged_keeps_one_load_error() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("bad.yml", BROKEN)]);
    assert_eq!(errors_for(&dir, &c, "bad.yml"), 1);
    assert!(reload(&dir, &mut c, "bad.yml").is_empty());
    assert_eq!(errors_for(&dir, &c, "bad.yml"), 1);
}

#[test]
fn opaque_items_reload_and_stay_load_errors() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    write(&dir, "q.yml", &item("grpc", "q", 2));
    assert_eq!(reload(&dir, &mut c, "q.yml"), one("q.yml", Added, Opaque));
    assert_eq!(
        errors_for(&dir, &c, "q.yml"),
        1,
        "an opaque item is also a load error"
    );
    assert!(
        reload(&dir, &mut c, "q.yml").is_empty(),
        "same bytes: nothing, not even the error"
    );
    assert_eq!(errors_for(&dir, &c, "q.yml"), 1);
    write(&dir, "q.yml", &item("grpc", "renamed", 2));
    assert_eq!(
        reload(&dir, &mut c, "q.yml"),
        one("q.yml", Modified, Opaque)
    );
    assert_eq!(c.opaque_item(&ip("q.yml")).expect("q").name(), "renamed");
    assert_eq!(errors_for(&dir, &c, "q.yml"), 1);
}

#[test]
fn a_dirty_request_survives_its_file_breaking_until_it_parses_again() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request("a", 1))]);
    c.request_mut(&ip("a.yml")).expect("a").set_method("POST");
    write(&dir, "a.yml", "");
    assert_eq!(
        reload(&dir, &mut c, "a.yml"),
        one("a.yml", Conflict, Request)
    );
    assert_eq!(errors_for(&dir, &c, "a.yml"), 1);
    let a = c.request_mut(&ip("a.yml")).expect("draft kept");
    assert_eq!((a.method(), a.is_dirty()), ("POST", true));
    assert!(matches!(a.save(), Err(SaveError::Conflict { .. })));

    // The loaded bytes come back: the draft is restored, the error clears and the draft saves.
    write(&dir, "a.yml", &request("a", 1));
    assert_eq!(
        reload(&dir, &mut c, "a.yml"),
        one("a.yml", Restored, Request)
    );
    assert_eq!(errors_for(&dir, &c, "a.yml"), 0);
    let a = c.request_mut(&ip("a.yml")).expect("a");
    a.save().expect("saves over the restored file");

    // Other valid content after a break conflicts and clears the error too.
    c.request_mut(&ip("a.yml")).expect("a").set_method("PUT");
    write(&dir, "a.yml", BROKEN);
    assert_eq!(
        reload(&dir, &mut c, "a.yml"),
        one("a.yml", Conflict, Request)
    );
    write(&dir, "a.yml", &changed_url("a", 1));
    assert_eq!(
        reload(&dir, &mut c, "a.yml"),
        one("a.yml", Conflict, Request)
    );
    assert_eq!(errors_for(&dir, &c, "a.yml"), 0);
    assert_eq!(c.request(&ip("a.yml")).expect("a").method(), "PUT");

    // A removal keeps the draft too, as a conflict, and clears the error.
    std::fs::remove_file(dir.path().join("a.yml")).expect("rm");
    assert_eq!(
        reload(&dir, &mut c, "a.yml"),
        one("a.yml", Conflict, Request)
    );
    assert_eq!(errors_for(&dir, &c, "a.yml"), 0);
    assert_eq!(c.request(&ip("a.yml")).expect("kept").method(), "PUT");
}

#[test]
fn a_graphql_file_reloads_as_a_request() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    write(&dir, "q.yml", &item("graphql", "q", 2));
    assert_eq!(reload(&dir, &mut c, "q.yml"), one("q.yml", Added, Request));
    assert_eq!(errors_for(&dir, &c, "q.yml"), 0);
    assert_eq!(
        c.request(&ip("q.yml")).expect("q").kind(),
        apinata_core::RequestKind::Graphql
    );
}

#[test]
fn a_graphql_file_rewritten_as_http_reloads_with_the_new_kind() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("q.yml", &item("graphql", "q", 2)),
    ]);
    write(&dir, "q.yml", &request("q", 2));
    assert_eq!(
        reload(&dir, &mut c, "q.yml"),
        one("q.yml", Modified, Request)
    );
    assert_eq!(
        c.request(&ip("q.yml")).expect("q").kind(),
        apinata_core::RequestKind::Http
    );
}
