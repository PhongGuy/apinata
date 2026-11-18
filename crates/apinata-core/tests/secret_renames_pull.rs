//! Pulled renames: a real `rebase_pull`, moving the offer, and holding it back during a clash.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

use crate::common::at;
use crate::git_branch::cloned;
use crate::git_support::{Sandbox, write};
use crate::git_sync::rev;
use crate::secret_renames::{env, env_with_host, names, row};
use crate::team_rebase::{edit, teammate};

use std::path::Path;
use std::sync::Arc;

use apinata_core::team::ConflictCopies;
use apinata_core::{
    CancelFlag, Collection, ContextOptions, MemoryStore, PullRange, RebasePulled, SecretStore,
    VarContext, rebase_pull,
};

const REQUEST: &str = "info:\n  name: r\n  type: http\nhttp:\n  method: get\n  url: \"https://x.test/{{api_token}}/{{host_key}}\"\n";

/// Pull `col`; the range from HEAD before to HEAD after.
fn pulled(sb: &Sandbox, col: &Path, copies: &Path) -> (PullRange, RebasePulled) {
    let from = rev(sb, col, "HEAD");
    let got = rebase_pull(col, copies, &CancelFlag::new(), &sb.options()).unwrap();
    (PullRange::new(from, rev(sb, col, "HEAD")), got)
}

/// A clone holding `files` from the teammate, with `dev/token` stored.
fn base(sb: &Sandbox, files: &[(&str, Option<&str>)]) -> (std::path::PathBuf, Arc<MemoryStore>) {
    let col = cloned(sb);
    teammate(sb, files);
    sb.git(&col, &["pull", "-q", "--ff-only"]);
    let c = Collection::open(&col).unwrap();
    let store = Arc::new(MemoryStore::new());
    store.set(&c.secret_key("dev", "token"), "t1").unwrap();
    (col, store)
}

#[test]
fn a_pulled_rename_is_offered_and_moving_it_lets_requests_resolve() {
    let sb = Sandbox::new();
    let dev = env("dev", &["token", "host_key"]);
    let (col, store) = base(
        &sb,
        &[
            ("environments/dev.yml", Some(dev.as_str())),
            ("r.yml", Some(REQUEST)),
        ],
    );
    let c = Collection::open(&col).unwrap();
    store.set(&c.secret_key("dev", "host_key"), "k1").unwrap();
    let staging = env("staging", &["api_token", "host_key"]);
    teammate(
        &sb,
        &[
            ("environments/dev.yml", None),
            ("environments/staging.yml", Some(staging.as_str())),
        ],
    );
    let (range, _) = pulled(&sb, &col, &sb.path().join("copies"));
    let c = Collection::open(&col).unwrap();
    let found = c
        .secret_renames(&range, None, &*store, &sb.options())
        .unwrap();
    assert_eq!(
        names(&found),
        [
            row("dev/host_key", &["staging/host_key"], true),
            row("dev/token", &["staging/api_token"], false)
        ]
    );
    for r in &found.renames {
        c.move_renamed_secret(&r.from, &r.to[0].0, &*store).unwrap();
    }
    let ctx = VarContext::new(
        &c,
        ContextOptions::new().env("staging").secrets(store.clone()),
    )
    .unwrap();
    assert_eq!(at(&c, &ctx, "r.yml").unwrap().url(), "https://x.test/t1/k1");
    assert_eq!(store.get(&c.secret_key("dev", "token")).unwrap(), None);
    assert_eq!(store.get(&c.secret_key("dev", "host_key")).unwrap(), None);
}

#[test]
fn a_rename_that_conflicted_is_held_back_until_its_copy_is_settled() {
    let sb = Sandbox::new();
    let dev = env("dev", &["token"]);
    let (col, store) = base(&sb, &[("environments/dev.yml", Some(dev.as_str()))]);
    let mine = env_with_host("dev", &["token"], "mine");
    edit(&sb, &col, &[("environments/dev.yml", Some(mine.as_str()))]);
    let theirs = env_with_host("staging", &["token"], "theirs");
    teammate(
        &sb,
        &[
            ("environments/dev.yml", None),
            ("environments/staging.yml", Some(theirs.as_str())),
        ],
    );
    let dir = sb.path().join("copies");
    let (range, got) = pulled(&sb, &col, &dir);
    assert!(matches!(got, RebasePulled::Conflicted { .. }), "{got:?}");
    let copies = ConflictCopies::new(dir);
    assert_eq!(copies.list().unwrap(), ["environments/dev.yml"]);
    let c = Collection::open(&col).unwrap();
    // Asked again with the clash open, it stays held: the answer is re-derived each time.
    for _ in 0..2 {
        let held = c
            .secret_renames(&range, Some(&copies), &*store, &sb.options())
            .unwrap();
        assert!(held.renames.is_empty());
        assert_eq!(held.held_back, 1);
    }
    copies.remove("environments/dev.yml").unwrap();
    let found = c
        .secret_renames(&range, Some(&copies), &*store, &sb.options())
        .unwrap();
    assert_eq!(found.held_back, 0);
    assert_eq!(found.renames.len(), 1);
    assert_eq!(found.renames[0].to[0].0.environment, "staging");
}

#[test]
fn a_local_delete_mark_on_the_new_file_holds_the_rename_back() {
    let sb = Sandbox::new();
    let dev = env("dev", &["token"]);
    let (col, store) = base(&sb, &[("environments/dev.yml", Some(dev.as_str()))]);
    let staging = env("staging", &["token"]);
    teammate(
        &sb,
        &[
            ("environments/dev.yml", None),
            ("environments/staging.yml", Some(staging.as_str())),
        ],
    );
    let copies = ConflictCopies::new(sb.path().join("copies"));
    let (range, _) = pulled(&sb, &col, copies.dir());
    write(&copies.marks_dir(), "environments/staging.yml", "");
    let c = Collection::open(&col).unwrap();
    let held = c
        .secret_renames(&range, Some(&copies), &*store, &sb.options())
        .unwrap();
    assert_eq!((held.renames.len(), held.held_back), (0, 1));
    copies.forget_deleted("environments/staging.yml").unwrap();
    let found = c
        .secret_renames(&range, Some(&copies), &*store, &sb.options())
        .unwrap();
    assert_eq!((found.renames.len(), found.held_back), (1, 0));
}

#[test]
fn a_copy_of_another_file_holds_nothing_back() {
    let sb = Sandbox::new();
    let dev = env("dev", &["token"]);
    let (col, store) = base(&sb, &[("environments/dev.yml", Some(dev.as_str()))]);
    let staging = env("staging", &["token"]);
    teammate(
        &sb,
        &[
            ("environments/dev.yml", None),
            ("environments/staging.yml", Some(staging.as_str())),
        ],
    );
    let copies = ConflictCopies::new(sb.path().join("copies"));
    let (range, _) = pulled(&sb, &col, copies.dir());
    write(copies.dir(), "a.txt", "mine\n");
    let c = Collection::open(&col).unwrap();
    let found = c
        .secret_renames(&range, Some(&copies), &*store, &sb.options())
        .unwrap();
    assert_eq!((found.renames.len(), found.held_back), (1, 0));
}
