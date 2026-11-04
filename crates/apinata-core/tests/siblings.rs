//! Item paths, lookups by path, and the sibling set with opaque items.

use crate::common;
use crate::parity;

use apinata_core::{Collection, ItemPath, Sibling};
use common::write_tree;
use std::fs;
use std::path::Path;

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: c\n";
const GRPC: &str = "info:\n  name: gql\n  type: grpc\n  seq: 2\ngrpc:\n  url: \"{{host}}/rpc\"\n";

fn request(name: &str, seq: u64) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: https://x.test/{name}\n"
    )
}

fn open(files: &[(&str, &str)]) -> (tempfile::TempDir, Collection) {
    let dir = write_tree(files);
    let c = Collection::open(dir.path()).expect("opens");
    (dir, c)
}

fn path(rel: &str) -> ItemPath {
    ItemPath::new(rel).expect("valid item path")
}

#[test]
fn item_paths_are_relative_slash_separated_segments() {
    let p = path("users/get-user.yml");
    assert_eq!(p.as_str(), "users/get-user.yml");
    assert_eq!(p.to_string(), "users/get-user.yml");
    assert_eq!(p.file_name(), "get-user.yml");
    assert_eq!(p.parent(), Some(path("users")));
    assert_eq!(path("users").parent(), Some(ItemPath::root()));
    assert_eq!(ItemPath::root().parent(), None);
    assert!(ItemPath::root().is_root());
    assert_eq!(ItemPath::new(""), Some(ItemPath::root()));
    assert_eq!(path("users").join("get-user.yml"), Some(p.clone()));
    assert_eq!(ItemPath::root().join("a.yml"), Some(path("a.yml")));
    assert_eq!(
        p.to_path(Path::new("/c")),
        Path::new("/c/users/get-user.yml")
    );
    assert_eq!(ItemPath::root().to_path(Path::new("/c")), Path::new("/c"));
}

#[test]
fn item_paths_reject_escapes_and_empty_segments() {
    for bad in ["/abs", "a//b", "a/", "./a", "a/../b", "..", "."] {
        assert_eq!(ItemPath::new(bad), None, "{bad}");
    }
    for bad in ["", "a/b", "..", "."] {
        assert_eq!(ItemPath::root().join(bad), None, "{bad}");
    }
}

#[test]
fn a_collection_maps_disk_paths_to_item_paths() {
    let (dir, c) = open(&[
        ("opencollection.yml", ROOT),
        ("users/get.yml", &request("get", 1)),
    ]);
    assert_eq!(
        c.item_path(&dir.path().join("users/get.yml")),
        Some(path("users/get.yml"))
    );
    assert_eq!(c.item_path(dir.path()), Some(ItemPath::root()));
    assert_eq!(c.item_path(Path::new("/elsewhere/get.yml")), None);
    assert_eq!(c.item_path(&dir.path().join("users/../get.yml")), None);
}

#[test]
fn items_are_found_by_item_path() {
    let (dir, c) = open(&[
        ("opencollection.yml", ROOT),
        ("top.yml", &request("top", 1)),
        ("users/get.yml", &request("get", 1)),
        ("users/admin/list.yml", &request("list", 1)),
        ("users/query.yml", GRPC),
    ]);
    assert_eq!(
        c.folder(&ItemPath::root()).expect("root").path(),
        c.tree().path()
    );
    assert_eq!(
        c.folder(&path("users/admin")).expect("folder").dir_name(),
        "admin"
    );
    assert_eq!(c.request(&path("top.yml")).expect("top").name(), "top");
    assert_eq!(
        c.request(&path("users/admin/list.yml"))
            .expect("list")
            .path(),
        dir.path().join("users/admin/list.yml")
    );
    assert_eq!(
        c.opaque_item(&path("users/query.yml"))
            .expect("opaque")
            .kind(),
        "grpc"
    );
    assert!(c.request(&path("users/query.yml")).is_none());
    assert!(c.request(&path("users")).is_none());
    assert!(c.request(&ItemPath::root()).is_none());
    assert!(c.folder(&path("users/get.yml")).is_none());
    assert!(c.request(&path("nope/get.yml")).is_none());
}

#[test]
fn a_request_found_by_path_can_be_edited_and_saved() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("users/get.yml", &request("get", 1)),
    ]);
    let r = c.request_mut(&path("users/get.yml")).expect("request");
    r.set_method("POST");
    r.save().expect("saves");
    let text = fs::read_to_string(dir.path().join("users/get.yml")).expect("read");
    assert!(text.contains("method: POST"), "{text}");
}

