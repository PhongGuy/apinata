//! Platform trash folders inside a collection are never loaded, run or watched.

use crate::tree_support;

use apinata_core::{IgnorePolicy, OpenOptions};
use tree_support::{ROOT, ip, open, reload, request, write};

/// Where each platform's trash puts a deleted request when the collection is a volume root.
const TRASHED: [&str; 6] = [
    ".Trash-1000/files/Users/gone.yml",
    ".Trash/1000/files/gone.yml",
    ".Trashes/501/gone.yml",
    "$RECYCLE.BIN/S-1-5-21/gone.yml",
    "sub/$Recycle.Bin/S-1-5-21/gone.yml",
    "sub/.Trash-0/files/gone.yml",
];

fn open_as(policy: IgnorePolicy) -> (tempfile::TempDir, apinata_core::Collection) {
    let mut files = vec![
        ("opencollection.yml", ROOT.to_owned()),
        ("kept.yml", request("kept", 1)),
        ("sub/also.yml", request("also", 1)),
        (
            ".Trash-1000/info/gone.yml.trashinfo",
            "[Trash Info]\n".to_owned(),
        ),
        (".Trashed/real.yml", request("real", 1)),
    ];
    files.extend(TRASHED.iter().map(|p| (*p, request("gone", 2))));
    let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
    let (dir, _) = open(&files);
    let c = apinata_core::Collection::open_with(dir.path(), &OpenOptions::new().ignore(policy))
        .unwrap();
    (dir, c)
}

#[test]
fn trash_folders_are_not_loaded_or_run_under_either_policy() {
    for policy in [IgnorePolicy::BrunoApp, IgnorePolicy::BrunoCli] {
        let (dir, c) = open_as(policy);
        for gone in TRASHED {
            assert!(c.request(&ip(gone)).is_none(), "{gone} under {policy:?}");
            assert!(
                c.is_ignored(&dir.path().join(gone)),
                "{gone} under {policy:?}"
            );
        }
        for folder in [
            ".Trash-1000",
            ".Trash",
            ".Trashes",
            "$RECYCLE.BIN",
            "sub/.Trash-0",
        ] {
            assert!(c.folder(&ip(folder)).is_none(), "{folder} under {policy:?}");
        }
        for kept in ["kept.yml", "sub/also.yml", ".Trashed/real.yml"] {
            assert!(c.request(&ip(kept)).is_some(), "{kept} under {policy:?}");
        }
        assert!(c.load_errors().is_empty(), "{:?}", c.load_errors());
        let plan = c.plan(dir.path()).expect("root");
        let mut names: Vec<&str> = plan.requests().map(|r| r.name()).collect();
        names.sort();
        assert_eq!(names, ["also", "kept", "real"], "{policy:?}");
        let watched: Vec<String> = c
            .watch_dirs()
            .iter()
            .map(|d| {
                d.strip_prefix(dir.path())
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        assert_eq!(
            watched,
            ["", ".Trashed", "environments", "sub"],
            "{policy:?}"
        );
    }
}

#[test]
fn a_request_trashed_after_open_does_not_come_back_on_reload() {
    let (dir, mut c) = open_as(IgnorePolicy::BrunoApp);
    write(&dir, ".Trash-1000/files/kept.yml", &request("kept", 1));
    std::fs::remove_file(dir.path().join("kept.yml")).unwrap();
    for rel in [
        ".Trash-1000",
        ".Trash-1000/files",
        ".Trash-1000/files/kept.yml",
    ] {
        assert!(reload(&dir, &mut c, rel).is_empty(), "{rel}");
    }
    reload(&dir, &mut c, "kept.yml");
    c.reload();
    assert!(c.request(&ip(".Trash-1000/files/kept.yml")).is_none());
    assert!(c.folder(&ip(".Trash-1000")).is_none());
}
