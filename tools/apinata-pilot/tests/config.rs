//! The repository's pilot wiring: `gpui-pilot.toml` builds and launches this package, and
//! `.mcp.json` registers the server.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn the_headless_config_builds_and_runs_this_package() {
    let text = std::fs::read_to_string(repo_root().join("gpui-pilot.toml")).unwrap();
    let config: toml::Table = toml::from_str(&text).unwrap();
    let headless = config["headless"].as_table().unwrap();
    let build: Vec<&str> = headless["build"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(
        build,
        [
            "cargo",
            "build",
            "--manifest-path",
            "tools/apinata-pilot/Cargo.toml"
        ]
    );
    assert!(repo_root().join(build[3]).is_file());
    assert_eq!(
        headless["binary"].as_str(),
        Some("tools/apinata-pilot/target/debug/apinata-pilot")
    );
}

#[test]
fn the_session_config_builds_and_runs_the_real_app() {
    let text = std::fs::read_to_string(repo_root().join("gpui-pilot.toml")).unwrap();
    let config: toml::Table = toml::from_str(&text).unwrap();
    let session = config["session"].as_table().unwrap();
    let build: Vec<&str> = session["build"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(
        build,
        [
            "cargo",
            "build",
            "-p",
            "apinata-app",
            "--bin",
            "apinata-app"
        ]
    );
    assert_eq!(session["binary"].as_str(), Some("target/debug/apinata-app"));
}

#[test]
fn the_project_registers_the_server_over_stdio() {
    let text = std::fs::read_to_string(repo_root().join(".mcp.json")).unwrap();
    let config: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        config["mcpServers"]["gpui-pilot"],
        serde_json::json!({ "command": "gpui-pilot", "args": ["mcp"] })
    );
}
