//! `apinata env renames`: listing, moving and picking the stored secrets a pulled rename left
//! under an old name, against a file-backed test store, and the hint `git pull` prints.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

use crate::cli_git_clone::{cli, cloned, err_text, out_text};
use crate::cli_git_sync::head;
use crate::git_support::{Sandbox, write};

use std::fs;
use std::path::{Path, PathBuf};

use apinata_core::Collection;
use serde_json::{Value, json};

/// Every stored value starts with this, so a leak is one `contains` away.
const MARK: &str = "SECRETVALUE";

fn env(name: &str, secrets: &[&str]) -> String {
    let mut text = format!("name: {name}\nvariables:\n  - name: host\n    value: h.test\n");
    for v in secrets {
        text.push_str(&format!("  - name: {v}\n    secret: true\n"));
    }
    text
}

/// A repository `col` and the file store the CLI reads, with the commit before the edit.
struct Case {
    sb: Sandbox,
    col: PathBuf,
    base: String,
    secrets: PathBuf,
}

/// A commit of `before`, then one applying `after`: dev renamed to staging (token free, key
/// taken) and qa's `old` replaced by two new secrets, which leaves it ambiguous.
fn case() -> Case {
    let sb = Sandbox::new();
    let col = sb.repo("col");
    write(
        &col,
        "opencollection.yml",
        "opencollection: 1.0.0\ninfo:\n  name: c\n",
    );
    write(&col, "environments/dev.yml", &env("dev", &["token", "key"]));
    write(&col, "environments/qa.yml", &env("qa", &["old"]));
    sb.commit_all(&col, "base");
    let base = head(&sb, &col);
    fs::remove_file(col.join("environments/dev.yml")).unwrap();
    write(
        &col,
        "environments/staging.yml",
        &env("staging", &["token", "key"]),
    );
    write(&col, "environments/qa.yml", &env("qa", &["a", "b"]));
    sb.commit_all(&col, "edit");
    let secrets = sb.path().join("secrets.json");
    let case = Case {
        sb,
        col,
        base,
        secrets,
    };
    case.store(&[
        ("dev", "token", "dev-token"),
        ("dev", "key", "dev-key"),
        ("staging", "key", "staging-key"),
        ("qa", "old", "qa-old"),
    ]);
    case
}

impl Case {
    /// Replace the store with `rows` of `(env, var, value)`, each value marked.
    fn store(&self, rows: &[(&str, &str, &str)]) {
        let c = Collection::open(&self.col).unwrap();
        let entries: Vec<Value> = rows
            .iter()
            .map(|(e, v, value)| {
                let key = c.secret_key(e, v);
                json!({
                    "collection": key.collection,
                    "environment": e,
                    "variable": v,
                    "value": format!("{MARK}-{value}"),
                })
            })
            .collect();
        fs::write(&self.secrets, serde_json::to_vec(&entries).unwrap()).unwrap();
    }

    /// What the store holds, as sorted `env/var=value` with the mark dropped.
    fn stored(&self) -> Vec<String> {
        let entries: Vec<Value> =
            serde_json::from_slice(&fs::read(&self.secrets).unwrap()).unwrap();
        let mut rows: Vec<String> = entries
            .iter()
            .map(|e| {
                let value = e["value"].as_str().unwrap();
                format!(
                    "{}/{}={}",
                    e["environment"].as_str().unwrap(),
                    e["variable"].as_str().unwrap(),
                    value.trim_start_matches(&format!("{MARK}-"))
                )
            })
            .collect();
        rows.sort();
        rows
    }

    /// `apinata env renames ARGS` in the collection, its exit code, stdout and stderr; no
    /// value may ever show on either stream.
    fn run(&self, args: &[&str]) -> (i32, String, String) {
        let out = cli(&self.sb, &self.col)
            .env("APINATA_TEST_SECRETS_FILE", &self.secrets)
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                "unix:path=/nonexistent/apinata-bus",
            )
            .env("XDG_RUNTIME_DIR", self.sb.path().join("xdg"))
            .args(["env", "renames"])
            .args(args)
            .output()
            .unwrap();
        let (stdout, stderr) = (
            String::from_utf8(out.stdout).unwrap(),
            String::from_utf8(out.stderr).unwrap(),
        );
        assert!(!stdout.contains(MARK), "a value on stdout: {stdout}");
        assert!(!stderr.contains(MARK), "a value on stderr: {stderr}");
        (out.status.code().unwrap(), stdout, stderr)
    }
}

const LISTED: &str = "dev / key → staging / key: taken\n\
                      dev / token → staging / token: ready\n\
                      qa / old → qa / a or qa / b: ambiguous\n";

#[test]
fn lists_ready_ambiguous_and_taken_renames_without_values() {
    let c = case();
    let (code, out, err) = c.run(&["--since", &c.base]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, LISTED);
    assert!(err.contains("--pick qa/old=qa/a"), "{err}");
    assert!(err.contains("--move"), "{err}");
}

#[test]
fn move_takes_the_ready_ones_and_skips_the_others() {
    let c = case();
    let (code, out, _) = c.run(&["--since", &c.base, "--move"]);
    assert_eq!(code, 1, "something was skipped");
    assert_eq!(
        out,
        "skipped dev / key → staging / key: taken\n\
         moved dev / token → staging / token\n\
         skipped qa / old → qa / a or qa / b: ambiguous\n"
    );
    assert_eq!(
        c.stored(),
        [
            "dev/key=dev-key",
            "qa/old=qa-old",
            "staging/key=staging-key",
            "staging/token=dev-token",
        ]
    );
    // The moved one is no longer listed, and nothing else changed.
    let (code, out, _) = c.run(&["--since", &c.base]);
    assert_eq!(code, 0);
    assert_eq!(
        out,
        "dev / key → staging / key: taken\n\
         qa / old → qa / a or qa / b: ambiguous\n"
    );
}

