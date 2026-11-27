//! A request's own certificate check and HTTP version: how the file stores them, how editing
//! writes them and how resolution reads them, failing closed on values it does not understand.

use crate::common;
use crate::parity;

use apinata_core::{HttpVersionPolicy, RequestFile, RequestSettings, TlsVerify, Warning};
use common::{at, setup};
use parity::{file_with, keys, saved};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

fn http(top: &str) -> String {
    format!("info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: https://x.test/\n{top}")
}

#[test]
fn absent_keys_inherit_and_new_files_write_none() {
    let plain = RequestSettings::new();
    assert_eq!(plain.verify_tls, TlsVerify::Inherit);
    assert_eq!(plain.http_version, None);
    let fresh = RequestSettings::for_new_file();
    assert_eq!(
        (fresh.verify_tls, fresh.http_version),
        (TlsVerify::Inherit, None)
    );
}

#[test]
fn editing_writes_the_keys_after_omit_headers_and_default_removes_them() {
    let text = http("settings:\n  encodeUrl: true\n  omitHeaders:\n    - User-Agent\n");
    let (_dir, path) = file_with(&text);
    let mut r = RequestFile::load(&path).expect("loads");
    let mut s = r.settings();
    s.verify_tls = TlsVerify::Off;
    s.http_version = Some(HttpVersionPolicy::Http2PriorKnowledge);
    r.set_settings(s);
    r.save().expect("saves");
    let after = saved(&path);
    assert_eq!(
        keys(&after["settings"]),
        ["encodeUrl", "omitHeaders", "verifySsl", "httpVersion"]
    );
    assert_eq!(
        after["settings"]["verifySsl"],
        yaml_serde::Value::Bool(false)
    );
    assert_eq!(
        after["settings"]["httpVersion"],
        yaml_serde::Value::from("2-prior-knowledge")
    );

    let mut r = RequestFile::load(&path).expect("reloads");
    let s = r.settings();
    assert_eq!(s.verify_tls, TlsVerify::Off);
    assert_eq!(s.http_version, Some(HttpVersionPolicy::Http2PriorKnowledge));
    let mut s = r.settings();
    s.verify_tls = TlsVerify::Inherit;
    s.http_version = None;
    r.set_settings(s);
    r.save().expect("saves");
    assert_eq!(
        keys(&saved(&path)["settings"]),
        ["encodeUrl", "omitHeaders"]
    );
}

#[test]
fn a_file_without_settings_loses_the_block_again_when_set_back_to_default() {
    let (_dir, path) = file_with(&http(""));
    let mut r = RequestFile::load(&path).expect("loads");
    let mut s = r.settings();
    s.verify_tls = TlsVerify::On;
    r.set_settings(s);
    assert!(r.is_dirty());
    s.verify_tls = TlsVerify::Inherit;
    r.set_settings(s);
    assert!(!r.is_dirty());
}

#[test]
fn resolution_reads_the_keys_and_fails_closed_on_anything_else() {
    let files = [
        ("opencollection.yml", ROOT.to_owned()),
        ("plain.yml", http("")),
        (
            "off.yml",
            http("settings:\n  verifySsl: false\n  httpVersion: auto\n"),
        ),
        (
            "text.yml",
            http("settings:\n  verifySsl: \"false\"\n  httpVersion: \"1.1\"\n"),
        ),
        ("on.yml", http("settings:\n  verifySsl: true\n")),
        ("var.yml", http("settings:\n  verifySsl: \"{{skip}}\"\n")),
        ("null.yml", http("settings:\n  verifySsl: null\n")),
        (
            "junk.yml",
            http("settings:\n  verifySsl: 0\n  httpVersion: h2\n"),
        ),
    ];
    let refs: Vec<(&str, &str)> = files.iter().map(|(n, t)| (*n, t.as_str())).collect();
    let (_dir, c, ctx) = setup(&refs);
    let r = |rel: &str| at(&c, &ctx, rel).expect(rel);

    let plain = r("plain.yml");
    assert_eq!((plain.verify_tls(), plain.http_version()), (None, None));
    assert!(plain.warnings().is_empty());

    let off = r("off.yml");
    assert_eq!(off.verify_tls(), Some(false));
    assert_eq!(off.http_version(), Some(HttpVersionPolicy::Auto));
    // Only the YAML boolean turns the check off; the string keeps it on and says so.
    let text = r("text.yml");
    assert_eq!(text.verify_tls(), Some(true));
    assert_eq!(text.http_version(), Some(HttpVersionPolicy::Http1Only));
    assert_eq!(text.warnings(), [Warning::SettingIgnored("verifySsl")]);
    for (name, verify) in [("var.yml", Some(true)), ("null.yml", None)] {
        assert_eq!(r(name).verify_tls(), verify, "{name}");
    }
    assert_eq!(r("on.yml").verify_tls(), Some(true));

    // A value that is not understood keeps the check on and the default version, and says so.
    let junk = r("junk.yml");
    assert_eq!((junk.verify_tls(), junk.http_version()), (Some(true), None));
    assert_eq!(
        junk.warnings(),
        [
            Warning::SettingIgnored("verifySsl"),
            Warning::SettingIgnored("httpVersion")
        ]
    );
    assert_eq!(
        Warning::SettingIgnored("verifySsl").to_string(),
        "settings.verifySsl has a value that is not understood; certificates are verified"
    );
}

#[test]
fn default_on_a_block_holding_only_the_key_drops_the_block() {
    let (_dir, path) = file_with(&http("settings:\n  verifySsl: false\n"));
    let mut r = RequestFile::load(&path).expect("loads");
    let mut s = r.settings();
    s.verify_tls = TlsVerify::Inherit;
    r.set_settings(s);
    r.save().expect("saves");
    assert!(saved(&path).get("settings").is_none(), "{:?}", saved(&path));
}
