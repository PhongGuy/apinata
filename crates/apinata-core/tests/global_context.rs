//! The global environment layer: precedence, secrets, overrides, sensitivity and errors.

use crate::common;

use apinata_core::{
    Collection, ContextError, ContextOptions, GlobalEnvironments, MemoryStore, ResolveError,
    SecretError, SecretKey, SecretStore, VarContext, VarScope,
};
use common::{at, request, write_tree};
use std::sync::Arc;
use tempfile::TempDir;

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

fn req(name: &str, url: &str) -> String {
    format!("info:\n  name: {name}\n  type: http\nhttp:\n  method: get\n  url: \"{url}\"\n")
}

/// Collection `c/` and global environments `g/` side by side in one temp dir.
fn fixture() -> (TempDir, Collection, Arc<GlobalEnvironments>) {
    let dir = write_tree(&[
        (
            "c/opencollection.yml",
            "opencollection: 1.0.0\ninfo:\n  name: c\nrequest:\n  variables:\n    - name: both\n      value: from-collection\n    - name: shadow\n      value: open\n",
        ),
        (
            "c/environments/dev.yml",
            "name: dev\nvariables:\n  - name: envwins\n    value: from-env\n  - name: envshadow\n    value: env-open\n",
        ),
        (
            "c/r.yml",
            &req("r", "https://x.test/{{only}}/{{both}}/{{envwins}}"),
        ),
        ("c/tok.yml", &req("tok", "https://x.test/{{tok}}")),
        ("c/shadow.yml", &req("shadow", "https://x.test/{{shadow}}")),
        (
            "c/envshadow.yml",
            &req("envshadow", "https://x.test/{{envshadow}}"),
        ),
        ("c/name.yml", &req("name", "https://x.test/{{__name__}}")),
        (
            "g/g.yml",
            "name: g\nvariables:\n  - name: only\n    value: from-global\n  - name: both\n    value: from-global\n  - name: envwins\n    value: from-global\n  - secret: true\n    name: tok\n  - secret: true\n    name: shadow\n  - secret: true\n    name: envshadow\n",
        ),
        ("g/broken.yml", "name: [unclosed\n"),
    ]);
    let c = Collection::open(dir.path().join("c")).expect("opens");
    assert!(c.load_errors().is_empty(), "{:?}", c.load_errors());
    let g = GlobalEnvironments::open(dir.path().join("g")).expect("globals");
    (dir, c, Arc::new(g))
}

fn url(c: &Collection, ctx: &VarContext, rel: &str) -> Result<String, ResolveError> {
    at(c, ctx, rel).map(|r| r.url().to_owned())
}

fn context_error(c: &Collection, options: ContextOptions) -> ContextError {
    match VarContext::new(c, options) {
        Err(e) => e,
        Ok(_) => panic!("expected a context error"),
    }
}

#[test]
fn the_global_layer_sits_below_collection_variables_and_the_environment() {
    let (_d, c, g) = fixture();
    let ctx =
        VarContext::new(&c, ContextOptions::new().env("dev").global_env(g, "g")).expect("context");
    assert_eq!(
        url(&c, &ctx, "r.yml"),
        Ok("https://x.test/from-global/from-collection/from-env".into())
    );
    let vars = ctx.variables(request(&c, "r.yml"));
    assert_eq!(
        vars.get("only").expect("only").scope(),
        VarScope::GlobalEnvironment
    );
    assert_eq!(
        vars.get("both").expect("both").scope(),
        VarScope::Collection
    );
    assert_eq!(
        vars.get("envwins").expect("envwins").scope(),
        VarScope::Environment
    );
}

#[test]
fn global_secrets_come_from_the_store_under_the_global_key() {
    let (_d, c, g) = fixture();
    let store = MemoryStore::new();
    store.set(&g.secret_key("g", "tok"), "s3cret").expect("set");
    let options = ContextOptions::new()
        .global_env(g.clone(), "g")
        .secrets(Arc::new(store));
    let ctx = VarContext::new(&c, options).expect("context");
    let r = at(&c, &ctx, "tok.yml").expect("resolves");
    assert_eq!(r.url(), "https://x.test/s3cret");
    assert!(r.sensitive_values().iter().any(|v| v == "s3cret"));

    let bare = VarContext::new(&c, ContextOptions::new().global_env(g, "g")).expect("context");
    assert_eq!(
        url(&c, &bare, "tok.yml"),
        Err(ResolveError::UndefinedVariable {
            name: "tok".into(),
            hint: None
        })
    );
}

#[test]
fn global_overrides_beat_the_file_and_carry_their_sensitivity() {
    let (_d, c, g) = fixture();
    let plain = VarContext::new(
        &c,
        ContextOptions::new()
            .global_env(g.clone(), "g")
            .global_env_var("tok", "cli"),
    )
    .expect("context");
    let r = at(&c, &plain, "tok.yml").expect("resolves");
    assert_eq!(r.url(), "https://x.test/cli");
    assert!(!r.sensitive_values().iter().any(|v| v == "cli"));

    let hidden = VarContext::new(
        &c,
        ContextOptions::new()
            .global_env(g, "g")
            .sensitive_global_env_var("tok", "hid"),
    )
    .expect("context");
    let r = at(&c, &hidden, "tok.yml").expect("resolves");
    assert_eq!(r.url(), "https://x.test/hid");
    assert!(r.sensitive_values().iter().any(|v| v == "hid"));
}

