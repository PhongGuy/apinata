//! `apinata trust`: the per-user trust list from the command line, in a temporary trust file.

use crate::common;

use apinata_core::TrustStore;
use common::{ROOT, apinata, write_tree};
use predicates::str::contains;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

struct Fx {
    coll: TempDir,
    state: TempDir,
}

impl Fx {
    fn new() -> Fx {
        Fx {
            coll: write_tree(&[("opencollection.yml", ROOT), ("users/a.yml", "x")]),
            state: TempDir::new().unwrap(),
        }
    }
    fn file(&self) -> PathBuf {
        self.state.path().join("trusted-hosts.json")
    }
    fn root(&self) -> PathBuf {
        fs::canonicalize(self.coll.path()).unwrap()
    }
    fn store(&self) -> TrustStore {
        TrustStore::load(self.file()).unwrap()
    }
    fn trust(&self, args: &[&str]) -> assert_cmd::assert::Assert {
        apinata()
            .arg("trust")
            .args(args)
            .arg("--collection")
            .arg(self.coll.path())
            .arg("--trust-file")
            .arg(self.file())
            .assert()
    }
    fn list(&self) -> String {
        let out = self.trust(&["list"]).code(0).get_output().stdout.clone();
        String::from_utf8(out).unwrap()
    }
}

#[test]
fn add_trusts_a_normalised_host_and_list_shows_it() {
    let fx = Fx::new();
    fx.trust(&["add", "API.Example.com."])
        .code(0)
        .stdout(contains("trusted api.example.com"));
    assert!(fx.store().is_trusted(&fx.root(), "api.example.com"));
    assert_eq!(fx.list(), "trusted api.example.com\n");
}

#[test]
fn deny_moves_a_host_to_the_denied_list() {
    let fx = Fx::new();
    fx.trust(&["add", "api.example.com"]).code(0);
    fx.trust(&["deny", "api.example.com"])
        .code(0)
        .stdout(contains("denied api.example.com"));
    assert!(fx.store().is_denied(&fx.root(), "api.example.com"));
    assert!(!fx.store().is_trusted(&fx.root(), "api.example.com"));
    assert_eq!(fx.list(), "denied api.example.com\n");
}

#[test]
fn remove_takes_a_host_off_either_list() {
    let fx = Fx::new();
    fx.trust(&["add", "a.example"]).code(0);
    fx.trust(&["deny", "b.example"]).code(0);
    assert_eq!(fx.list(), "trusted a.example\ndenied b.example\n");
    fx.trust(&["remove", "a.example"])
        .code(0)
        .stdout(contains("removed a.example"));
    fx.trust(&["remove", "B.example"])
        .code(0)
        .stdout(contains("removed b.example"));
    fx.trust(&["remove", "b.example"])
        .code(0)
        .stdout(contains("was not listed"));
    assert_eq!(fx.list(), "");
}

#[test]
fn the_collection_is_found_from_a_subdirectory() {
    let fx = Fx::new();
    apinata()
        .current_dir(fx.coll.path().join("users"))
        .args(["trust", "add", "api.example.com", "--trust-file"])
        .arg(fx.file())
        .assert()
        .code(0);
    assert_eq!(fx.store().trusted(&fx.root()), ["api.example.com"]);
}

#[test]
fn other_collections_entries_survive_a_write() {
    let fx = Fx::new();
    let other = TempDir::new().unwrap();
    let mut s = TrustStore::empty(fx.file());
    s.trust(other.path(), "elsewhere.example").unwrap();
    s.save().unwrap();
    fx.trust(&["add", "api.example.com"]).code(0);
    assert!(fx.store().is_trusted(other.path(), "elsewhere.example"));
}

#[test]
fn errors_exit_3_and_write_nothing() {
    let fx = Fx::new();
    fx.trust(&["add", "exa mple"])
        .code(3)
        .stderr(contains("not a host"));
    assert!(!fx.file().exists());
    let bare = TempDir::new().unwrap();
    apinata()
        .args(["trust", "list", "--collection"])
        .arg(bare.path())
        .arg("--trust-file")
        .arg(fx.file())
        .assert()
        .code(3)
        .stderr(contains("opencollection.yml"));
    fs::write(fx.file(), "{ broken").unwrap();
    fx.trust(&["add", "api.example.com"]).code(3);
    fx.trust(&["list"]).code(3);
    assert_eq!(fs::read_to_string(fx.file()).unwrap(), "{ broken");
}

#[test]
fn usage_errors_exit_3_and_help_hides_the_test_seam() {
    apinata().arg("trust").assert().code(3);
    apinata().args(["trust", "add"]).assert().code(3);
    let help = apinata().args(["trust", "--help"]).assert().code(0);
    let text = String::from_utf8(help.get_output().stdout.clone()).unwrap();
    for word in ["add", "deny", "remove", "list", "--collection"] {
        assert!(text.contains(word), "{word} in {text}");
    }
    assert!(!text.contains("--trust-file"), "{text}");
}

#[test]
fn a_url_or_authority_never_trusts_a_different_host() {
    let fx = Fx::new();
    for input in [
        "http://good.example",
        "good.example@evil.example",
        "evil.example#.good.example",
        "evil.example/.good",
        "ex\tample.com",
    ] {
        fx.trust(&["add", input])
            .code(3)
            .stderr(contains("bare host"));
        fx.trust(&["deny", input]).code(3);
        assert!(!fx.file().exists(), "{input:?} wrote the trust file");
    }
}

#[test]
fn control_characters_never_reach_the_terminal() {
    let fx = Fx::new();
    for input in ["evil\u{1b}[2J.example", "a.example\u{202e}"] {
        let out = fx.trust(&["add", input]).code(3).stderr(contains("host"));
        let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
        assert!(!stderr.contains(['\u{1b}', '\u{202e}']), "{stderr:?}");
    }
    let coll = write_tree(&[("c\u{1b}[2J/opencollection.yml", ROOT)]);
    let out = apinata()
        .args(["trust", "add", "api.example.com", "--collection"])
        .arg(coll.path().join("c\u{1b}[2J"))
        .arg("--trust-file")
        .arg(fx.file())
        .assert()
        .code(0)
        .stdout(contains("trusted api.example.com for"));
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(!stdout.contains('\u{1b}'), "{stdout:?}");
}

#[test]
fn tag_characters_never_reach_the_terminal() {
    let fx = Fx::new();
    let out = fx.trust(&["add", "a.example\u{e0041}"]).code(3);
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(!stderr.contains('\u{e0041}'), "{stderr:?}");
}
