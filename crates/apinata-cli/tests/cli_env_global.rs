//! `apinata env --global`: the collection actions on the profile's global environments, with no
//! collection needed and secrets kept out of files and output.

use crate::common;

use apinata_core::{GlobalEnvironments, SecretKey};
use common::{ROOT, apinata, write_tree};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const SECRET: &str = "sentinel-gl0bal-s3cret";

/// A profile directory, a secrets file outside it, and an empty working directory.
struct Scratch {
    profile: TempDir,
    state: TempDir,
    cwd: TempDir,
}

impl Scratch {
    fn new() -> Scratch {
        Scratch {
            profile: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            cwd: TempDir::new().unwrap(),
        }
    }

    fn dir(&self) -> PathBuf {
        self.profile.path().join("global-environments")
    }

    fn write(&self, name: &str, body: &str) {
        std::fs::create_dir_all(self.dir()).unwrap();
        std::fs::write(self.dir().join(format!("{name}.yml")), body).unwrap();
    }

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.dir().join(format!("{name}.yml"))).unwrap()
    }

    /// The exit code, stdout and stderr of `apinata ARGS` with `stdin` piped in.
    fn run_with(&self, args: &[&str], stdin: &str) -> (i32, String, String) {
        let out = apinata()
            .env("APINATA_PROFILE_DIR", self.profile.path())
            .env(
                "APINATA_TEST_SECRETS_FILE",
                self.state.path().join("s.json"),
            )
            .env("XDG_CONFIG_HOME", self.state.path().join("config"))
            .env("XDG_DATA_HOME", self.state.path().join("data"))
            .current_dir(self.cwd.path())
            .args(args)
            .write_stdin(stdin)
            .output()
            .unwrap();
        (
            out.status.code().unwrap(),
            String::from_utf8(out.stdout).unwrap(),
            String::from_utf8(out.stderr).unwrap(),
        )
    }

    fn run(&self, args: &[&str]) -> (i32, String, String) {
        self.run_with(args, "")
    }

    fn key(&self, env: &str, var: &str) -> SecretKey {
        GlobalEnvironments::open(self.dir())
            .unwrap()
            .secret_key(env, var)
    }

    /// Every stored value, as the debug file store keeps them.
    fn stored(&self) -> String {
        std::fs::read_to_string(self.state.path().join("s.json")).unwrap_or_default()
    }

    fn has_secret(&self, env: &str, var: &str) -> bool {
        let key = self.key(env, var);
        let entries: Value = serde_json::from_str(&self.stored()).unwrap_or(Value::Null);
        let dir = key.collection.to_string_lossy().into_owned();
        entries.as_array().into_iter().flatten().any(|e| {
            e["collection"] == dir.as_str() && e["environment"] == env && e["variable"] == var
        })
    }
}

#[test]
fn new_ls_set_and_show_work_with_no_collection() {
    let s = Scratch::new();
    let (code, out, err) = s.run(&["env", "new", "shared", "--global"]);
    assert_eq!((code, out.as_str()), (0, "created shared\n"), "{err}");
    let (code, out, _) = s.run(&["env", "new", "shared", "--global", "--json"]);
    assert_eq!(code, 0);
    let json: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(json["name"], "shared copy");

    let (code, out, err) = s.run(&["env", "ls", "--global"]);
    assert_eq!((code, out.as_str()), (0, "shared\nshared copy\n"), "{err}");

    let (code, out, err) = s.run(&["env", "set", "shared", "host=example.test", "--global"]);
    assert_eq!((code, out.as_str()), (0, "set host in shared\n"), "{err}");
    let before = s.read("shared");
    let (code, out, _) = s.run(&["env", "set", "shared", "host=example.test", "--global"]);
    assert_eq!((code, out.as_str()), (0, "unchanged: host in shared\n"));
    assert_eq!(s.read("shared"), before);

    let (code, out, err) = s.run(&["env", "show", "shared", "--global", "--json"]);
    assert_eq!(code, 0, "{err}");
    let json: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(json["variables"][0]["value"], "example.test");
}

