//! Renames between two commits that leave a stored secret under an old name.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

use crate::git_support::{Sandbox, write};
use crate::git_sync::rev;
use crate::team_rebase::edit;

use std::path::{Path, PathBuf};

use apinata_core::{
    Collection, EnvError, MemoryStore, PullRange, SecretError, SecretKey, SecretName,
    SecretRenames, SecretStore, TargetState,
};

/// An environment file `name` with a plain `host` and each of `secrets` marked secret.
pub fn env(name: &str, secrets: &[&str]) -> String {
    env_with_host(name, secrets, "h.test")
}

/// Like [`env`], with `host` set to `host`.
pub fn env_with_host(name: &str, secrets: &[&str], host: &str) -> String {
    let mut text = format!("name: {name}\nvariables:\n  - name: host\n    value: {host}\n");
    for v in secrets {
        text.push_str(&format!("  - name: {v}\n    secret: true\n"));
    }
    text
}

/// Each row as `env/var`, its candidates as `env/var`, and whether it is sure.
pub fn names(found: &SecretRenames) -> Vec<(String, Vec<String>, bool)> {
    let n = |s: &SecretName| format!("{}/{}", s.environment, s.variable);
    let row = |r: &apinata_core::SecretRename| {
        (n(&r.from), r.to.iter().map(|(t, _)| n(t)).collect(), r.sure)
    };
    found.renames.iter().map(row).collect()
}

/// One expected row in the shape [`names`] returns.
pub fn row(from: &str, to: &[&str], sure: bool) -> (String, Vec<String>, bool) {
    (
        from.to_owned(),
        to.iter().map(|s| s.to_string()).collect(),
        sure,
    )
}

/// A store holding `<env>-<var>` under each of `keys` of `c`.
pub fn stored(c: &Collection, keys: &[(&str, &str)]) -> MemoryStore {
    let store = MemoryStore::new();
    for (env, var) in keys {
        store
            .set(&c.secret_key(env, var), &format!("{env}-{var}"))
            .unwrap();
    }
    store
}

/// A repository `col` whose first commit holds `files`; returns it and that commit.
fn repo(sb: &Sandbox, files: &[(&str, &str)]) -> (PathBuf, String) {
    let col = sb.repo("col");
    write(
        &col,
        "opencollection.yml",
        "opencollection: 1.0.0\ninfo:\n  name: c\n",
    );
    for (rel, text) in files {
        write(&col, rel, text);
    }
    sb.commit_all(&col, "base");
    let base = rev(sb, &col, "HEAD");
    (col, base)
}

/// The renames between `base` and HEAD, with no copies folder.
fn scan(sb: &Sandbox, col: &Path, base: &str, store: &dyn SecretStore) -> SecretRenames {
    let c = Collection::open(col).unwrap();
    let range = PullRange::new(base, rev(sb, col, "HEAD"));
    c.secret_renames(&range, None, store, &sb.options())
        .unwrap()
}

/// A store that cannot list its keys.
struct NoList(MemoryStore);

impl SecretStore for NoList {
    fn get(&self, key: &SecretKey) -> Result<Option<String>, SecretError> {
        self.0.get(key)
    }
    fn set(&self, key: &SecretKey, value: &str) -> Result<(), SecretError> {
        self.0.set(key, value)
    }
    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        self.0.delete(key)
    }
}

fn renamed_dev(sb: &Sandbox, keys: &[(&str, &str)]) -> SecretRenames {
    let (col, base) = repo(
        sb,
        &[(
            "environments/dev.yml",
            env("dev", &["token", "host_key"]).as_str(),
        )],
    );
    let staging = env("staging", &["token", "host_key"]);
    edit(
        sb,
        &col,
        &[
            ("environments/dev.yml", None),
            ("environments/staging.yml", Some(staging.as_str())),
        ],
    );
    let c = Collection::open(&col).unwrap();
    scan(sb, &col, &base, &stored(&c, keys))
}

#[test]
fn a_renamed_environment_offers_each_stored_secret_as_a_sure_row() {
    let sb = Sandbox::new();
    let found = renamed_dev(&sb, &[("dev", "token"), ("dev", "host_key")]);
    assert_eq!(
        names(&found),
        [
            row("dev/host_key", &["staging/host_key"], true),
            row("dev/token", &["staging/token"], true),
        ]
    );
    assert_eq!(found.held_back, 0);
    assert!(found.renames.iter().all(|r| r.to[0].1 == TargetState::Free));
}

#[test]
fn only_values_in_the_store_are_offered() {
    let sb = Sandbox::new();
    let found = renamed_dev(&sb, &[("dev", "token")]);
    assert_eq!(names(&found), [row("dev/token", &["staging/token"], true)]);
}

