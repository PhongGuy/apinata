//! `apinata env new`, `cp`, `mv` and `rm`: whole environments, written by the core writer.

use crate::common;

use apinata_core::{Collection, MemoryStore};
use common::{ROOT, apinata, write_tree};
use serde_json::Value;
use std::path::Path;

const DEV: &str = "name: dev\ncolor: \"#00ff00\"\nvariables:\n  - name: host\n    value: example.test\n  - name: token\n    secret: true\n";
const CHILD: &str = "name: child\nextends: dev\nvariables:\n  - name: port\n    value: \"8080\"\n";
const PROD: &str = "name: prod\nvariables:\n  - name: host\n    value: prod.test\n";

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("environments/child.yml", CHILD),
        ("environments/prod.yml", PROD),
    ])
}

fn read(dir: &Path, env: &str) -> String {
    std::fs::read_to_string(dir.join(format!("environments/{env}.yml"))).unwrap()
}

/// The command's exit code, stdout and stderr.
fn run(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let out = apinata().current_dir(dir).args(args).output().unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

/// A twin collection with `op` applied through core directly, so the CLI's bytes can be
/// compared with the writer's.
fn twin(op: impl FnOnce(&mut Collection, &MemoryStore)) -> tempfile::TempDir {
    let dir = tree();
    let mut c = Collection::open(dir.path()).unwrap();
    op(&mut c, &MemoryStore::new());
    dir
}

#[test]
fn env_new_writes_the_core_file_and_suffixes_a_taken_name() {
    let dir = tree();
    let (code, out, err) = run(dir.path(), &["env", "new", "staging"]);
    assert_eq!((code, out.as_str()), (0, "created staging\n"), "{err}");
    let want = twin(|c, s| {
        c.create_environment("staging", s).unwrap();
    });
    assert_eq!(read(dir.path(), "staging"), read(want.path(), "staging"));

    let (code, out, _) = run(dir.path(), &["env", "new", "dev", "--json"]);
    assert_eq!(code, 0);
    let json: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(json["name"], "dev copy");
    assert!(dir.path().join("environments/dev copy.yml").is_file());
}

#[test]
fn env_new_refuses_an_invalid_name_with_exit_3() {
    let dir = tree();
    let (code, _, err) = run(dir.path(), &["env", "new", "   "]);
    assert_eq!(code, 3, "{err}");
    assert!(err.contains("not a valid environment name"), "{err}");
}

#[test]
fn env_cp_writes_the_core_copy_and_prints_its_name() {
    let dir = tree();
    let (code, out, err) = run(dir.path(), &["env", "cp", "dev"]);
    assert_eq!(
        (code, out.as_str()),
        (0, "copied dev to dev copy\n"),
        "{err}"
    );
    let want = twin(|c, s| {
        c.duplicate_environment("dev", s).unwrap();
    });
    assert_eq!(read(dir.path(), "dev copy"), read(want.path(), "dev copy"));
    assert_eq!(read(dir.path(), "dev"), DEV);

    let (code, out, _) = run(dir.path(), &["env", "cp", "dev", "--json"]);
    assert_eq!(code, 0);
    let json: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(json["name"], "dev copy 2");
}

#[test]
fn env_mv_renames_the_file_and_repoints_extends() {
    let dir = tree();
    let (code, out, err) = run(dir.path(), &["env", "mv", "dev", "staging"]);
    assert_eq!(
        (code, out.as_str()),
        (0, "renamed dev to staging\n"),
        "{err}"
    );
    assert!(!dir.path().join("environments/dev.yml").exists());
    let want = twin(|c, s| {
        c.rename_environment("dev", "staging", s).unwrap();
    });
    assert_eq!(read(dir.path(), "staging"), read(want.path(), "staging"));
    let child = read(dir.path(), "child");
    assert_eq!(child, read(want.path(), "child"));
    assert!(child.contains("extends: staging"), "{child}");
}

#[test]
fn env_mv_onto_an_existing_name_is_refused_with_exit_3() {
    let dir = tree();
    let (code, _, err) = run(dir.path(), &["env", "mv", "dev", "prod"]);
    assert_eq!(code, 3, "{err}");
    assert!(err.contains("environment `prod` already exists"), "{err}");
    assert_eq!(read(dir.path(), "dev"), DEV);
    assert_eq!(read(dir.path(), "prod"), PROD);
}

#[test]
fn env_rm_refuses_a_name_another_environment_extends_naming_it() {
    let dir = tree();
    let (code, _, err) = run(dir.path(), &["env", "rm", "dev"]);
    assert_eq!(code, 3, "{err}");
    assert!(err.contains("`child`"), "{err}");
    assert_eq!(read(dir.path(), "dev"), DEV);
}

#[test]
fn env_rm_deletes_the_file() {
    let dir = tree();
    let (code, out, err) = run(dir.path(), &["env", "rm", "prod"]);
    assert_eq!((code, out.as_str()), (0, "removed prod\n"), "{err}");
    assert!(!dir.path().join("environments/prod.yml").exists());
}

#[test]
fn an_unknown_environment_is_exit_3_for_cp_mv_and_rm() {
    let dir = tree();
    for args in [
        &["env", "cp", "nope"][..],
        &["env", "mv", "nope", "x"],
        &["env", "rm", "nope"],
    ] {
        let (code, _, err) = run(dir.path(), args);
        assert_eq!(code, 3, "{args:?}: {err}");
        assert!(err.contains("nope"), "{args:?}: {err}");
    }
}

/// The `secret store:` lines and the stderr of a command run with the store reported.
fn reported(dir: &Path, args: &[&str]) -> (Vec<String>, String) {
    let out = apinata()
        .current_dir(dir)
        .env("APINATA_TEST_REPORT_STORE", "1")
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "{args:?}: {out:?}");
    let err = String::from_utf8(out.stderr).unwrap();
    let lines = err
        .lines()
        .filter(|l| l.starts_with("secret store:"))
        .map(str::to_owned)
        .collect();
    (lines, err)
}

