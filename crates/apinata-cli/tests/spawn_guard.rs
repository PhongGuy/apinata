//! The real keychain is unreachable from this suite by construction: every spawn of the
//! binary goes through a helper in `common`, which pins the in-memory store.

use std::fs;
use std::path::Path;

use crate::common::{self, ROOT, write_tree};

/// Text that starts the binary without a helper. Built from pieces so this file does not
/// match itself.
fn bypass_markers() -> Vec<String> {
    vec![
        format!("CARGO_BIN_EXE_{}", "apinata\""),
        format!("{}(\"apinata\")", "cargo_bin"),
        format!("{}()", "get_program"),
    ]
}

#[test]
fn no_test_file_starts_the_binary_without_a_helper() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut offenders = Vec::new();
    let mut stack = vec![dir];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d).expect("tests dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let in_common = path.parent().unwrap().ends_with("common");
            if !name.ends_with(".rs") || in_common || name == "spawn_guard.rs" {
                continue;
            }
            let text = fs::read_to_string(&path).expect("read");
            for marker in bypass_markers() {
                if text.contains(&marker) {
                    offenders.push(format!("{}: {marker}", path.display()));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "spawns bypassing common: {offenders:#?}"
    );
}

#[test]
fn every_helper_pins_the_memory_store_and_points_the_session_bus_nowhere() {
    let std_cmd = common::apinata_std();
    let tokio_cmd = common::apinata_tokio();
    let assert_cmd = common::apinata();
    let envs = |all: Vec<(String, Option<String>)>| {
        let get = |k: &str| all.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        (
            get("APINATA_TEST_NO_KEYCHAIN"),
            get("DBUS_SESSION_BUS_ADDRESS"),
            get("XDG_RUNTIME_DIR"),
        )
    };
    let owned = |it: &mut dyn Iterator<Item = (&std::ffi::OsStr, Option<&std::ffi::OsStr>)>| {
        it.map(|(k, v)| {
            (
                k.to_string_lossy().into_owned(),
                v.map(|v| v.to_string_lossy().into_owned()),
            )
        })
        .collect::<Vec<_>>()
    };
    let sets = [
        envs(owned(&mut std_cmd.get_envs())),
        envs(owned(&mut tokio_cmd.as_std().get_envs())),
        envs(owned(&mut assert_cmd.get_envs())),
    ];
    for (store, bus, runtime) in sets {
        assert_eq!(store, Some(Some("1".to_owned())));
        // Unset is not enough: zbus falls back to /run/user/<uid>/bus, the real session bus.
        assert_eq!(
            bus,
            Some(Some(common::UNREACHABLE_BUS.to_owned())),
            "the session bus address names a socket that does not exist"
        );
        assert_eq!(runtime, Some(None), "the runtime dir is removed");
    }
}

const SECRET_ENV: &str = "name: dev\nvariables:\n  - secret: true\n    name: tok\n";
const NEEDS_SECRET: &str = "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://127.0.0.1:1/{{tok}}\"\n";

/// The `secret store:` lines a spawned command printed on stderr.
fn store_lines(args: &[&str], files: &[(&str, &str)]) -> Vec<String> {
    let dir = write_tree(files);
    let out = common::apinata()
        .env("APINATA_TEST_REPORT_STORE", "1")
        .args(args)
        .arg(dir.path())
        .output()
        .expect("runs");
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|l| l.starts_with("secret store:"))
        .map(str::to_owned)
        .collect()
}

fn secret_collection() -> [(&'static str, &'static str); 3] {
    [
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", SECRET_ENV),
        ("r.yml", NEEDS_SECRET),
    ]
}

#[test]
fn a_run_that_reads_a_secret_reports_the_store_it_opened_once() {
    let lines = store_lines(&["run", "--env", "dev"], &secret_collection());
    assert_eq!(lines, ["secret store: memory"]);
}

#[test]
fn a_run_that_reads_no_secret_opens_and_reports_no_store() {
    let lines = store_lines(&["run"], &[("opencollection.yml", ROOT)]);
    assert!(lines.is_empty(), "{lines:?}");
}

#[test]
fn no_keychain_opens_and_reports_no_store() {
    for cmd in ["run", "check"] {
        let lines = store_lines(
            &[cmd, "--env", "dev", "--no-keychain"],
            &secret_collection(),
        );
        assert!(lines.is_empty(), "{cmd}: {lines:?}");
    }
}

/// The platform store itself, with the bus pointed nowhere: it cannot reach any keychain, and
/// the failure reaches the user as the store-error hint, not as a bare "no value".
#[test]
fn an_unreachable_platform_store_is_reported_not_taken_for_no_value() {
    let dir = write_tree(&secret_collection());
    let out = common::apinata()
        .env_remove("APINATA_TEST_NO_KEYCHAIN")
        .args(["run", "--env", "dev"])
        .arg(dir.path())
        .output()
        .expect("runs");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(all.contains("secret store unavailable"), "{all}");
    assert!(!all.contains("with no value"), "{all}");
}
