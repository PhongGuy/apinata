//! The release manifest generator, and that core's updater accepts its output.

use apinata_core::update::{Format, Product, build_config_with, evaluate, target_id};
use std::process::Command;
use tempfile::TempDir;
use xtask::update_manifest::build;

const APPIMAGE: &str = "Apinata_1.2.0_x86_64.AppImage";
const CLI: &str = "apinata-1.2.0-x86_64-unknown-linux-gnu";

fn artifacts() -> TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join(APPIMAGE), b"appimage").unwrap();
    std::fs::write(d.path().join(CLI), b"cli").unwrap();
    d
}

fn map() -> String {
    format!("app-linux-x86_64 {APPIMAGE}\ncli-linux-x86_64 {CLI}\n")
}

#[test]
fn entries_carry_url_format_size_and_hash() {
    let d = artifacts();
    let json = build(
        "1.2.0",
        "https://dl.example.com/v1.2.0/",
        "Notes.\n",
        &map(),
        Some("2026-11-01T12:00:00Z"),
        d.path(),
    )
    .unwrap();
    assert!(json.ends_with('\n'));
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["version"], "1.2.0");
    assert_eq!(v["notes"], "Notes.");
    assert_eq!(v["pub_date"], "2026-11-01T12:00:00Z");
    let app = &v["targets"]["app-linux-x86_64"];
    assert_eq!(
        app["url"],
        format!("https://dl.example.com/v1.2.0/{APPIMAGE}")
    );
    assert_eq!(app["format"], "appimage");
    assert_eq!(app["size"], 8);
    assert_eq!(
        app["sha256"],
        "8b408ed68dfd56d503752ff2ee2ecb3c0ffa55a26f6fa107bd4444c3943ee6e1"
    );
    let cli = &v["targets"]["cli-linux-x86_64"];
    assert_eq!(
        (cli["format"].as_str(), cli["size"].as_u64()),
        (Some("binary"), Some(3))
    );
    assert_eq!(
        cli["sha256"],
        "99bb88401742848e032fd6f51709415fb6be169a72d2e5d7fc44289255160d3c"
    );
}

#[test]
fn empty_notes_and_no_date_are_omitted() {
    let d = artifacts();
    let v: serde_json::Value =
        serde_json::from_str(&build("1.2.0", "https://x", "  \n", &map(), None, d.path()).unwrap())
            .unwrap();
    assert!(
        v.get("notes").is_none() && v.get("pub_date").is_none(),
        "{v}"
    );
}

#[test]
fn formats_follow_the_target() {
    let d = artifacts();
    let m = format!(
        "app-macos-aarch64 {APPIMAGE}\napp-windows-x86_64 {CLI}\ncli-windows-aarch64 {APPIMAGE}\n"
    );
    let v: serde_json::Value =
        serde_json::from_str(&build("1.2.0", "https://x", "", &m, None, d.path()).unwrap())
            .unwrap();
    assert_eq!(v["targets"]["app-macos-aarch64"]["format"], "app");
    assert_eq!(v["targets"]["app-windows-x86_64"]["format"], "nsis");
    assert_eq!(v["targets"]["cli-windows-aarch64"]["format"], "binary");
}

#[test]
fn the_core_updater_accepts_the_output() {
    let d = artifacts();
    let m = format!("{} {CLI}\n", target_id(Product::Cli));
    let json = build(
        "1.2.0",
        "https://dl.example.com",
        "Notes.",
        &m,
        None,
        d.path(),
    )
    .unwrap();
    let cfg = build_config_with(
        Product::Cli,
        Some("https://dl.example.com/latest.json"),
        Some("https://dl.example.com"),
        "key",
        "1.0.0",
    )
    .unwrap();
    let a = evaluate(json.as_bytes(), &cfg).unwrap().expect("offered");
    assert_eq!(
        (a.format, a.size, a.url.as_str()),
        (Format::Binary, 3, &*format!("https://dl.example.com/{CLI}"))
    );
}

#[test]
fn bad_inputs_are_refused_with_the_culprit_named() {
    let d = artifacts();
    let ok = map();
    let cases: Vec<(&str, String, &str)> = vec![
        ("1.2.0-rc.1", ok.clone(), "1.2.0-rc.1"),
        ("1.2", ok.clone(), "1.2"),
        (
            "1.2.0",
            "app-linux-x86_64 missing.AppImage\n".to_string(),
            "missing.AppImage",
        ),
        ("1.2.0", format!("app-linux {APPIMAGE}\n"), "app-linux"),
        (
            "1.2.0",
            format!("tool-linux-x86_64 {APPIMAGE}\n"),
            "tool-linux-x86_64",
        ),
        (
            "1.2.0",
            format!("app-linux-riscv64 {APPIMAGE}\n"),
            "app-linux-riscv64",
        ),
        (
            "1.2.0",
            format!("app-bsd-x86_64 {APPIMAGE}\n"),
            "app-bsd-x86_64",
        ),
        (
            "1.2.0",
            format!("{ok}app-linux-x86_64 {CLI}\n"),
            "app-linux-x86_64",
        ),
        (
            "1.2.0",
            "cli-linux-x86_64 ../etc/passwd\n".to_string(),
            "../etc/passwd",
        ),
        (
            "1.2.0",
            "cli-linux-x86_64\n".to_string(),
            "cli-linux-x86_64",
        ),
        ("1.2.0", "\n".to_string(), "empty"),
    ];
    for (version, m, culprit) in cases {
        let err = build(version, "https://x", "", &m, None, d.path())
            .unwrap_err()
            .to_string();
        assert!(err.contains(culprit), "{version} {m:?}: {err}");
    }
}

#[test]
fn the_command_writes_latest_json_into_the_directory() {
    let d = artifacts();
    let inputs = tempfile::tempdir().unwrap();
    let (notes, map_file) = (
        inputs.path().join("notes.txt"),
        inputs.path().join("map.txt"),
    );
    std::fs::write(&notes, "Notes.\n").unwrap();
    std::fs::write(&map_file, map()).unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args([
            "update-manifest",
            "--version",
            "1.2.0",
            "--base-url",
            "https://x",
            "--notes",
        ])
        .arg(&notes)
        .arg("--map")
        .arg(&map_file)
        .arg(d.path())
        .status()
        .unwrap();
    assert!(status.success());
    let text = std::fs::read_to_string(d.path().join("latest.json")).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&text).unwrap()["version"],
        "1.2.0"
    );
}
