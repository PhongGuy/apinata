use crate::support;

use apinata_app::snapshot::{Content, SnapshotCache};
use apinata_app::watch::rules::{WatchMsg, translate};
use apinata_core::{Collection, IgnorePolicy, OpenOptions};
use notify_debouncer_full::DebouncedEvent;
use notify_debouncer_full::notify::event::{
    AccessKind, CreateKind, Flag, ModifyKind, RemoveKind, RenameMode,
};
use notify_debouncer_full::notify::{Error, ErrorKind, Event, EventKind};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Instant;
use support::{ROOT, folder, ip, request, write_tree};

fn ev(kind: EventKind, paths: &[&str]) -> DebouncedEvent {
    let event = paths
        .iter()
        .fold(Event::new(kind), |e, p| e.add_path(PathBuf::from(p)));
    DebouncedEvent::new(event, Instant::now())
}

fn tr(events: Vec<DebouncedEvent>) -> Option<WatchMsg> {
    translate(Ok(events), Path::new("/r"), Path::new("/private/r"))
}

fn paths(p: &[&str]) -> Option<WatchMsg> {
    Some(WatchMsg::Paths(p.iter().map(PathBuf::from).collect()))
}

#[test]
fn paths_skip_access_events_and_come_sorted_once() {
    let got = tr(vec![
        ev(EventKind::Remove(RemoveKind::Any), &["/r/b.yml"]),
        ev(EventKind::Create(CreateKind::File), &["/r/b.yml"]),
        ev(EventKind::Access(AccessKind::Any), &["/r/z.yml"]),
        ev(
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            &["/r/old", "/r/a"],
        ),
    ]);
    assert_eq!(got, paths(&["/r/a", "/r/b.yml", "/r/old"]));
}

#[test]
fn canonical_paths_map_back_under_the_root_and_outside_paths_drop() {
    let got = tr(vec![ev(
        EventKind::Create(CreateKind::File),
        &["/private/r/x.yml", "/elsewhere/y.yml"],
    )]);
    assert_eq!(got, paths(&["/r/x.yml"]));
}

#[test]
fn a_rescan_flag_asks_for_a_full_reload() {
    let flagged = Event::new(EventKind::Other).set_flag(Flag::Rescan);
    let got = tr(vec![
        ev(EventKind::Create(CreateKind::File), &["/r/a.yml"]),
        DebouncedEvent::new(flagged, Instant::now()),
    ]);
    assert_eq!(got, Some(WatchMsg::Rescan));
}

#[test]
fn the_watch_limit_in_the_stream_is_a_failure_and_other_errors_are_not() {
    let root = Path::new("/r");
    let limit = translate(Err(vec![Error::new(ErrorKind::MaxFilesWatch)]), root, root);
    assert_eq!(
        limit,
        Some(WatchMsg::Failed("OS file watch limit reached.".into()))
    );
    assert_eq!(
        translate(Err(vec![Error::new(ErrorKind::PathNotFound)]), root, root),
        None
    );
}

#[test]
fn a_batch_of_access_events_is_no_message() {
    assert_eq!(
        tr(vec![ev(EventKind::Access(AccessKind::Any), &["/r/a.yml"])]),
        None
    );
    assert_eq!(tr(vec![]), None);
}

fn content_of(c: &Collection) -> Content {
    SnapshotCache::new().build(c)
}

fn dirs(c: &Collection) -> BTreeSet<PathBuf> {
    content_of(c).dirs.iter().cloned().collect()
}

#[test]
fn watch_dirs_are_the_root_its_folders_and_environments() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
        ("users/folder.yml", &folder("Users")),
        ("users/deep/d.yml", &request("d", "http://x/0")),
        (".git/HEAD", "ref: refs/heads/main\n"),
        ("node_modules/x/package.json", "{}"),
    ]);
    let root = dir.path();
    let c = Collection::open(root).unwrap();
    let expected: BTreeSet<PathBuf> = [
        root.to_path_buf(),
        root.join("environments"),
        root.join("users"),
        root.join("users/deep"),
    ]
    .into();
    assert_eq!(dirs(&c), expected);
    assert_eq!(
        c.watch_dirs(),
        expected.into_iter().collect::<Vec<_>>(),
        "sorted, and the snapshot copies it"
    );
}

#[test]
fn ignored_folders_and_an_ignored_environments_directory_get_no_watch() {
    let root_file =
        format!("{ROOT}extensions:\n  bruno:\n    ignore:\n      - skip\n      - environments\n");
    let dir = write_tree(&[
        ("opencollection.yml", &root_file),
        ("skip/a.yml", &request("a", "http://x/0")),
        ("mocks/m.yml", &request("m", "http://x/0")),
        ("kept/b.yml", &request("b", "http://x/0")),
    ]);
    let root = dir.path();
    let c =
        Collection::open_with(root, &OpenOptions::new().ignore(IgnorePolicy::BrunoApp)).unwrap();
    assert_eq!(
        dirs(&c),
        BTreeSet::from([root.to_path_buf(), root.join("kept")])
    );
}

#[gpui_kit::test]
fn the_actor_opens_with_the_apps_ignore_rule(cx: &mut gpui_kit::TestAppContext) {
    let root_file = format!("{ROOT}extensions:\n  bruno:\n    ignore:\n      - skip\n");
    let dir = write_tree(&[
        ("opencollection.yml", &root_file),
        ("skip/a.yml", &request("a", "http://x/0")),
    ]);
    let model = support::open(cx, &dir);
    assert!(
        !support::snap(cx, &model)
            .content
            .requests
            .contains_key(&ip("skip/a.yml"))
    );
}

#[test]
fn a_folder_kept_only_for_drafts_gets_no_watch() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/u.yml", &request("u", "http://x/0")),
    ]);
    let root = dir.path();
    let mut c = Collection::open(root).unwrap();
    c.request_mut(&ip("users/u.yml"))
        .unwrap()
        .set_url("http://x/draft");
    std::fs::remove_dir_all(root.join("users")).unwrap();
    c.reload_path(&root.join("users"));
    let content = content_of(&c);
    assert!(
        content.rows.iter().any(|r| r.path == ip("users")),
        "the folder is kept for its draft"
    );
    assert!(!content.dirs.contains(&root.join("users")));
}
