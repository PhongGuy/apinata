//! Secret values from a store in the variable context: the active environment and its `extends`
//! chain, override precedence, missing values and store failures.

use crate::common;

use apinata_core::{
    Collection, ContextOptions, MemoryStore, ResolveError, SecretError, SecretKey,
    SecretReadFailure, SecretStore, UndefinedHint, VarContext,
};
use common::{at, write_tree};
use std::sync::Arc;
use tempfile::TempDir;

/// A store that cannot be reached.
struct Unavailable;

impl SecretStore for Unavailable {
    fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
        Err(SecretError::Unavailable {
            message: "no service".into(),
        })
    }
    fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
        Err(SecretError::Unavailable {
            message: "no service".into(),
        })
    }
    fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
        Err(SecretError::Unavailable {
            message: "no service".into(),
        })
    }
}

/// `dev` extends `base`. `base` owns secret `inherited`; `dev` owns `own` and re-declares
/// `shared`, which `base` also holds as a secret. `r.yml` puts one variable in its URL path.
fn collection() -> (TempDir, Collection) {
    let request = |name: &str| {
        format!(
            "info:\n  name: {name}\n  type: http\nhttp:\n  method: get\n  url: \"https://x.test/{{{{{name}}}}}\"\n"
        )
    };
    let dir = write_tree(&[
        (
            "opencollection.yml",
            "opencollection: 1.0.0\ninfo:\n  name: c\n",
        ),
        (
            "environments/base.yml",
            "name: base\nvariables:\n  - secret: true\n    name: inherited\n  - secret: true\n    name: shared\n",
        ),
        (
            "environments/dev.yml",
            "name: dev\nextends: base\nvariables:\n  - secret: true\n    name: own\n  - secret: true\n    name: shared\n  - secret: true\n    name: off\n    disabled: true\n",
        ),
        ("own.yml", &request("own")),
        ("inherited.yml", &request("inherited")),
        ("shared.yml", &request("shared")),
        ("off.yml", &request("off")),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    assert!(c.load_errors().is_empty(), "{:?}", c.load_errors());
    (dir, c)
}

fn url(c: &Collection, ctx: &VarContext, rel: &str) -> Result<String, ResolveError> {
    at(c, ctx, rel).map(|r| r.url().to_owned())
}

fn undefined(name: &str) -> Result<String, ResolveError> {
    Err(ResolveError::UndefinedVariable {
        name: name.to_owned(),
        hint: None,
    })
}

/// A secret of environment `dev` with no value, which now says how to supply it.
fn unset_secret(name: &str) -> Result<String, ResolveError> {
    Err(ResolveError::UndefinedVariable {
        name: name.to_owned(),
        hint: Some(UndefinedHint::SecretWithoutValue {
            environment: "dev".to_owned(),
        }),
    })
}

fn store_for(c: &Collection, values: &[(&str, &str, &str)]) -> Arc<MemoryStore> {
    let store = MemoryStore::new();
    for (env, var, value) in values {
        store.set(&c.secret_key(env, var), value).expect("set");
    }
    Arc::new(store)
}

#[test]
fn secrets_come_from_the_environment_that_declares_them() {
    let (_dir, c) = collection();
    let store = store_for(
        &c,
        &[
            ("dev", "own", "o1"),
            ("base", "inherited", "i1"),
            ("base", "shared", "from-base"),
            ("dev", "shared", "from-dev"),
            ("dev", "off", "never"),
        ],
    );
    let ctx =
        VarContext::new(&c, ContextOptions::new().env("dev").secrets(store)).expect("context");
    assert!(ctx.warnings().is_empty(), "{:?}", ctx.warnings());
    assert_eq!(url(&c, &ctx, "own.yml"), Ok("https://x.test/o1".into()));
    assert_eq!(
        url(&c, &ctx, "inherited.yml"),
        Ok("https://x.test/i1".into())
    );
    assert_eq!(
        url(&c, &ctx, "shared.yml"),
        Ok("https://x.test/from-dev".into())
    );
    assert_eq!(url(&c, &ctx, "off.yml"), undefined("off"));
}

#[test]
fn an_override_wins_over_the_store() {
    let (_dir, c) = collection();
    let store = store_for(&c, &[("dev", "own", "stored")]);
    let options = ContextOptions::new()
        .env("dev")
        .env_var("own", "cli")
        .secrets(store);
    let ctx = VarContext::new(&c, options).expect("context");
    assert_eq!(url(&c, &ctx, "own.yml"), Ok("https://x.test/cli".into()));
}

#[test]
fn a_missing_value_or_no_store_leaves_the_secret_undefined() {
    let (_dir, c) = collection();
    let empty = VarContext::new(
        &c,
        ContextOptions::new()
            .env("dev")
            .secrets(Arc::new(MemoryStore::new())),
    )
    .expect("context");
    assert!(empty.warnings().is_empty());
    assert_eq!(url(&c, &empty, "own.yml"), unset_secret("own"));
    let none = VarContext::new(&c, ContextOptions::new().env("dev")).expect("context");
    assert_eq!(url(&c, &none, "inherited.yml"), unset_secret("inherited"));
}

#[test]
fn a_failing_store_warns_once_per_secret_and_leaves_them_undefined() {
    let (_dir, c) = collection();
    let ctx = VarContext::new(
        &c,
        ContextOptions::new()
            .env("dev")
            .secrets(Arc::new(Unavailable)),
    )
    .expect("a store failure does not fail the context");
    let mut warnings = ctx.warnings().to_vec();
    warnings.sort();
    assert_eq!(
        warnings,
        [
            "secret `inherited` of environment `base` could not be read: secret store unavailable: no service",
            "secret `own` of environment `dev` could not be read: secret store unavailable: no service",
            "secret `shared` of environment `dev` could not be read: secret store unavailable: no service",
        ]
    );
    // The unreadable secret's hint carries the store's error, not just "no value".
    assert_eq!(
        url(&c, &ctx, "own.yml"),
        Err(ResolveError::UndefinedVariable {
            name: "own".to_owned(),
            hint: Some(UndefinedHint::SecretUnreadable {
                environment: "dev".to_owned(),
                reason: "secret store unavailable: no service".to_owned(),
            }),
        })
    );
}

#[test]
fn no_environment_means_no_store_reads() {
    let (_dir, c) = collection();
    let ctx =
        VarContext::new(&c, ContextOptions::new().secrets(Arc::new(Unavailable))).expect("context");
    assert!(ctx.warnings().is_empty(), "{:?}", ctx.warnings());
}

#[test]
fn a_failing_store_is_reported_as_typed_failures() {
    let (_dir, c) = collection();
    let ctx = VarContext::new(
        &c,
        ContextOptions::new()
            .env("dev")
            .secrets(Arc::new(Unavailable)),
    )
    .expect("context");
    let mut found: Vec<(String, String, bool)> = ctx
        .secret_read_failures()
        .iter()
        .map(|f| {
            assert!(matches!(f.error, SecretError::Unavailable { .. }));
            (f.environment.clone(), f.name.clone(), f.global)
        })
        .collect();
    found.sort();
    assert_eq!(
        found,
        [
            ("base".to_owned(), "inherited".to_owned(), false),
            ("dev".to_owned(), "own".to_owned(), false),
            ("dev".to_owned(), "shared".to_owned(), false),
        ]
    );
    // The text warning is the failure's Display, so every surface shows the same line.
    let failure: &SecretReadFailure = &ctx.secret_read_failures()[0];
    assert!(ctx.warnings().contains(&failure.to_string()));
}

/// A file that already holds one name twice: the later plain row wins, and a secret wins over a
/// plain row of its name whichever comes first, matching the format's own resolution.
#[test]
fn a_duplicated_name_resolves_to_the_last_row_and_a_secret_beats_a_plain_twin() {
    let request = |name: &str| {
        format!(
            "info:\n  name: {name}\n  type: http\nhttp:\n  method: get\n  url: \"https://x.test/{{{{{name}}}}}\"\n"
        )
    };
    let dir = write_tree(&[
        (
            "opencollection.yml",
            "opencollection: 1.0.0\ninfo:\n  name: c\n",
        ),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: twice\n    value: first\n  - name: twice\n    value: second\n  - secret: true\n    name: before\n  - name: before\n    value: plain\n  - name: after\n    value: plain\n  - secret: true\n    name: after\n  - name: off\n    value: on\n  - name: off\n    value: dropped\n    disabled: true\n",
        ),
        ("twice.yml", &request("twice")),
        ("before.yml", &request("before")),
        ("after.yml", &request("after")),
        ("off.yml", &request("off")),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    assert!(c.load_errors().is_empty(), "{:?}", c.load_errors());
    let store = store_for(&c, &[("dev", "before", "s1"), ("dev", "after", "s2")]);
    let ctx =
        VarContext::new(&c, ContextOptions::new().env("dev").secrets(store)).expect("context");
    assert_eq!(
        url(&c, &ctx, "twice.yml"),
        Ok("https://x.test/second".into())
    );
    assert_eq!(url(&c, &ctx, "before.yml"), Ok("https://x.test/s1".into()));
    assert_eq!(url(&c, &ctx, "after.yml"), Ok("https://x.test/s2".into()));
    assert_eq!(url(&c, &ctx, "off.yml"), Ok("https://x.test/on".into()));
}
