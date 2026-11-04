//! `apinata trust scripts`: print unapproved script texts and approve them on confirmation.

use crate::common;

use apinata_core::{ScriptDecision, ScriptTrust, script_hash};
use common::{ROOT, apinata, write_tree};
use predicates::prelude::*;
use predicates::str::contains;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

const COLL: &str = "bru.setVar('c', 1)";
const TESTS: &str = "test('a', () => {})";

struct Fx {
    coll: TempDir,
    state: TempDir,
}

impl Fx {
    fn new() -> Fx {
        let root = format!(
            "{ROOT}request:\n  scripts:\n    - type: before-request\n      code: \"{COLL}\"\n"
        );
        let r = format!(
            "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://127.0.0.1:9/\"\nruntime:\n  scripts:\n    - type: tests\n      code: \"{TESTS}\"\n"
        );
        Fx {
            coll: write_tree(&[("opencollection.yml", &root), ("r.yml", &r)]),
            state: TempDir::new().unwrap(),
        }
    }
    fn file(&self) -> PathBuf {
        self.state.path().join("script-trust.json")
    }
    fn root(&self) -> PathBuf {
        fs::canonicalize(self.coll.path()).unwrap()
    }
    fn scripts(&self, stdin: &str, extra: &[&str]) -> assert_cmd::assert::Assert {
        apinata()
            .args(["trust", "scripts"])
            .args(extra)
            .arg("--collection")
            .arg(self.coll.path())
            .arg("--script-trust-file")
            .arg(self.file())
            .write_stdin(stdin)
            .assert()
    }
}

#[test]
fn unapproved_texts_are_printed_and_approved_on_yes() {
    let fx = Fx::new();
    fx.scripts("y\n", &[])
        .code(0)
        .stdout(contains("r.yml: pre-request script (collection)"))
        .stdout(contains(format!("    {COLL}")))
        .stdout(contains("r.yml: tests script (request)"))
        .stdout(contains(format!("    {TESTS}")))
        .stdout(contains("Approve 2 scripts and allow scripts for"))
        .stdout(contains("approved 2 scripts for"));
    let trust = ScriptTrust::load(fx.file()).unwrap();
    assert_eq!(trust.decision(&fx.root()), Some(ScriptDecision::Allowed));
    assert!(trust.is_approved(&fx.root(), &script_hash(COLL)));
    assert!(trust.is_approved(&fx.root(), &script_hash(TESTS)));
}

#[test]
fn answering_no_changes_nothing() {
    let fx = Fx::new();
    fx.scripts("n\n", &[])
        .code(0)
        .stdout(contains("nothing was approved"));
    assert!(!fx.file().exists());
}

#[test]
fn yes_skips_the_question_and_a_second_run_finds_nothing() {
    let fx = Fx::new();
    fx.scripts("", &["--yes"])
        .code(0)
        .stdout(contains("approved 2 scripts"));
    fx.scripts("", &[])
        .code(0)
        .stdout(contains("no scripts need approval for"));
}

#[test]
fn an_unreadable_trust_file_is_an_error_and_is_kept() {
    let fx = Fx::new();
    fs::write(fx.file(), "{ broken").unwrap();
    fx.scripts("y\n", &[]).code(3).stderr(contains("error:"));
    assert_eq!(fs::read_to_string(fx.file()).unwrap(), "{ broken");
}

#[test]
fn a_dotenv_value_in_a_script_is_masked() {
    let fx = Fx::new();
    fs::write(fx.coll.path().join(".env"), "TOKEN=hunter2-dotenv\n").unwrap();
    let r = "info:\n  name: s\n  type: http\n  seq: 2\nhttp:\n  method: GET\n  url: \"http://127.0.0.1:9/\"\nruntime:\n  scripts:\n    - type: tests\n      code: \"const t = 'hunter2-dotenv'\"\n";
    fs::write(fx.coll.path().join("s.yml"), r).unwrap();
    fx.scripts("n\n", &[])
        .code(0)
        .stdout(contains("    const t = '<secret>'"))
        .stdout(predicates::str::contains("hunter2-dotenv").not());
}

#[test]
fn one_script_is_counted_in_the_singular() {
    let fx = Fx::new();
    let mut trust = ScriptTrust::empty(fx.file());
    trust.approve(&fx.root(), [script_hash(COLL)]);
    trust.save().unwrap();
    fx.scripts("y\n", &[])
        .code(0)
        .stdout(contains("Approve 1 script and allow scripts for"))
        .stdout(contains("approved 1 script for"));
}

#[test]
fn an_approval_keeps_a_trust_change_made_while_the_question_waits() {
    use std::io::{Read, Write};
    use std::process::Stdio;
    let fx = Fx::new();
    let mut child = common::apinata_std()
        .args(["trust", "scripts", "--collection"])
        .arg(fx.coll.path())
        .arg("--script-trust-file")
        .arg(fx.file())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    // Wait for the question, so the change below lands after the file was first read.
    let mut stdout = child.stdout.take().unwrap();
    let mut seen = Vec::new();
    let mut byte = [0u8; 1];
    while !String::from_utf8_lossy(&seen).contains("[y/N] ") {
        assert_eq!(stdout.read(&mut byte).unwrap(), 1, "no question asked");
        seen.push(byte[0]);
    }
    let other = TempDir::new().unwrap();
    let other_root = fs::canonicalize(other.path()).unwrap();
    let mut trust = ScriptTrust::load(fx.file()).unwrap();
    trust.approve(&other_root, [script_hash("app-approved")]);
    trust.save().unwrap();
    child.stdin.take().unwrap().write_all(b"y\n").unwrap();
    assert!(child.wait().unwrap().success());
    let trust = ScriptTrust::load(fx.file()).unwrap();
    assert!(trust.is_approved(&other_root, &script_hash("app-approved")));
    assert!(trust.is_approved(&fx.root(), &script_hash(COLL)));
    assert!(trust.is_approved(&fx.root(), &script_hash(TESTS)));
}
