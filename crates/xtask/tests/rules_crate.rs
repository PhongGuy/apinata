//! The server rules crate stays a library the app release never ships: unpublished, versioned
//! on its own, free of async, network and UI crates, and named by no release trigger or build.

use std::process::Command;

use serde_json::{Value, json};

/// Runs cargo at the workspace root without the network and returns its stdout.
fn cargo(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO"))
        .args(args)
        .args(["--locked", "--offline"])
        .current_dir(xtask::workspace_root())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "cargo {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// apinata-rules' normal dependencies on every target, one crate name per line.
fn rules_tree(extra: &[&str]) -> Vec<String> {
    let mut args = vec![
        "tree",
        "-p",
        "apinata-rules",
        "-e",
        "normal",
        "--target",
        "all",
        "--prefix",
        "none",
    ];
    args.extend_from_slice(extra);
    cargo(&args)
        .lines()
        .filter_map(|line| line.split_whitespace().next().map(str::to_owned))
        .collect()
}

/// The workspace package named `name`, from `cargo metadata`.
fn package(name: &str) -> Value {
    let meta: Value =
        serde_json::from_str(&cargo(&["metadata", "--format-version", "1", "--no-deps"])).unwrap();
    meta["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == name)
        .unwrap_or_else(|| panic!("{name} is not a workspace member"))
        .clone()
}

#[test]
fn the_rules_crate_links_only_the_reader_the_contract_and_yaml() {
    let mut direct = rules_tree(&["--depth", "1"]);
    direct.sort();
    direct.dedup();
    assert_eq!(
        direct,
        [
            "apinata-format",
            "apinata-rules",
            "potluck-kind",
            "yaml_serde"
        ]
    );
    let names = rules_tree(&[]);
    for banned in [
        "tokio",
        "reqwest",
        "hyper",
        "mio",
        "gpui",
        "gpui-kit",
        "rquickjs",
        "apinata-core",
    ] {
        assert!(!names.iter().any(|n| n == banned), "{banned} in {names:?}");
    }
}

#[test]
fn the_new_crates_are_unpublished_and_versioned_on_their_own() {
    for name in ["apinata-format", "apinata-rules"] {
        let pkg = package(name);
        assert_eq!(pkg["publish"], json!([]), "{name} must set publish = false");
        assert_eq!(pkg["version"], "0.1.0", "{name}");
    }
}

#[test]
fn potluck_kind_is_pinned_by_tag_and_semver() {
    let pkg = package("apinata-rules");
    let dep = pkg["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["name"] == "potluck-kind")
        .unwrap()
        .clone();
    assert_eq!(
        (dep["source"].as_str(), dep["req"].as_str()),
        (
            Some("git+https://github.com/PhongGuy/potluck?tag=potluck-kind-v0.1.0"),
            Some("^0.1")
        )
    );
    let lock = std::fs::read_to_string(xtask::workspace_root().join("Cargo.lock")).unwrap();
    assert!(
        lock.contains("source = \"git+https://github.com/PhongGuy/potluck?tag=potluck-kind-v0.1.0#e56d4285233ee8a03119d3bd812ad94300602a59\""),
        "Cargo.lock must pin the tag's commit"
    );
}

#[test]
fn no_release_trigger_or_build_names_the_rules_crate() {
    let text =
        std::fs::read_to_string(xtask::workspace_root().join(".github/workflows/release.yml"))
            .unwrap();
    let release: yaml_serde::Value = yaml_serde::from_str(&text).unwrap();
    let tags = release["on"]["push"]["tags"].as_sequence().unwrap();
    for pattern in tags {
        let prefix = pattern.as_str().unwrap().trim_end_matches('*');
        assert!(
            !"apinata-rules-v0.1.0".starts_with(prefix),
            "release.yml's tag pattern {pattern:?} matches a rules release"
        );
    }
    for script in ["scripts/release.sh", "scripts/package-linux.sh"] {
        let text = std::fs::read_to_string(xtask::workspace_root().join(script)).unwrap();
        let builds: Vec<&str> = text.lines().filter(|l| l.contains("cargo build")).collect();
        assert!(!builds.is_empty(), "{script} builds nothing");
        for line in builds {
            assert!(
                line.contains("-p apinata-cli -p apinata-app")
                    && !line.contains("--workspace")
                    && !line.contains("apinata-rules"),
                "{script}: {line}"
            );
        }
    }
}
