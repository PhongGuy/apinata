//! The manifest rules applied after the signature has been verified, and the build config.
#![cfg(feature = "update")]

use apinata_core::update::{
    Format, Product, UpdateConfig, UpdateError, build_config, build_config_with, evaluate,
    target_id,
};
use std::time::Duration;

const PAGE: &str = "https://example.com/download";
const SHA: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

fn cfg(current: &str) -> UpdateConfig {
    build_config_with(
        Product::Cli,
        Some("https://example.com/latest.json"),
        Some(PAGE),
        "a-key\n",
        current,
    )
    .unwrap()
}

fn manifest(version: &str, target: &str, format: &str, url: &str, sha: &str) -> String {
    format!(
        r#"{{"version":"{version}","pub_date":"2026-11-01T12:00:00Z","notes":"Fixes.","extra":1,
          "targets":{{"{target}":{{"url":"{url}","format":"{format}","size":3,"sha256":"{sha}","more":true}}}}}}"#
    )
}

fn own(version: &str) -> String {
    manifest(
        version,
        &target_id(Product::Cli),
        "binary",
        "https://example.com/apinata",
        SHA,
    )
}

#[test]
fn target_ids_name_product_os_and_arch() {
    let id = target_id(Product::App);
    let parts: Vec<&str> = id.splitn(3, '-').collect();
    assert_eq!(parts[0], "app");
    assert!(["linux", "macos", "windows"].contains(&parts[1]), "{id}");
    assert_eq!(parts[2], std::env::consts::ARCH);
    assert!(target_id(Product::Cli).starts_with("cli-"));
}

#[test]
fn a_newer_stable_release_is_offered_with_its_entry() {
    let a = evaluate(own("1.2.0").as_bytes(), &cfg("1.1.9"))
        .unwrap()
        .expect("an update");
    assert_eq!(a.version, "1.2.0");
    assert_eq!(a.notes.as_deref(), Some("Fixes."));
    assert_eq!(a.pub_date.as_deref(), Some("2026-11-01T12:00:00Z"));
    assert_eq!(a.format, Format::Binary);
    assert_eq!(
        (a.url.as_str(), a.sha256.as_str(), a.size),
        ("https://example.com/apinata", SHA, 3)
    );
}

#[test]
fn only_a_strictly_greater_version_is_offered() {
    let table = [
        ("1.0.0", "1.0.0", false),
        ("0.9.9", "1.0.0", false),
        ("1.0.1", "1.0.0", true),
        ("2.0.0", "1.9.9", true),
        ("1.0.0", "1.0.0-dev", true),
    ];
    for (remote, current, offered) in table {
        let got = evaluate(own(remote).as_bytes(), &cfg(current)).unwrap();
        assert_eq!(got.is_some(), offered, "{remote} against {current}");
    }
}

#[test]
fn a_pre_release_manifest_is_invalid() {
    for v in ["1.2.0-rc.1", "9.0.0-beta"] {
        let err = evaluate(own(v).as_bytes(), &cfg("0.1.0")).unwrap_err();
        assert!(
            matches!(err, UpdateError::ManifestInvalid(_)),
            "{v}: {err:?}"
        );
    }
}

#[test]
fn a_missing_target_is_no_entry() {
    let m = manifest(
        "2.0.0",
        "cli-plan9-mips",
        "binary",
        "https://example.com/a",
        SHA,
    );
    assert_eq!(
        evaluate(m.as_bytes(), &cfg("0.1.0")).unwrap_err(),
        UpdateError::NoEntryForTarget(target_id(Product::Cli))
    );
}

#[test]
fn malformed_documents_and_entries_are_invalid() {
    let id = target_id(Product::Cli);
    let bad = [
        "not json".to_string(),
        r#"{"targets":{}}"#.to_string(),
        manifest("two", &id, "binary", "https://example.com/a", SHA),
        manifest("2.0.0", &id, "dmg", "https://example.com/a", SHA),
        manifest("2.0.0", &id, "binary", "http://example.com/a", SHA),
        manifest("2.0.0", &id, "binary", "https://example.com/a", "abc"),
        manifest(
            "2.0.0",
            &id,
            "binary",
            "https://example.com/a",
            &SHA.to_uppercase(),
        ),
    ];
    for m in bad {
        let got = evaluate(m.as_bytes(), &cfg("0.1.0"));
        assert!(
            matches!(got, Err(UpdateError::ManifestInvalid(_))),
            "{m}: {got:?}"
        );
    }
}

#[test]
fn every_format_name_maps() {
    let id = target_id(Product::Cli);
    for (name, format) in [
        ("appimage", Format::AppImage),
        ("app", Format::App),
        ("nsis", Format::Nsis),
        ("binary", Format::Binary),
    ] {
        let m = manifest("2.0.0", &id, name, "https://example.com/a", SHA);
        assert_eq!(
            evaluate(m.as_bytes(), &cfg("0.1.0"))
                .unwrap()
                .unwrap()
                .format,
            format
        );
    }
}

