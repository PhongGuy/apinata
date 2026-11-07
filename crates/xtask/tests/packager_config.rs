use std::path::Path;

use serde_json::{Value, json};
use xtask::packager_config::{Signing, packager_config, write_private};

fn metadata(packager: Value) -> Value {
    json!({
        "target_directory": "/t",
        "packages": [
            { "name": "apinata-core", "version": "9.9.9", "manifest_path": "/r/crates/apinata-core/Cargo.toml", "metadata": null },
            {
                "name": "apinata-app",
                "version": "1.2.3",
                "manifest_path": "/r/crates/apinata-app/Cargo.toml",
                "metadata": { "packager": packager }
            }
        ]
    })
}

fn base() -> Value {
    json!({
        "productName": "Apinata",
        "icons": ["../../assets/icon/generated/png/*.png", "/abs/apinata.icns"],
        "macos": { "minimumSystemVersion": "11.0" },
        "deb": { "depends": ["libxcb1"] }
    })
}

fn unsigned(m: &Value, target: Option<&str>) -> Value {
    packager_config(m, target, Path::new("/o"), &Signing::default()).unwrap()
}

#[test]
fn fills_name_version_and_directories() {
    let c = unsigned(&metadata(base()), None);
    assert_eq!(c["name"], "apinata-app");
    assert_eq!(c["version"], "1.2.3");
    assert_eq!(c["binariesDir"], "/t/release");
    assert_eq!(c["outDir"], "/o");
    assert_eq!(c["productName"], "Apinata");
    assert_eq!(c["deb"]["depends"], json!(["libxcb1"]));
    let cross = unsigned(&metadata(base()), Some("aarch64-unknown-linux-gnu"));
    assert_eq!(cross["binariesDir"], "/t/aarch64-unknown-linux-gnu/release");
}

#[test]
fn icons_resolve_against_the_manifest_directory() {
    assert_eq!(
        unsigned(&metadata(base()), None)["icons"],
        json!([
            "/r/crates/apinata-app/../../assets/icon/generated/png/*.png",
            "/abs/apinata.icns"
        ])
    );
}

#[test]
fn signing_is_added_only_from_the_environment() {
    let plain = unsigned(&metadata(base()), None);
    assert_eq!(plain["macos"], json!({ "minimumSystemVersion": "11.0" }));
    assert!(plain.get("windows").is_none());

    let signing = Signing {
        macos_identity: Some("Developer ID Application: Example (TEAM123)".into()),
    };
    let signed = packager_config(&metadata(base()), None, Path::new("/o"), &signing).unwrap();
    assert_eq!(
        signed["macos"]["signingIdentity"],
        "Developer ID Application: Example (TEAM123)"
    );
    assert_eq!(signed["macos"]["minimumSystemVersion"], "11.0");
}

#[test]
fn windows_builds_take_no_sign_command_from_the_environment() {
    let signing = Signing::from_vars(|n| {
        (n == "APINATA_WINDOWS_SIGN_COMMAND").then(|| "signtool sign %1".to_owned())
    });
    let config = packager_config(&metadata(base()), None, Path::new("/o"), &signing).unwrap();
    assert!(config.get("windows").is_none(), "{config}");
}

#[test]
fn blank_variables_mean_unsigned() {
    let vars = |name: &str| match name {
        "APINATA_MACOS_SIGNING_IDENTITY" => Some("  ".to_owned()),
        _ => None,
    };
    assert_eq!(Signing::from_vars(vars), Signing::default());
    let set =
        Signing::from_vars(|n| (n == "APINATA_MACOS_SIGNING_IDENTITY").then(|| "Dev".to_owned()));
    assert_eq!(set.macos_identity.as_deref(), Some("Dev"));
}

#[test]
fn committed_signing_settings_are_refused() {
    for (section, key) in [
        ("macos", "signingIdentity"),
        ("macos", "signing-identity"),
        ("macos", "signing_identity"),
        ("windows", "signCommand"),
        ("windows", "sign-command"),
        ("windows", "sign_command"),
        ("windows", "certificateThumbprint"),
        ("windows", "certificate-thumbprint"),
        ("windows", "certificate_thumbprint"),
    ] {
        let mut committed = base();
        committed[section] = json!({ key: "x" });
        let err = packager_config(
            &metadata(committed),
            None,
            Path::new("/o"),
            &Signing::default(),
        )
        .unwrap_err();
        assert!(err.to_string().contains(key), "{err}");
    }
}

#[test]
fn a_missing_package_or_section_is_an_error() {
    let no_app = json!({ "target_directory": "/t", "packages": [] });
    assert!(packager_config(&no_app, None, Path::new("/o"), &Signing::default()).is_err());
    let mut no_section = metadata(base());
    no_section["packages"][1]["metadata"] = Value::Null;
    let err = packager_config(&no_section, None, Path::new("/o"), &Signing::default()).unwrap_err();
    assert!(
        err.to_string().contains("[package.metadata.packager]"),
        "{err}"
    );
}

#[cfg(unix)]
#[test]
fn write_private_leaves_an_owner_only_file() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("packager-config.json");
    std::fs::write(&path, "old").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    write_private(&path, b"{}").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"{}");
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn the_desktop_template_resolves_against_the_manifest_directory() {
    let mut p = base();
    p["deb"]["desktopTemplate"] = json!("packaging/apinata-app.desktop");
    let c = unsigned(&metadata(p), None);
    assert_eq!(
        c["deb"]["desktopTemplate"],
        "/r/crates/apinata-app/packaging/apinata-app.desktop"
    );
    assert_eq!(c["deb"]["depends"], json!(["libxcb1"]));
}

#[test]
fn a_config_without_a_template_gains_none() {
    assert!(
        unsigned(&metadata(base()), None)["deb"]
            .get("desktopTemplate")
            .is_none()
    );
}

#[test]
fn resources_resolve_against_the_manifest_directory() {
    let mut p = base();
    p["resources"] = json!([
        "../../licenses/*.txt",
        { "src": "notes/README.txt", "target": "doc/README.txt" },
        "/abs/x.txt"
    ]);
    assert_eq!(
        unsigned(&metadata(p), None)["resources"],
        json!([
            "/r/crates/apinata-app/../../licenses/*.txt",
            { "src": "/r/crates/apinata-app/notes/README.txt", "target": "doc/README.txt" },
            "/abs/x.txt"
        ])
    );
}

#[test]
fn a_config_without_resources_gains_none() {
    assert!(unsigned(&metadata(base()), None).get("resources").is_none());
}