#[test]
fn global_overrides_apply_without_a_selected_global_environment() {
    let (_d, c, _g) = fixture();
    let ctx = VarContext::new(
        &c,
        ContextOptions::new().env("dev").global_env_var("only", "x"),
    )
    .expect("context");
    assert_eq!(
        url(&c, &ctx, "r.yml"),
        Ok("https://x.test/x/from-collection/from-env".into())
    );
}

#[test]
fn a_sensitive_global_redefined_by_a_higher_layer_is_not_sensitive() {
    let (_d, c, g) = fixture();
    let store = MemoryStore::new();
    store
        .set(&g.secret_key("g", "shadow"), "hidden")
        .expect("set");
    store
        .set(&g.secret_key("g", "envshadow"), "hidden2")
        .expect("set");
    let options = ContextOptions::new()
        .env("dev")
        .global_env(g, "g")
        .secrets(Arc::new(store));
    let ctx = VarContext::new(&c, options).expect("context");
    let r = at(&c, &ctx, "shadow.yml").expect("resolves");
    assert_eq!(r.url(), "https://x.test/open");
    assert!(
        !r.sensitive_values().iter().any(|v| v == "open"),
        "{:?}",
        r.sensitive_values()
    );
    let r = at(&c, &ctx, "envshadow.yml").expect("resolves");
    assert_eq!(r.url(), "https://x.test/env-open");
    assert!(
        !r.sensitive_values().iter().any(|v| v == "env-open"),
        "{:?}",
        r.sensitive_values()
    );
}

#[test]
fn a_global_environment_never_sets_the_environment_name() {
    let (_d, c, g) = fixture();
    let only =
        VarContext::new(&c, ContextOptions::new().global_env(g.clone(), "g")).expect("context");
    assert_eq!(
        url(&c, &only, "name.yml"),
        Err(ResolveError::UndefinedVariable {
            name: "__name__".into(),
            hint: None
        })
    );
    let both =
        VarContext::new(&c, ContextOptions::new().env("dev").global_env(g, "g")).expect("context");
    assert_eq!(url(&c, &both, "name.yml"), Ok("https://x.test/dev".into()));
}

#[test]
fn an_unknown_or_broken_global_environment_is_an_error() {
    let (_d, c, g) = fixture();
    let e = context_error(&c, ContextOptions::new().global_env(g.clone(), "nope"));
    assert!(
        matches!(&e, ContextError::GlobalEnvironmentNotFound { name, .. } if name == "nope"),
        "{e:?}"
    );
    assert_eq!(
        e.to_string(),
        "global environment \"nope\" not found; available: g"
    );
    let e = context_error(&c, ContextOptions::new().global_env(g, "broken"));
    assert!(
        matches!(&e, ContextError::GlobalEnvironmentFailedToLoad { name, .. } if name == "broken"),
        "{e:?}"
    );
    assert!(
        e.to_string()
            .starts_with("global environment `broken` failed to load: "),
        "{e}"
    );
}

#[test]
fn a_failing_store_names_the_global_environment() {
    let (_d, c, g) = fixture();
    let strict = ContextOptions::new()
        .global_env(g.clone(), "g")
        .secrets(Arc::new(Unavailable))
        .strict_secrets();
    let e = context_error(&c, strict);
    assert!(
        matches!(&e, ContextError::GlobalSecretUnavailable { environment, name, .. } if environment == "g" && name == "envshadow"),
        "{e:?}"
    );
    let lenient = VarContext::new(
        &c,
        ContextOptions::new()
            .global_env(g, "g")
            .secrets(Arc::new(Unavailable)),
    )
    .expect("context");
    assert!(
        lenient
            .warnings()
            .iter()
            .any(|w| w.contains("global environment `g`")),
        "{:?}",
        lenient.warnings()
    );
}

#[test]
fn undefined_name_redaction_covers_a_read_global_secret() {
    // The name is authored, so only the global secret's sensitivity can withhold it.
    let dir = write_tree(&[
        (
            "c/opencollection.yml",
            "opencollection: 1.0.0\ninfo:\n  name: c\n",
        ),
        ("c/r.yml", &req("r", "https://x.test/{{a-s3cret}}/{{tok}}")),
        (
            "g/g.yml",
            "name: g\nvariables:\n  - secret: true\n    name: tok\n",
        ),
    ]);
    let c = Collection::open(dir.path().join("c")).expect("opens");
    let g = Arc::new(GlobalEnvironments::open(dir.path().join("g")).expect("globals"));
    let store = MemoryStore::new();
    store.set(&g.secret_key("g", "tok"), "s3cret").expect("set");
    let ctx = |store: MemoryStore| {
        let options = ContextOptions::new()
            .global_env(g.clone(), "g")
            .secrets(Arc::new(store));
        VarContext::new(&c, options).expect("context")
    };
    assert!(ctx(store).name_inside_sensitive_value(request(&c, "r.yml"), "a-s3cret"));
    assert!(
        !ctx(MemoryStore::new()).name_inside_sensitive_value(request(&c, "r.yml"), "a-s3cret"),
        "with nothing stored, the authored name is named"
    );
}