#[test]
fn without_no_keychain_cp_mv_and_rm_reach_the_store() {
    for args in [
        &["env", "cp", "dev"][..],
        &["env", "mv", "dev", "staging"],
        &["env", "rm", "prod"],
    ] {
        let dir = tree();
        // `prod` has no secret, so give it one for rm to remove.
        std::fs::write(
            dir.path().join("environments/prod.yml"),
            "name: prod\nvariables:\n  - name: token\n    secret: true\n",
        )
        .unwrap();
        let (lines, err) = reported(dir.path(), args);
        assert_eq!(lines, ["secret store: memory"], "{args:?}: {err}");
    }
}

#[test]
fn no_keychain_on_cp_mv_and_rm_opens_no_store_and_says_the_secrets_were_left() {
    let cases: [(&[&str], &str); 3] = [
        (&["env", "cp", "dev", "--no-keychain"], "not copied"),
        (&["env", "mv", "dev", "staging", "--no-keychain"], "left"),
        (&["env", "rm", "prod", "--no-keychain"], "left"),
    ];
    for (args, said) in cases {
        let dir = tree();
        std::fs::write(
            dir.path().join("environments/prod.yml"),
            "name: prod\nvariables:\n  - name: token\n    secret: true\n",
        )
        .unwrap();
        let (lines, err) = reported(dir.path(), args);
        assert!(lines.is_empty(), "{args:?}: {lines:?}");
        assert!(
            err.contains("--no-keychain") && err.contains(said) && err.contains("token"),
            "{args:?}: {err}"
        );
    }
}

#[test]
fn env_rm_removes_an_environment_that_extends_itself() {
    let dir = tree();
    std::fs::write(
        dir.path().join("environments/prod.yml"),
        "name: prod\nextends: prod\n",
    )
    .unwrap();
    let (code, out, err) = run(dir.path(), &["env", "rm", "prod"]);
    assert_eq!((code, out.as_str()), (0, "removed prod\n"), "{err}");
    assert!(!dir.path().join("environments/prod.yml").exists());
}

#[test]
fn env_rm_refusal_reads_for_one_or_many_children() {
    let dir = tree();
    let (_, _, err) = run(dir.path(), &["env", "rm", "dev"]);
    assert!(err.contains("change `extends` in each first"), "{err}");
}

#[test]
fn env_mv_onto_the_same_name_says_unchanged_and_writes_nothing() {
    let dir = tree();
    let (code, out, err) = run(dir.path(), &["env", "mv", "dev", "dev", "--no-keychain"]);
    assert_eq!((code, out.as_str()), (0, "unchanged: dev\n"), "{err}");
    assert!(!err.contains("note:"), "{err}");
    assert_eq!(read(dir.path(), "dev"), DEV);
}

#[test]
fn env_mv_changing_only_case_renames_the_file_and_repoints_extends() {
    let dir = tree();
    let (code, out, err) = run(dir.path(), &["env", "mv", "dev", "Dev"]);
    assert_eq!((code, out.as_str()), (0, "renamed dev to Dev\n"), "{err}");
    let names: Vec<String> = std::fs::read_dir(dir.path().join("environments"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert!(
        names.contains(&"Dev.yml".to_owned()) && !names.contains(&"dev.yml".to_owned()),
        "{names:?}"
    );
    assert!(read(dir.path(), "Dev").starts_with("name: Dev\n"));
    assert!(read(dir.path(), "child").contains("extends: Dev"));
}

#[test]
fn env_rm_refuses_a_name_two_environments_extend_naming_both() {
    let dir = tree();
    std::fs::write(
        dir.path().join("environments/qa.yml"),
        "name: qa\nextends: dev\n",
    )
    .unwrap();
    let (code, _, err) = run(dir.path(), &["env", "rm", "dev"]);
    assert_eq!(code, 3, "{err}");
    assert!(
        err.contains(
            "environment `dev` is extended by `child`, `qa`; change `extends` in each first"
        ),
        "{err}"
    );
    assert_eq!(read(dir.path(), "dev"), DEV);
}
