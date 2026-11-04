//! Bruno's ignore rule at open time, under each policy.

use crate::tree_support;

use apinata_core::{IgnorePolicy, OpenOptions};
use tree_support::{ROOT, ip, open, request};

fn open_as(
    policy: IgnorePolicy,
    files: &[(&str, &str)],
) -> (tempfile::TempDir, apinata_core::Collection) {
    let (dir, _) = open(files);
    let c = apinata_core::Collection::open_with(dir.path(), &OpenOptions::new().ignore(policy))
        .unwrap();
    (dir, c)
}

fn root_with(entries: &[&str]) -> String {
    let list: String = entries.iter().map(|e| format!("      - '{e}'\n")).collect();
    format!("{ROOT}extensions:\n  bruno:\n    ignore:\n{list}")
}

fn env(name: &str) -> String {
    format!("name: {name}\nvariables:\n  - name: host\n    value: x\n")
}

#[test]
fn listed_paths_root_mocks_and_vcs_directories_are_not_loaded() {
    let (_dir, c) = open_as(
        IgnorePolicy::BrunoApp,
        &[
            (
                "opencollection.yml",
                &root_with(&["skip", "one.yml", "deep/inner", "environments/old.yml"]),
            ),
            ("skip/a.yml", &request("a", 1)),
            ("skipper/b.yml", &request("b", 1)),
            ("one.yml", &request("one", 1)),
            ("two.yml", &request("two", 2)),
            ("deep/inner/c.yml", &request("c", 1)),
            ("deep/d.yml", &request("d", 1)),
            ("mocks/m.yml", &request("m", 1)),
            ("sub/mocks/n.yml", &request("n", 1)),
            ("environments/dev.yml", &env("dev")),
            ("environments/old.yml", &env("old")),
        ],
    );
    for gone in ["skip/a.yml", "one.yml", "deep/inner/c.yml", "mocks/m.yml"] {
        assert!(c.request(&ip(gone)).is_none(), "{gone} is ignored");
    }
    for kept in ["skipper/b.yml", "two.yml", "deep/d.yml", "sub/mocks/n.yml"] {
        assert!(c.request(&ip(kept)).is_some(), "{kept} is loaded");
    }
    assert!(c.folder(&ip("skip")).is_none() && c.folder(&ip("mocks")).is_none());
    assert!(c.environment("dev").is_some() && c.environment("old").is_none());
    assert!(c.load_errors().is_empty());
}

#[test]
fn the_cli_and_app_policies_differ_as_bruno_cli_and_the_app_do() {
    let files = [
        ("opencollection.yml", root_with(&["skip"])),
        ("skip/a.yml", request("a", 1)),
        ("mocks/m.yml", request("m", 1)),
        (
            "x_node_modules_y/folder.yml",
            "info:\n  name: X\n  type: folder\n  seq: 1\n".to_string(),
        ),
        ("x_node_modules_y/b.yml", request("b", 1)),
        ("x_node_modules_y/inner/c.yml", request("c", 1)),
        ("node_modules_notes.yml", request("notes", 1)),
        ("node_modules/n.yml", request("n", 1)),
        (".git/g.yml", request("g", 1)),
        ("plain/c.yml", request("c", 1)),
    ];
    let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
    let loaded = |c: &apinata_core::Collection| -> Vec<&str> {
        [
            "skip/a.yml",
            "mocks/m.yml",
            "x_node_modules_y/b.yml",
            "x_node_modules_y/inner/c.yml",
            "node_modules_notes.yml",
            "node_modules/n.yml",
            ".git/g.yml",
            "plain/c.yml",
        ]
        .into_iter()
        .filter(|r| c.request(&ip(r)).is_some())
        .collect()
    };
    let (_d1, cli) = open_as(IgnorePolicy::BrunoCli, &files);
    let (_d2, app) = open_as(IgnorePolicy::BrunoApp, &files);
    let (_d3, default) = open(&files);
    assert_eq!(
        loaded(&cli),
        [
            "skip/a.yml",
            "mocks/m.yml",
            "node_modules_notes.yml",
            "plain/c.yml"
        ]
    );
    let x = cli
        .folder(&ip("x_node_modules_y"))
        .expect("bruno-cli keeps the folder, not its contents");
    assert_eq!(x.seq(), Some(1), "its folder.yml is read");
    assert!(
        cli.folder(&ip("x_node_modules_y/inner")).is_none(),
        "nor its subfolders"
    );
    assert_eq!(
        loaded(&app),
        [
            "x_node_modules_y/b.yml",
            "x_node_modules_y/inner/c.yml",
            "node_modules_notes.yml",
            "plain/c.yml"
        ]
    );
    assert_eq!(loaded(&default), loaded(&cli), "open is the CLI policy");
}

#[test]
fn a_collection_under_a_node_modules_directory_still_loads_under_the_cli_policy() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("node_modules/coll");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("opencollection.yml"), ROOT).unwrap();
    std::fs::write(root.join("sub/a.yml"), request("a", 1)).unwrap();
    let c = apinata_core::Collection::open(&root).unwrap();
    assert!(c.request(&ip("sub/a.yml")).is_some());
}