#[test]
fn pick_settles_an_ambiguous_one() {
    let c = case();
    let (code, out, err) = c.run(&["--since", &c.base, "--pick", "qa/old=qa/b"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        out,
        "moved qa / old → qa / b\n\
         dev / key → staging / key: taken\n\
         dev / token → staging / token: ready\n"
    );
    assert!(c.stored().contains(&"qa/b=qa-old".to_owned()));
    assert!(!c.stored().iter().any(|r| r.starts_with("qa/old=")));
}

#[test]
fn pick_with_move_leaves_only_the_taken_one() {
    let c = case();
    let (code, out, _) = c.run(&["--since", &c.base, "--move", "--pick", "qa/old=qa/a"]);
    assert_eq!(code, 1);
    assert_eq!(
        out,
        "moved qa / old → qa / a\n\
         skipped dev / key → staging / key: taken\n\
         moved dev / token → staging / token\n"
    );
    let (code, out, _) = c.run(&["--since", &c.base, "--move"]);
    assert_eq!(code, 1);
    assert_eq!(out, "skipped dev / key → staging / key: taken\n");
}

#[test]
fn a_pick_onto_a_taken_name_is_skipped_and_never_overwrites() {
    let c = case();
    let (code, out, _) = c.run(&["--since", &c.base, "--pick", "dev/key=staging/key"]);
    assert_eq!(code, 1);
    assert!(
        out.starts_with("skipped dev / key → staging / key: taken\n"),
        "{out}"
    );
    assert!(c.stored().contains(&"staging/key=staging-key".to_owned()));
    assert!(c.stored().contains(&"dev/key=dev-key".to_owned()));
}

#[test]
fn a_pick_that_names_no_pending_rename_is_a_usage_error() {
    let c = case();
    for pick in ["qa/old=qa/zzz", "qa/nope=qa/a", "qa-old"] {
        let (code, _, err) = c.run(&["--since", &c.base, "--pick", pick]);
        assert_eq!(code, 3, "{pick}: {err}");
    }
    assert_eq!(c.stored().len(), 4, "nothing moved");
}

#[test]
fn with_everything_moved_it_says_so_and_exits_zero() {
    let c = case();
    c.store(&[("dev", "token", "dev-token")]);
    let (code, out, _) = c.run(&["--since", &c.base, "--move"]);
    assert_eq!(code, 0);
    assert_eq!(out, "moved dev / token → staging / token\n");
    let (code, out, _) = c.run(&["--since", &c.base, "--move"]);
    assert_eq!(code, 0);
    assert_eq!(out, "no stored secrets to move\n");
}

#[test]
fn without_since_it_uses_orig_head_or_asks_for_since() {
    let c = case();
    let (code, _, err) = c.run(&[]);
    assert_eq!(code, 3);
    assert!(err.contains("--since"), "{err}");
    c.sb.git(&c.col, &["update-ref", "ORIG_HEAD", &c.base]);
    let (code, out, _) = c.run(&[]);
    assert_eq!(code, 0);
    assert_eq!(out, LISTED);
}

#[test]
fn an_unknown_since_is_a_usage_error() {
    let c = case();
    let (code, _, err) = c.run(&["--since", "no-such-commit"]);
    assert_eq!(code, 3);
    assert!(err.contains("no-such-commit"), "{err}");
}

#[test]
fn a_pull_that_renames_a_stored_secret_hints_at_env_renames() {
    let sb = Sandbox::new();
    let col = cloned(&sb);
    let seed = sb.path().join("seed");
    sb.git(&seed, &["switch", "-q", "main"]);
    write(&seed, "environments/dev.yml", &env("dev", &["token"]));
    sb.commit_all(&seed, "dev");
    sb.git(&seed, &["push", "-q", "origin", "main"]);
    let secrets = sb.path().join("secrets.json");
    let pull = |col: &Path| {
        cli(&sb, col)
            .env("APINATA_TEST_SECRETS_FILE", &secrets)
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                "unix:path=/nonexistent/apinata-bus",
            )
            .env("XDG_RUNTIME_DIR", sb.path().join("xdg"))
            .args(["git", "pull"])
            .assert()
            .code(0)
    };
    let first = pull(&col);
    assert!(!err_text(&first).contains("env renames"));
    let key = Collection::open(&col).unwrap().secret_key("dev", "token");
    let entry = json!([{
        "collection": key.collection,
        "environment": "dev",
        "variable": "token",
        "value": format!("{MARK}-dev-token"),
    }]);
    fs::write(&secrets, serde_json::to_vec(&entry).unwrap()).unwrap();
    let before = head(&sb, &col);
    fs::remove_file(seed.join("environments/dev.yml")).unwrap();
    write(
        &seed,
        "environments/staging.yml",
        &env("staging", &["token"]),
    );
    sb.commit_all(&seed, "rename");
    sb.git(&seed, &["push", "-q", "origin", "main"]);
    let second = pull(&col);
    let (out, err) = (out_text(&second), err_text(&second));
    assert!(out.starts_with("Fast-forwarded main"), "{out}");
    assert!(
        err.contains(&format!("apinata env renames --since {}", &before[..7])),
        "{err}"
    );
    assert!(!out.contains(MARK) && !err.contains(MARK));
}
