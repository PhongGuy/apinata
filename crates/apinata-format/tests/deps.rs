//! Pins what the reader kernel links: the server's rules depend on it, so it must stay free of
//! async, network and UI crates.

use std::path::Path;
use std::process::Command;

/// `cargo tree` for apinata-format's normal edges on every target, one crate name per line.
fn tree(extra: &[&str]) -> Vec<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = Command::new(env!("CARGO"))
        .args([
            "tree",
            "-p",
            "apinata-format",
            "-e",
            "normal",
            "--target",
            "all",
        ])
        .args(["--prefix", "none", "--locked"])
        .args(extra)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .filter_map(|line| line.split_whitespace().next().map(str::to_owned))
        .collect()
}

#[test]
fn the_direct_dependencies_are_the_two_yaml_crates() {
    let mut direct = tree(&["--depth", "1"]);
    direct.sort();
    direct.dedup();
    assert_eq!(direct, ["apinata-format", "saphyr-parser", "yaml_serde"]);
}

#[test]
fn no_async_network_or_ui_crate_is_linked() {
    let names = tree(&[]);
    for banned in [
        "tokio", "reqwest", "hyper", "mio", "gpui", "gpui-kit", "rquickjs",
    ] {
        assert!(!names.iter().any(|n| n == banned), "{banned} in {names:?}");
    }
    assert!(
        !names
            .iter()
            .any(|n| n.starts_with("apinata-") && n != "apinata-format"),
        "{names:?}"
    );
}
