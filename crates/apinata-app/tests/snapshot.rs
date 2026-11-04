use crate::support;

use apinata_app::command::{Edit, Field, Target};
use apinata_app::snapshot::{RowKind, SnapshotCache};
use apinata_core::Collection;
use std::sync::Arc;
use support::{ROOT, folder, ip, request, write_tree};

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("health.yml", &request("Health", "http://x/health")),
        ("users/folder.yml", &folder("Users")),
        ("users/list.yml", &request("List users", "http://x/users")),
        (
            "users/admin/create.yml",
            &request("Create admin", "http://x/admin"),
        ),
        ("broken.yml", "info: [\n"),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: host\n    value: x\n",
        ),
    ])
}

#[test]
fn rows_list_folders_before_requests_depth_first() {
    let dir = tree();
    let c = Collection::open(dir.path()).unwrap();
    let content = SnapshotCache::new().build(&c);
    let rows: Vec<(&str, RowKind, usize)> = content
        .rows
        .iter()
        .map(|r| (r.path.as_str(), r.kind, r.depth))
        .collect();
    assert_eq!(
        rows,
        [
            ("users", RowKind::Folder, 0),
            ("users/admin", RowKind::Folder, 1),
            ("users/admin/create.yml", RowKind::Request, 2),
            ("users/list.yml", RowKind::Request, 1),
            ("health.yml", RowKind::Request, 0),
        ]
    );
    assert_eq!(content.rows[0].name, "Users");
    assert_eq!(content.rows[1].name, "admin");
}

#[test]
fn views_carry_the_collection_state() {
    let dir = tree();
    let c = Collection::open(dir.path()).unwrap();
    let content = SnapshotCache::new().build(&c);
    assert_eq!(
        content.requests[&ip("users/list.yml")].url,
        "http://x/users"
    );
    assert_eq!(content.requests.len(), 3);
    let mut folders: Vec<&str> = content.folders.keys().map(|k| k.as_str()).collect();
    folders.sort();
    assert_eq!(folders, ["users", "users/admin"]);
    assert_eq!(content.folders[&ip("users")].name.as_deref(), Some("Users"));
    assert_eq!(content.collection.name.as_deref(), Some("c"));
    assert_eq!(content.environments.len(), 1);
    assert_eq!(content.environments[0].name, "dev");
    assert_eq!(content.load_errors.len(), 1);
    assert_eq!(
        content.load_errors[0].path,
        std::path::Path::new("broken.yml")
    );
    assert!(!content.load_errors[0].message.is_empty());
}

#[test]
fn untouched_views_are_shared_between_builds() {
    let dir = tree();
    let mut c = Collection::open(dir.path()).unwrap();
    let mut cache = SnapshotCache::new();
    let first = cache.build(&c);
    c.request_mut(&ip("health.yml"))
        .unwrap()
        .set_url("http://x/changed");
    cache.touch(&Target::Request(ip("health.yml")));
    let second = cache.build(&c);
    let health = ip("health.yml");
    assert_eq!(second.requests[&health].url, "http://x/changed");
    assert!(second.requests[&health].dirty);
    assert!(second.rows.iter().find(|r| r.path == health).unwrap().dirty);
    assert!(!Arc::ptr_eq(
        &first.requests[&health],
        &second.requests[&health]
    ));
    let list = ip("users/list.yml");
    assert!(Arc::ptr_eq(&first.requests[&list], &second.requests[&list]));
    assert!(Arc::ptr_eq(
        &first.folders[&ip("users")],
        &second.folders[&ip("users")]
    ));
    assert!(Arc::ptr_eq(&first.collection, &second.collection));
    assert!(Arc::ptr_eq(&first.environments[0], &second.environments[0]));
}

#[test]
fn touch_all_rebuilds_every_view() {
    let dir = tree();
    let c = Collection::open(dir.path()).unwrap();
    let mut cache = SnapshotCache::new();
    let first = cache.build(&c);
    cache.touch_all();
    let second = cache.build(&c);
    let list = ip("users/list.yml");
    assert!(!Arc::ptr_eq(
        &first.requests[&list],
        &second.requests[&list]
    ));
    assert!(!Arc::ptr_eq(&first.collection, &second.collection));
}

#[test]
fn url_and_params_are_one_field() {
    assert_eq!(Edit::Url("http://x".into()).field(), Field::UrlParams);
    assert_eq!(Edit::Params(Vec::new()).field(), Field::UrlParams);
    assert_eq!(Edit::Method("POST".into()).field(), Field::Method);
    assert_eq!(Edit::Docs(String::new()).field(), Field::Docs);
}
