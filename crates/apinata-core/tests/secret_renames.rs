//! Renames between two commits that leave a stored secret under an old name.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

use crate::git_support::{Sandbox, write};
use crate::git_sync::rev;
use crate::team_rebase::edit;

use std::path::{Path, PathBuf};

use apinata_core::{
    Collection, EnvError, MemoryStore, PullRange, RenameStatus, SecretError, SecretKey, SecretName,
    SecretRename, SecretRenames, SecretStore, TargetState,
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

fn renamed_vars(sb: &Sandbox, old: &str, new: &str, keys: &[(&str, &str)]) -> SecretRenames {
    let (col, base) = repo(sb, &[("environments/dev.yml", old)]);
    if old.starts_with("name: dev") && new.starts_with("name: dev") {
        edit(sb, &col, &[("environments/dev.yml", Some(new))]);
    } else {
        edit(
            sb,
            &col,
            &[
                ("environments/dev.yml", None),
                ("environments/staging.yml", Some(new)),
            ],
        );
    }
    let c = Collection::open(&col).unwrap();
    scan(sb, &col, &base, &stored(&c, keys))
}

#[test]
fn an_environment_and_a_variable_renamed_together_compose() {
    let sb = Sandbox::new();
    let found = renamed_vars(
        &sb,
        &env("dev", &["token", "host_key"]),
        &env("staging", &["api_token", "host_key"]),
        &[("dev", "token"), ("dev", "host_key")],
    );
    assert_eq!(
        names(&found),
        [
            row("dev/host_key", &["staging/host_key"], true),
            row("dev/token", &["staging/api_token"], false),
        ]
    );
}

#[test]
fn a_variable_renamed_in_place_is_offered_but_not_sure() {
    let sb = Sandbox::new();
    let found = renamed_vars(
        &sb,
        &env("dev", &["token"]),
        &env("dev", &["api_token"]),
        &[("dev", "token")],
    );
    assert_eq!(names(&found), [row("dev/token", &["dev/api_token"], false)]);
}

#[test]
fn two_removed_and_two_added_offer_every_pairing() {
    let sb = Sandbox::new();
    let found = renamed_vars(
        &sb,
        &env("dev", &["a", "b"]),
        &env("dev", &["c", "d"]),
        &[("dev", "a"), ("dev", "b")],
    );
    assert_eq!(
        names(&found),
        [
            row("dev/a", &["dev/c", "dev/d"], false),
            row("dev/b", &["dev/c", "dev/d"], false)
        ]
    );
}

#[test]
fn a_secret_made_plain_is_not_offered() {
    let sb = Sandbox::new();
    let plain = "name: dev\nvariables:\n  - name: host\n    value: h.test\n  - name: token\n    value: plain\n";
    let found = renamed_vars(&sb, &env("dev", &["token"]), plain, &[("dev", "token")]);
    assert!(found.renames.is_empty());
}

#[test]
fn the_target_state_says_whether_a_value_is_already_there() {
    let sb = Sandbox::new();
    let (col, base) = repo(
        &sb,
        &[(
            "environments/dev.yml",
            env("dev", &["a", "b", "c"]).as_str(),
        )],
    );
    let staging = env("staging", &["a", "b", "c"]);
    edit(
        &sb,
        &col,
        &[
            ("environments/dev.yml", None),
            ("environments/staging.yml", Some(staging.as_str())),
        ],
    );
    let c = Collection::open(&col).unwrap();
    let store = MemoryStore::new();
    for var in ["a", "b", "c"] {
        store.set(&c.secret_key("dev", var), "v").unwrap();
    }
    store.set(&c.secret_key("staging", "b"), "v").unwrap();
    store.set(&c.secret_key("staging", "c"), "other").unwrap();
    let found = scan(&sb, &col, &base, &store);
    let states: Vec<TargetState> = found.renames.iter().map(|r| r.to[0].1).collect();
    assert_eq!(
        states,
        [TargetState::Free, TargetState::Same, TargetState::Taken]
    );
}

#[test]
fn a_target_the_current_file_no_longer_declares_is_dropped() {
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
    // An edit after the pull, not yet committed, drops the secret again.
    write(&col, "environments/staging.yml", &env("staging", &[]));
    let c = Collection::open(&col).unwrap();
    let found = scan(&sb, &col, &base, &stored(&c, &[("dev", "token")]));
    assert!(found.renames.is_empty());
}

#[test]
fn a_collection_opened_before_the_pull_still_sees_the_new_file() {
    let sb = Sandbox::new();
    let (col, base) = repo(
        &sb,
        &[("environments/dev.yml", env("dev", &["token"]).as_str())],
    );
    // The app's collection learns of pulled files only when its watcher reloads, which may
    // come after the ask; the new file is read from disk all the same.
    let c = Collection::open(&col).unwrap();
    let store = stored(&c, &[("dev", "token")]);
    let staging = env("staging", &["token"]);
    edit(
        &sb,
        &col,
        &[
            ("environments/dev.yml", None),
            ("environments/staging.yml", Some(staging.as_str())),
        ],
    );
    let range = PullRange::new(base, rev(&sb, &col, "HEAD"));
    let found = c
        .secret_renames(&range, None, &store, &sb.options())
        .unwrap();
    assert_eq!(names(&found), [row("dev/token", &["staging/token"], true)]);
}

#[test]
fn a_renamed_environment_whose_old_file_did_not_mark_the_variable_secret_offers_nothing() {
    let sb = Sandbox::new();
    let plain = "name: dev\nvariables:\n  - name: token\n    value: plain\n";
    let found = renamed_vars(&sb, plain, &env("staging", &["token"]), &[("dev", "token")]);
    assert!(found.renames.is_empty());
}

#[test]
fn a_renamed_environment_whose_old_file_does_not_parse_offers_nothing() {
    let sb = Sandbox::new();
    let found = renamed_vars(
        &sb,
        "name: [unclosed\n",
        &env("staging", &["token"]),
        &[("dev", "token")],
    );
    assert!(found.renames.is_empty());
}

#[test]
fn a_range_commit_shaped_like_an_option_is_never_read_as_one() {
    let sb = Sandbox::new();
    let (col, _) = repo(
        &sb,
        &[("environments/dev.yml", env("dev", &["token"]).as_str())],
    );
    let c = Collection::open(&col).unwrap();
    let planted = col.join("planted.txt");
    let range = PullRange::new(
        format!("--output={}", planted.display()),
        rev(&sb, &col, "HEAD"),
    );
    let store = stored(&c, &[("dev", "token")]);
    let found = c
        .secret_renames(&range, None, &store, &sb.options())
        .unwrap();
    assert_eq!(found, SecretRenames::default());
    assert!(!planted.exists());
}

#[test]
fn only_a_sure_row_with_one_free_or_same_target_is_ready() {
    let n = |s: &str| {
        let (e, v) = s.split_once('/').unwrap();
        SecretName::new(e, v)
    };
    let status = |to: &[(&str, TargetState)], sure: bool| {
        let to = to.iter().map(|(t, s)| (n(t), *s)).collect();
        SecretRename::new(n("dev/a"), to, sure).status()
    };
    use TargetState::{Free, Same, Taken};
    assert_eq!(status(&[("s/a", Free)], true), RenameStatus::Ready);
    assert_eq!(status(&[("s/a", Same)], true), RenameStatus::Ready);
    assert_eq!(status(&[("s/a", Taken)], true), RenameStatus::Taken);
    assert_eq!(status(&[("s/a", Taken)], false), RenameStatus::Taken);
    assert_eq!(status(&[("s/a", Free)], false), RenameStatus::Ambiguous);
    assert_eq!(
        status(&[("s/a", Free), ("s/b", Free)], true),
        RenameStatus::Ambiguous
    );
}

#[test]
fn a_range_to_head_resolves_both_ends_or_is_none() {
    let sb = Sandbox::new();
    let (col, base) = repo(
        &sb,
        &[("environments/dev.yml", env("dev", &["token"]).as_str())],
    );
    let key = env("dev", &["key"]);
    let tip = edit(&sb, &col, &[("environments/dev.yml", Some(key.as_str()))]);
    let o = sb.options();
    assert_eq!(
        PullRange::to_head(&col, &base[..7], &o).unwrap(),
        Some(PullRange::new(base.clone(), tip.clone()))
    );
    assert_eq!(PullRange::to_head(&col, "ORIG_HEAD", &o).unwrap(), None);
    assert_eq!(PullRange::to_head(&col, "--all", &o).unwrap(), None);
    sb.git(&col, &["update-ref", "ORIG_HEAD", &base]);
    assert_eq!(
        PullRange::to_head(&col, "ORIG_HEAD", &o).unwrap(),
        Some(PullRange::new(base, tip))
    );
}

#[test]
fn a_forward_range_needs_head_to_have_moved_on_from_the_seen_commit() {
    let sb = Sandbox::new();
    let (col, base) = repo(
        &sb,
        &[("environments/dev.yml", env("dev", &["token"]).as_str())],
    );
    let key = env("dev", &["key"]);
    let tip = edit(&sb, &col, &[("environments/dev.yml", Some(key.as_str()))]);
    let o = sb.options();
    assert_eq!(
        PullRange::forward(&col, &base, &o).unwrap(),
        Some(PullRange::new(base.clone(), tip.clone()))
    );
    assert_eq!(PullRange::forward(&col, &tip, &o).unwrap(), None);
    assert_eq!(PullRange::forward(&col, "--all", &o).unwrap(), None);
    // Back on the older commit, or on a line that does not hold the seen one, is not a pull.
    sb.git(&col, &["switch", "-q", "--detach", &base]);
    assert_eq!(PullRange::forward(&col, &tip, &o).unwrap(), None);
    sb.git(&col, &["switch", "-q", "--orphan", "other"]);
    write(&col, "x.txt", "x\n");
    sb.commit_all(&col, "unrelated");
    assert_eq!(PullRange::forward(&col, &base, &o).unwrap(), None);
}
