//! `run --env` and `--global-env` with an unknown name list the names that exist.

use crate::common;

use common::{ROOT, apinata, write_tree};
use predicates::str::contains;

const ENV: &str = "name: x\nvariables: []\n";

#[test]
fn env_show_lists_names_and_suggests() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", ENV),
        ("environments/prod.yml", ENV),
    ]);
    apinata()
        .current_dir(dir.path())
        .args(["env", "show", "prd"])
        .assert()
        .code(3)
        .stderr(contains(
            "environment \"prd\" not found; available: dev, prod; did you mean prod?",
        ));
}

#[test]
fn mcp_global_env_gets_the_full_core_text() {
    let profile = tempfile::TempDir::new().expect("tempdir");
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    apinata()
        .env("APINATA_PROFILE_DIR", profile.path())
        .args(["mcp", "--collection"])
        .arg(dir.path())
        .args(["--global-env", "x"])
        .timeout(std::time::Duration::from_secs(30))
        .assert()
        .code(3)
        .stderr(contains(
            "global environment \"x\" not found; no global environments",
        ));
}

#[test]
fn an_env_typo_lists_names_and_suggests_one() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", ENV),
        ("environments/staging.yml", ENV),
    ]);
    apinata()
        .arg("run")
        .arg(dir.path())
        .args(["--env", "stagin"])
        .assert()
        .code(3)
        .stderr(contains(
            "environment \"stagin\" not found; available: dev, staging; did you mean staging?",
        ));
}

#[test]
fn a_case_typo_suggests_the_right_case() {
    let dir = write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", ENV)]);
    apinata()
        .arg("run")
        .arg(dir.path())
        .args(["--env", "Dev"])
        .assert()
        .code(3)
        .stderr(contains("available: dev; did you mean dev?"));
}

#[test]
fn a_collection_without_environments_says_so() {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    apinata()
        .arg("run")
        .arg(dir.path())
        .args(["--env", "x"])
        .assert()
        .code(3)
        .stderr(contains(
            "environment \"x\" not found; this collection has no environments",
        ));
}

#[test]
fn an_unknown_global_env_lists_the_global_names() {
    let profile = write_tree(&[("global-environments/shared.yml", ENV)]);
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    let out = apinata()
        .env("APINATA_PROFILE_DIR", profile.path())
        .arg("run")
        .arg(dir.path())
        .args(["--global-env", "shard"])
        .assert()
        .code(3);
    let err = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert!(
        err.contains("global environment \"shard\" not found; "),
        "{err}"
    );
}

#[test]
fn no_global_environments_says_so() {
    let profile = tempfile::TempDir::new().expect("tempdir");
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    apinata()
        .env("APINATA_PROFILE_DIR", profile.path())
        .arg("run")
        .arg(dir.path())
        .args(["--global-env", "x"])
        .assert()
        .code(3)
        .stderr(contains(
            "global environment \"x\" not found; no global environments",
        ));
}