#[test]
fn opaque_items_join_the_siblings_and_stay_load_errors() {
    let (dir, c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("query.yml", GRPC),
        ("stream.yml", "info:\n  name: ws\n  type: websocket\n"),
        ("broken.yml", "info: [unclosed\n"),
        ("no-type.yml", "info:\n  name: x\n"),
        ("http-without-block.yml", "info:\n  name: h\n  type: http\n"),
        ("sub/folder.yml", "info:\n  name: Sub\n  seq: 3\n"),
    ]);
    let root = c.tree();
    let opaque: Vec<&str> = root.opaque_items().iter().map(|o| o.file_name()).collect();
    assert_eq!(opaque, ["query.yml", "stream.yml"]);
    assert_eq!(root.requests().len(), 1);
    // Every file RequestFile rejects is still reported; an unknown type is named in the message.
    let mut failed: Vec<String> = c
        .load_errors()
        .iter()
        .map(|e| {
            e.path()
                .strip_prefix(dir.path())
                .expect("under root")
                .display()
                .to_string()
        })
        .collect();
    failed.sort();
    assert_eq!(
        failed,
        [
            "broken.yml",
            "http-without-block.yml",
            "no-type.yml",
            "query.yml",
            "stream.yml"
        ]
    );
    let grpc = c
        .load_errors()
        .iter()
        .find(|e| e.path().ends_with("query.yml"))
        .expect("reported");
    assert_eq!(
        grpc.message(),
        "unsupported request type `grpc`, only `http` and `graphql` are supported"
    );
}

#[test]
fn siblings_list_folders_then_requests_and_opaque_items_in_one_seq_order() {
    let (_dir, c) = open(&[
        ("opencollection.yml", ROOT),
        ("b.yml", &request("b", 3)),
        ("a.yml", &request("a", 1)),
        ("query.yml", GRPC),
        ("z.yml", "info:\n  name: z\n  type: grpc\ngrpc:\n  url: x\n"),
        ("sub/folder.yml", "info:\n  name: Sub\n  seq: 3\n"),
    ]);
    let siblings = c.tree().siblings();
    let summary: Vec<(&str, &str, Option<u64>)> = siblings
        .iter()
        .map(|s| {
            let kind = match s {
                Sibling::Folder(_) => "folder",
                Sibling::Request(_) => "request",
                Sibling::Opaque(_) => "opaque",
                _ => "other",
            };
            (kind, s.name(), s.seq())
        })
        .collect();
    // An opaque item takes its seq's place among the requests; a seq-less one goes last.
    assert_eq!(
        summary,
        [
            ("folder", "Sub", Some(3)),
            ("request", "a", Some(1)),
            ("opaque", "gql", Some(2)),
            ("request", "b", Some(3)),
            ("opaque", "z", None),
        ]
    );
    assert!(siblings[2].path().ends_with("query.yml"));
}

#[test]
fn an_untouched_opaque_item_is_never_rewritten() {
    let odd = "# keep me\ninfo: {name: gql, type: grpc, seq: 2}\ngrpc: {url: x}\n";
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("query.yml", odd)]);
    let item = c.opaque_item_mut(&path("query.yml")).expect("opaque");
    item.set_seq(2);
    assert!(!item.is_dirty());
    item.save().expect("saves");
    assert_eq!(
        fs::read_to_string(dir.path().join("query.yml")).expect("read"),
        odd
    );
}

#[test]
fn a_graphql_seq_edit_writes_the_reference_bytes() {
    let normalised = parity::normalized("synthetic/graphql.yml");
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("graphql.yml", &normalised)]);
    let item = c.request_mut(&path("graphql.yml")).expect("request");
    assert_eq!(item.seq(), Some(3));
    item.set_seq(7);
    assert!(item.is_dirty());
    item.save().expect("saves");
    assert_eq!(
        fs::read_to_string(dir.path().join("graphql.yml")).expect("read"),
        parity::expected("opaque-seq")
    );
}

#[test]
fn an_opaque_seq_edit_on_another_file_changes_only_the_seq() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("query.yml", GRPC)]);
    let item = c.opaque_item_mut(&path("query.yml")).expect("opaque");
    item.set_seq(5);
    item.save().expect("saves");
    let after: yaml_serde::Value =
        yaml_serde::from_str(&fs::read_to_string(dir.path().join("query.yml")).expect("read"))
            .expect("parses");
    let mut expected: yaml_serde::Value = yaml_serde::from_str(GRPC).expect("parses");
    expected["info"]["seq"] = yaml_serde::Value::from(5);
    assert_eq!(after, expected);
    assert_eq!(
        yaml_serde::to_string(&after).expect("dump"),
        yaml_serde::to_string(&expected).expect("dump")
    );
}