#[test]
fn loopback_http_artifacts_are_allowed_for_local_testing() {
    let m = manifest(
        "2.0.0",
        &target_id(Product::Cli),
        "binary",
        "http://127.0.0.1:9/a",
        SHA,
    );
    assert!(evaluate(m.as_bytes(), &cfg("0.1.0")).unwrap().is_some());
}

#[test]
fn any_missing_input_disables_updates() {
    let url = Some("https://e.com/l.json");
    let try_ = |u: Option<&str>, p: Option<&str>, k: &str| {
        build_config_with(Product::App, u, p, k, "0.1.0")
    };
    assert_eq!(
        try_(None, Some(PAGE), "k").unwrap_err(),
        UpdateError::Disabled
    );
    assert_eq!(
        try_(Some(""), Some(PAGE), "k").unwrap_err(),
        UpdateError::Disabled
    );
    assert_eq!(try_(url, None, "k").unwrap_err(), UpdateError::Disabled);
    assert_eq!(try_(url, Some(""), "k").unwrap_err(), UpdateError::Disabled);
    assert_eq!(
        try_(url, Some(PAGE), "").unwrap_err(),
        UpdateError::Disabled
    );
    assert_eq!(
        try_(url, Some(PAGE), "# comment only\n\n").unwrap_err(),
        UpdateError::Disabled
    );
    assert_eq!(
        UpdateError::Disabled.to_string(),
        "Update checks are not configured for this build"
    );
}

#[test]
fn a_config_keeps_its_inputs_and_default_limits() {
    let c = build_config_with(
        Product::App,
        Some("https://e.com/l.json"),
        Some(PAGE),
        "# k\nkey-one\n  key-two  \n",
        "0.3.0",
    )
    .unwrap();
    assert_eq!(c.manifest_url, "https://e.com/l.json");
    assert_eq!(c.download_page, PAGE);
    assert_eq!(c.keys, ["key-one", "key-two"]);
    assert_eq!(c.current_version, "0.3.0");
    assert_eq!(c.target_id, target_id(Product::App));
    assert_eq!(c.limits.connect, Duration::from_secs(10));
    assert_eq!(c.limits.manifest, Duration::from_secs(30));
    assert_eq!(c.limits.artifact_total, Duration::from_secs(15 * 60));
    assert_eq!(c.limits.artifact_idle, Duration::from_secs(30));
}

#[test]
fn a_plain_http_manifest_url_is_refused_unless_loopback() {
    let r = build_config_with(
        Product::Cli,
        Some("http://example.com/l.json"),
        Some(PAGE),
        "k",
        "0.1.0",
    );
    assert!(matches!(r, Err(UpdateError::ManifestInvalid(_))), "{r:?}");
    for local in [
        "http://127.0.0.1:8080/l.json",
        "http://localhost/l.json",
        "http://[::1]:9/l.json",
    ] {
        assert!(
            build_config_with(Product::Cli, Some(local), Some(PAGE), "k", "0.1.0").is_ok(),
            "{local}"
        );
    }
}

#[test]
fn the_download_page_must_be_https() {
    let url = Some("https://e.com/l.json");
    for page in [
        "http://example.com/download",
        "ftp://example.com/",
        "not a url",
    ] {
        let r = build_config_with(Product::App, url, Some(page), "k", "0.1.0");
        assert!(
            matches!(r, Err(UpdateError::ManifestInvalid(_))),
            "{page}: {r:?}"
        );
    }
    // A loopback page serves local runs, so only debug builds accept it.
    let r = build_config_with(
        Product::App,
        url,
        Some("http://127.0.0.1:8000/"),
        "k",
        "0.1.0",
    );
    if cfg!(debug_assertions) {
        assert!(r.is_ok(), "{r:?}");
    } else {
        assert!(matches!(r, Err(UpdateError::ManifestInvalid(_))), "{r:?}");
    }
    // A loopback manifest URL stays allowed in every profile.
    let r = build_config_with(
        Product::App,
        Some("http://127.0.0.1:8000/l.json"),
        Some("https://e.com/download"),
        "k",
        "0.1.0",
    );
    assert!(r.is_ok(), "{r:?}");
}

#[test]
fn the_unconfigured_build_is_disabled() {
    // The repository builds without the release variables and with no key in keys.txt.
    let unset = std::env::var_os("APINATA_UPDATE_MANIFEST_URL").is_none()
        && option_env!("APINATA_UPDATE_MANIFEST_URL").is_none();
    if unset {
        assert_eq!(
            build_config(Product::Cli).unwrap_err(),
            UpdateError::Disabled
        );
    }
}
