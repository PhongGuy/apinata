//! The committed packager metadata of apinata-app, checked through the real `cargo metadata`.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};
use xtask::icons::PNG_SIZES;
use xtask::packager_config::{Signing, packager_config};

fn config() -> Value {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let out = Command::new(cargo)
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(xtask::workspace_root())
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let metadata: Value = serde_json::from_slice(&out.stdout).unwrap();
    packager_config(&metadata, None, Path::new("/o"), &Signing::default())
        .expect("committed metadata has no signing settings")
}

#[test]
fn identity_and_binaries() {
    let c = config();
    assert_eq!(c["productName"], "Apinata");
    assert_eq!(c["identifier"], "dev.apinata.app");
    assert_eq!(c["category"], "DeveloperTool");
    assert_eq!(
        c["version"],
        env!("CARGO_PKG_VERSION"),
        "xtask shares the workspace version"
    );
    assert_eq!(
        c["binaries"],
        json!([{ "path": "apinata-app", "main": true }, { "path": "apinata", "main": false }])
    );
    assert!(
        c.get("formats").is_none(),
        "formats are chosen per OS on the command line"
    );
}

#[test]
fn deb_declares_gpui_libraries() {
    assert_eq!(
        config()["deb"]["depends"],
        json!([
            "libxcb1",
            "libxkbcommon0",
            "libxkbcommon-x11-0",
            "libvulkan1",
            "libwayland-client0",
            "libwayland-egl1",
            "libegl1"
        ])
    );
}

#[test]
fn icons_point_at_committed_exports() {
    let icons: Vec<PathBuf> = config()["icons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| PathBuf::from(i.as_str().unwrap()))
        .collect();
    assert_eq!(icons.len(), 3);
    let png_dir = icons[0].parent().unwrap();
    assert_eq!(icons[0].file_name().unwrap(), "*.png");
    for size in PNG_SIZES {
        assert!(
            png_dir.join(format!("{size}x{size}.png")).is_file(),
            "{size}"
        );
    }
    assert!(icons[1].ends_with("apinata.icns") && icons[1].is_file());
    assert!(icons[2].ends_with("apinata.ico") && icons[2].is_file());
}

#[test]
fn the_desktop_template_names_the_window_class() {
    let path = PathBuf::from(config()["deb"]["desktopTemplate"].as_str().unwrap());
    let text = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    for want in [
        "[Desktop Entry]",
        "Exec={{exec}} {{exec_arg}}",
        "Icon={{icon}}",
        "Name={{name}}",
        "StartupWMClass=apinata-app",
        "Type=Application",
    ] {
        assert!(
            lines.contains(&want),
            "{want} missing from {}",
            path.display()
        );
    }
}

#[test]
fn resources_ship_every_licence_text() {
    let resources = config()["resources"].clone();
    let list = resources.as_array().expect("resources is a list");
    assert_eq!(list.len(), 3);
    let glob = PathBuf::from(list[0].as_str().unwrap());
    assert_eq!(glob.file_name().unwrap(), "*.txt");
    let dir = glob.parent().unwrap();
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".txt"))
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "Inter-OFL.txt",
            "JetBrainsMono-OFL.txt",
            "Lucide-ISC.txt",
            "ajv-MIT.txt",
            "ajv-formats-MIT.txt",
            "arborium-MIT.txt",
            "base64-js-MIT.txt",
            "bruno-MIT.txt",
            "buffer-MIT.txt",
            "chai-MIT.txt",
            "crypto-js-MIT.txt",
            "fast-deep-equal-MIT.txt",
            "fast-uri-BSD-3-Clause.txt",
            "graphql-js-MIT.txt",
            "ieee754-BSD-3-Clause.txt",
            "json-schema-traverse-MIT.txt",
            "jsonwebtoken-MIT.txt",
            "lodash-MIT.txt",
            "moment-MIT.txt",
            "ms-MIT.txt",
            "netmask-MIT.txt",
            "pac-resolver-MIT.txt",
            "publicsuffix-list-MPL-2.0.txt",
            "tv4-LICENSE.txt"
        ]
    );
}

#[test]
fn resources_ship_the_generated_crate_notices() {
    let resources = config()["resources"].clone();
    for (i, name) in [(1, "THIRD-PARTY.md"), (2, "THIRD-PARTY-CRATES.md")] {
        let path = PathBuf::from(resources[i].as_str().expect("a path"));
        assert_eq!(path.file_name().unwrap(), name);
        // Both sit at the repository root, next to LICENSE-MIT; the crate notices are generated.
        assert!(path.parent().unwrap().join("LICENSE-MIT").is_file());
    }
}
