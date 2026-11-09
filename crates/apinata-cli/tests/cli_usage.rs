//! Exit code 3: argument errors and collections that cannot be opened; help exits 0.

use crate::common;

use common::{ROOT, apinata, write_tree};
use predicates::prelude::*;
use predicates::str::contains;

#[test]
fn argument_errors_exit_3_instead_of_clap_2() {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    let path = dir.path().to_str().expect("utf-8");
    for args in [
        vec!["run", path, "--nope"],
        vec!["run", path, "--format", "xml"],
        vec!["run", path, "--env-var", "novalue"],
        vec!["run", path, "--env-var", "=v"],
        vec!["run"],
        vec!["launch", path],
        vec![],
    ] {
        apinata().args(&args).assert().code(3);
    }
}

#[test]
fn help_and_version_exit_0() {
    apinata()
        .arg("--help")
        .assert()
        .code(0)
        .stdout(contains("run"));
    apinata()
        .args(["run", "--help"])
        .assert()
        .code(0)
        .stdout(contains("--env-var"));
    apinata()
        .arg("--version")
        .assert()
        .code(0)
        .stdout(contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn help_names_the_third_party_notices_file() {
    apinata()
        .arg("--help")
        .assert()
        .code(0)
        .stdout(contains("THIRD-PARTY-CRATES.md"));
}

#[test]
fn unopenable_targets_exit_3_with_a_message() {
    let dir = write_tree(&[
        ("c/opencollection.yml", ROOT),
        ("c/environments/dev.yml", "name: dev\nvariables: []\n"),
        ("loose/r.yml", "info:\n  name: r\n"),
    ]);
    let at = |rel: &str| dir.path().join(rel);
    apinata()
        .arg("run")
        .arg(at("c/missing.yml"))
        .assert()
        .code(3)
        .stderr(contains("error:"));
    apinata()
        .arg("run")
        .arg(at("loose/r.yml"))
        .assert()
        .code(3)
        .stderr(contains("opencollection.yml"));
    apinata()
        .arg("run")
        .arg(at("c/environments/dev.yml"))
        .assert()
        .code(3);
    apinata()
        .arg("run")
        .arg(at("c"))
        .args(["--env", "nope"])
        .assert()
        .code(3)
        .stderr(contains("environment \"nope\" not found; available: dev"));
}

#[test]
fn a_malformed_env_var_never_echoes_its_value() {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    let path = dir.path().to_str().expect("utf-8");
    // A bare word is a variable name now, so only an empty key is malformed.
    for arg in ["=hunter2", "=hunter2=more"] {
        let out = apinata()
            .args(["run", path, "--env-var", arg])
            .output()
            .expect("runs");
        assert_eq!(out.status.code(), Some(3));
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stderr.contains("--env-var"), "{stderr}");
        assert!(stderr.contains("expected KEY=VALUE"), "{stderr}");
        assert!(!stderr.contains("hunter2"), "{stderr}");
        assert!(!stdout.contains("hunter2"), "{stdout}");
    }
}

#[test]
fn selecting_an_environment_file_is_a_usage_error_whether_or_not_it_loads() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", "name: dev\nvariables: []\n"),
        ("environments/broken.yml", "variables: [unclosed\n"),
    ]);
    for rel in [
        "environments/dev.yml",
        "environments/broken.yml",
        "environments",
    ] {
        apinata()
            .arg("run")
            .arg(dir.path().join(rel))
            .assert()
            .code(3)
            .stdout("")
            .stderr(contains("environment").and(contains("--env")));
    }
}

#[test]
fn run_help_lists_the_exit_codes() {
    apinata().args(["run", "--help"]).assert().code(0).stdout(
        contains("0  all passed")
            .and(contains("1  an assertion or test failed"))
            .and(contains("2  a request errored\n"))
            .and(contains(
                "3  usage error, collection or file that failed to load",
            )),
    );
}

#[test]
fn run_script_timeout_must_be_between_1_and_86400_seconds() {
    // One request, so the accepted timeout runs something (and fails to connect, exit 2).
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://127.0.0.1:9/\"\n",
        ),
    ]);
    for value in ["0", "86401", "18446744073709551615"] {
        apinata()
            .arg("run")
            .arg(dir.path())
            .args(["--script-timeout", value])
            .assert()
            .code(3)
            .stderr(contains("--script-timeout"));
    }
    apinata()
        .arg("run")
        .arg(dir.path())
        .args(["--script-timeout", "86400"])
        .assert()
        .code(2);
}

#[test]
fn run_help_says_cookie_jar_cannot_combine_with_disable_cookies() {
    apinata()
        .args(["run", "--help"])
        .assert()
        .code(0)
        .stdout(contains("Cannot be combined with --disable-cookies"));
}

/// `apinata ARGS --help` on stdout with every run of whitespace folded to one space, so a
/// phrase can be matched however clap wraps it.
fn help_text(args: &[&str]) -> String {
    let out = apinata().args(args).arg("--help").assert().code(0);
    let text = String::from_utf8(out.get_output().stdout.clone()).expect("utf-8");
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// Each command that creates a collection says where it goes and what that folder may hold.
#[test]
fn import_help_says_the_collection_gets_a_new_folder_inside_into() {
    let help = help_text(&["import", "collection"]);
    assert!(
        help.contains("a new folder named after it inside DIR"),
        "{help}"
    );
    assert!(
        help.contains("so DIR may already hold other files"),
        "{help}"
    );
}

#[test]
fn convert_and_clone_help_say_the_new_folder_must_not_exist() {
    let convert = help_text(&["convert"]);
    assert!(convert.contains("NEW_DIR must not exist yet"), "{convert}");
    let clone = help_text(&["git", "clone"]);
    assert!(clone.contains("The folder must not exist yet"), "{clone}");
}

#[test]
fn run_help_lists_exit_130_for_a_watch_stopped_before_any_run_finished() {
    let help = help_text(&["run"]);
    assert!(
        help.contains("130 watch stopped with Ctrl+C before any run finished"),
        "{help}"
    );
}

#[test]
fn import_help_says_it_never_fills_an_existing_folder() {
    let help = help_text(&["import", "collection"]);
    assert!(help.contains("never fills an existing folder"), "{help}");
}