#[test]
fn a_secret_is_stored_never_written_or_printed_and_unset_removes_it() {
    let s = Scratch::new();
    s.write("shared", "name: shared\n");
    let args = ["env", "set", "--secret", "shared", "tok", "--global"];
    let (code, out, err) = s.run_with(&args, &format!("{SECRET}\n"));
    assert_eq!(
        (code, out.as_str()),
        (0, "set secret tok in shared\n"),
        "{err}"
    );
    assert!(!s.read("shared").contains(SECRET));
    assert!(s.has_secret("shared", "tok"));

    let (code, out, _) = s.run(&["env", "show", "shared", "--global"]);
    assert_eq!(code, 0);
    assert!(out.contains("<secret>") && !out.contains(SECRET), "{out}");

    let (code, _, err) = s.run(&["env", "unset", "shared", "tok", "--global"]);
    assert_eq!(code, 3, "a secret needs --secret: {err}");
    let (code, out, err) = s.run(&["env", "unset", "shared", "tok", "--secret", "--global"]);
    assert_eq!(
        (code, out.as_str()),
        (0, "removed tok from shared\n"),
        "{err}"
    );
    assert!(!s.has_secret("shared", "tok"));
}

#[test]
fn cp_mv_and_rm_carry_and_remove_stored_secrets() {
    let s = Scratch::new();
    s.write("base", "name: base\n");
    s.write("dev", "name: dev\nextends: base\n");
    let args = ["env", "set", "--secret", "base", "tok", "--global"];
    assert_eq!(s.run_with(&args, SECRET).0, 0);

    let (code, out, err) = s.run(&["env", "cp", "base", "--global"]);
    assert_eq!(
        (code, out.as_str()),
        (0, "copied base to base copy\n"),
        "{err}"
    );
    assert!(s.has_secret("base copy", "tok"));

    let (code, out, err) = s.run(&["env", "mv", "base", "root", "--global"]);
    assert_eq!((code, out.as_str()), (0, "renamed base to root\n"), "{err}");
    assert!(s.read("dev").contains("extends: root"));
    assert!(s.has_secret("root", "tok") && !s.has_secret("base", "tok"));

    let (code, _, err) = s.run(&["env", "rm", "root", "--global"]);
    assert_eq!(code, 3, "{err}");
    assert!(err.contains("`dev`"), "{err}");
    let (code, out, err) = s.run(&["env", "rm", "base copy", "--global"]);
    assert_eq!((code, out.as_str()), (0, "removed base copy\n"), "{err}");
    assert!(!s.dir().join("base copy.yml").exists());
    assert!(!s.has_secret("base copy", "tok"));
}

#[test]
fn an_unknown_global_environment_is_exit_3_naming_the_global_set() {
    let s = Scratch::new();
    s.write("shared", "name: shared\n");
    for args in [
        &["env", "show", "nope", "--global"][..],
        &["env", "set", "nope", "a=b", "--global"][..],
        &["env", "unset", "nope", "a", "--global"][..],
        &["env", "cp", "nope", "--global"][..],
        &["env", "mv", "nope", "x", "--global"][..],
        &["env", "rm", "nope", "--global"][..],
    ] {
        let (code, _, err) = s.run(args);
        assert_eq!(code, 3, "{args:?}: {err}");
        assert!(err.contains("global environment"), "{args:?}: {err}");
    }
}

#[test]
fn global_with_collection_is_a_usage_error_for_every_action() {
    let s = Scratch::new();
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let col = col.path().to_str().unwrap();
    for action in [
        &["ls"][..],
        &["show", "a"][..],
        &["set", "a", "k=v"][..],
        &["unset", "a", "k"][..],
        &["new", "a"][..],
        &["cp", "a"][..],
        &["mv", "a", "b"][..],
        &["rm", "a"][..],
        &["orphans", "a"][..],
        &["forget", "a", "--all"][..],
    ] {
        let mut args = vec!["env"];
        args.extend_from_slice(action);
        args.extend_from_slice(&["--global", "--collection", col]);
        let (code, _, err) = s.run(&args);
        assert_eq!(code, 3, "{args:?}: {err}");
        assert!(err.contains("--collection"), "{args:?}: {err}");
    }
    assert!(!s.dir().exists(), "nothing was created");
}

#[test]
fn a_collection_environment_is_untouched_by_a_global_edit() {
    let s = Scratch::new();
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/shared.yml", "name: shared\n"),
    ]);
    s.write("shared", "name: shared\n");
    let out = apinata()
        .env("APINATA_PROFILE_DIR", s.profile.path())
        .current_dir(col.path())
        .args(["env", "set", "shared", "a=b", "--global"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(s.read("shared").contains("name: a"));
    let local = Path::new(col.path()).join("environments/shared.yml");
    assert_eq!(std::fs::read_to_string(local).unwrap(), "name: shared\n");
}