#[test]
fn one_delete_with_two_identical_adds_offers_both_and_is_not_sure() {
    let sb = Sandbox::new();
    let (col, base) = repo(
        &sb,
        &[("environments/prod.yml", env("prod", &["t"]).as_str())],
    );
    let same = env("prod", &["t"]);
    edit(
        &sb,
        &col,
        &[
            ("environments/prod.yml", None),
            ("environments/aaa.yml", Some(same.as_str())),
            ("environments/production.yml", Some(same.as_str())),
        ],
    );
    let c = Collection::open(&col).unwrap();
    let found = scan(&sb, &col, &base, &stored(&c, &[("prod", "t")]));
    // git pairs prod with aaa (R100); that partner is listed first.
    assert_eq!(
        names(&found),
        [row("prod/t", &["aaa/t", "production/t"], false)]
    );
}

#[test]
fn a_delete_and_an_unrelated_add_are_offered_but_not_sure() {
    let sb = Sandbox::new();
    let (col, base) = repo(
        &sb,
        &[("environments/prod.yml", env("prod", &["t"]).as_str())],
    );
    let mut qa = String::from("name: qa\nvariables:\n");
    for i in 1..=8 {
        qa.push_str(&format!("  - name: v{i}\n    value: q{i}\n"));
    }
    qa.push_str("  - name: t\n    secret: true\n");
    edit(
        &sb,
        &col,
        &[
            ("environments/prod.yml", None),
            ("environments/qa.yml", Some(qa.as_str())),
        ],
    );
    let c = Collection::open(&col).unwrap();
    let found = scan(&sb, &col, &base, &stored(&c, &[("prod", "t")]));
    assert_eq!(names(&found), [row("prod/t", &["qa/t"], false)]);
}

#[test]
fn a_copied_environment_offers_nothing() {
    let sb = Sandbox::new();
    let (col, base) = repo(
        &sb,
        &[("environments/dev.yml", env("dev", &["token"]).as_str())],
    );
    let copy = env("staging", &["token"]);
    edit(
        &sb,
        &col,
        &[("environments/staging.yml", Some(copy.as_str()))],
    );
    let c = Collection::open(&col).unwrap();
    let found = scan(&sb, &col, &base, &stored(&c, &[("dev", "token")]));
    assert_eq!(found, SecretRenames::default());
}

#[test]
fn a_rename_undone_within_the_range_offers_nothing() {
    let sb = Sandbox::new();
    let (col, base) = repo(
        &sb,
        &[("environments/dev.yml", env("dev", &["token"]).as_str())],
    );
    let (dev, staging) = (env("dev", &["token"]), env("staging", &["token"]));
    edit(
        &sb,
        &col,
        &[
            ("environments/dev.yml", None),
            ("environments/staging.yml", Some(staging.as_str())),
        ],
    );
    edit(
        &sb,
        &col,
        &[
            ("environments/staging.yml", None),
            ("environments/dev.yml", Some(dev.as_str())),
        ],
    );
    let c = Collection::open(&col).unwrap();
    let found = scan(&sb, &col, &base, &stored(&c, &[("dev", "token")]));
    assert!(found.renames.is_empty());
}

#[test]
fn an_unparsable_new_file_offers_nothing() {
    let sb = Sandbox::new();
    let (col, base) = repo(
        &sb,
        &[("environments/dev.yml", env("dev", &["token"]).as_str())],
    );
    edit(
        &sb,
        &col,
        &[
            ("environments/dev.yml", None),
            ("environments/staging.yml", Some("name: [unclosed\n")),
        ],
    );
    let c = Collection::open(&col).unwrap();
    let found = scan(&sb, &col, &base, &stored(&c, &[("dev", "token")]));
    assert!(found.renames.is_empty());
}

#[test]
fn a_commit_missing_from_the_repository_offers_nothing() {
    let sb = Sandbox::new();
    let (col, _) = repo(
        &sb,
        &[("environments/dev.yml", env("dev", &["token"]).as_str())],
    );
    let c = Collection::open(&col).unwrap();
    let range = PullRange::new(
        "1111111111111111111111111111111111111111",
        rev(&sb, &col, "HEAD"),
    );
    let store = stored(&c, &[("dev", "token")]);
    let found = c
        .secret_renames(&range, None, &store, &sb.options())
        .unwrap();
    assert_eq!(found, SecretRenames::default());
}

#[test]
fn a_store_that_cannot_list_fails_the_scan() {
    let sb = Sandbox::new();
    let (col, base) = repo(
        &sb,
        &[("environments/dev.yml", env("dev", &["token"]).as_str())],
    );
    let staging = env("staging", &["token"]);
    edit(
        &sb,
        &col,
        &[
            ("environments/dev.yml", None),
            ("environments/staging.yml", Some(staging.as_str())),
        ],
    );
    let c = Collection::open(&col).unwrap();
    let store = NoList(MemoryStore::new());
    store.set(&c.secret_key("dev", "token"), "v").unwrap();
    let range = PullRange::new(base, rev(&sb, &col, "HEAD"));
    let got = c.secret_renames(&range, None, &store, &sb.options());
    assert!(
        matches!(got, Err(EnvError::SecretListRoot { .. })),
        "{got:?}"
    );
}
