//! The hint an undefined variable carries: why it is undefined and how to supply it, built from
//! environment and variable names only.

use crate::common;
use crate::oauth2_support;

use apinata_core::{
    CancelHandle, Collection, ContextOptions, OAuth2Error, ResolveError, RunError, RunOptions,
    TokenCache, UndefinedHint, VarContext, prepare, resolve, run,
};
use common::{request, write_tree};
use oauth2_support::{Fixture, ROOT, executor};
use std::sync::Arc;

const COLLECTION: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: c\n";

fn req_text(var: &str) -> String {
    format!(
        "info:\n  name: {var}\n  type: http\nhttp:\n  method: get\n  url: \"https://x.test/{{{{{var}}}}}\"\n"
    )
}

/// `dev` holds secret `tok` (no value) and plain `base`; `prod` holds `shared` and `dev` too.
fn collection(extra: &[(&str, &str)]) -> (tempfile::TempDir, Collection) {
    let mut files = vec![
        ("opencollection.yml", COLLECTION),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: base\n    value: b\n  - secret: true\n    name: tok\n  - name: shared\n    value: s\n",
        ),
        (
            "environments/prod.yml",
            "name: prod\nvariables:\n  - name: shared\n    value: p\n  - name: ghost\n    value: g\n",
        ),
        ("environments/empty.yml", "name: empty\nvariables: []\n"),
    ];
    files.extend_from_slice(extra);
    let dir = write_tree(&files);
    let c = Collection::open(dir.path()).expect("opens");
    assert!(c.load_errors().is_empty(), "{:?}", c.load_errors());
    (dir, c)
}

fn hint_for(c: &Collection, options: ContextOptions, rel: &str) -> Option<String> {
    let ctx = VarContext::new(c, options).expect("context");
    let err = resolve(request(c, rel), &ctx).expect_err("undefined");
    assert!(
        matches!(err, ResolveError::UndefinedVariable { .. }),
        "{err:?}"
    );
    err.cli_hint()
}

#[test]
fn secret_without_value_says_how_to_supply_it() {
    let t = req_text("tok");
    let (_d, c) = collection(&[("tok.yml", &t)]);
    let hint = hint_for(&c, ContextOptions::new().env("dev"), "tok.yml");
    assert_eq!(
        hint.as_deref(),
        Some(
            "tok is a secret in environment dev with no value; pass --env-var tok=VALUE or set it in the app's environment editor"
        )
    );
}

/// A store that cannot be reached.
struct Refusing;

impl apinata_core::SecretStore for Refusing {
    fn get(
        &self,
        _: &apinata_core::SecretKey,
    ) -> Result<Option<String>, apinata_core::SecretError> {
        Err(apinata_core::SecretError::Unavailable {
            message: "no session bus".into(),
        })
    }
    fn set(&self, _: &apinata_core::SecretKey, _: &str) -> Result<(), apinata_core::SecretError> {
        unreachable!("the hint test never writes")
    }
    fn delete(&self, _: &apinata_core::SecretKey) -> Result<(), apinata_core::SecretError> {
        unreachable!("the hint test never deletes")
    }
}

#[test]
fn a_secret_the_store_could_not_read_names_the_store_error() {
    let t = req_text("tok");
    let (_d, c) = collection(&[("tok.yml", &t)]);
    let options = ContextOptions::new().env("dev").secrets(Arc::new(Refusing));
    let hint = hint_for(&c, options, "tok.yml");
    assert_eq!(
        hint.as_deref(),
        Some(
            "tok is a secret in environment dev; its stored value could not be read (secret store unavailable: no session bus); pass --env-var tok=VALUE or set it in the app's environment editor"
        )
    );
}

#[test]
fn no_environment_names_the_one_that_defines_it() {
    let t = req_text("base");
    let (_d, c) = collection(&[("base.yml", &t)]);
    let hint = hint_for(&c, ContextOptions::new(), "base.yml");
    assert_eq!(
        hint.as_deref(),
        Some("base is defined in environment dev; pass --env dev")
    );
}

#[test]
fn several_environments_are_listed_sorted() {
    let t = req_text("shared");
    let (_d, c) = collection(&[("shared.yml", &t)]);
    let expected = "shared is defined in environments dev, prod; pass --env with one of them";
    let none = hint_for(&c, ContextOptions::new(), "shared.yml");
    assert_eq!(none.as_deref(), Some(expected));
    let other = hint_for(&c, ContextOptions::new().env("empty"), "shared.yml");
    assert_eq!(other.as_deref(), Some(expected));
}

#[test]
fn a_name_nobody_defines_has_no_hint() {
    let t = req_text("nowhere");
    let (_d, c) = collection(&[("nowhere.yml", &t)]);
    assert_eq!(
        hint_for(&c, ContextOptions::new().env("dev"), "nowhere.yml"),
        None
    );
}

#[test]
fn a_supplied_env_var_is_no_error() {
    let t = req_text("tok");
    let (_d, c) = collection(&[("tok.yml", &t)]);
    let ctx = VarContext::new(&c, ContextOptions::new().env("dev").env_var("tok", "v")).unwrap();
    let ok = resolve(request(&c, "tok.yml"), &ctx).expect("resolves");
    assert_eq!(ok.url(), "https://x.test/v");
}

#[test]
fn prepare_stays_an_ok_and_unresolved_stays_an_error() {
    let t = req_text("base");
    let (_d, c) = collection(&[("base.yml", &t)]);
    let ctx = VarContext::new(&c, ContextOptions::new()).unwrap();
    assert!(prepare(request(&c, "base.yml"), &ctx).is_ok());
    let err = resolve(request(&c, "base.yml"), &ctx).unwrap_err();
    assert_eq!(err.to_string(), "variable `base` is not defined");
}

#[test]
fn a_name_written_inside_a_secret_value_gets_no_hint() {
    // `ghost` exists in prod, so only the leak guard keeps the hint away.
    let t = req_text("sec");
    let (_d, c) = collection(&[("sec.yml", &t)]);
    let options = ContextOptions::new()
        .env("dev")
        .sensitive_env_var("sec", "s3cr3t-{{ghost}}");
    let ctx = VarContext::new(&c, options).unwrap();
    let err = resolve(request(&c, "sec.yml"), &ctx).unwrap_err();
    assert!(
        matches!(err, ResolveError::UndefinedVariable { .. }),
        "{err:?}"
    );
    assert_eq!(err.hint(), None);
    assert!(!format!("{err:?}").contains("s3cr3t"));
}

#[test]
fn no_hint_carries_a_secret_value() {
    let t = req_text("tok");
    let (_d, c) = collection(&[("tok.yml", &t)]);
    let options = ContextOptions::new()
        .env("dev")
        .sensitive_env_var("other", "s3cr3t");
    let hint = hint_for(&c, options, "tok.yml").expect("hint");
    assert!(!hint.contains("s3cr3t"), "{hint}");
}

#[tokio::test]
async fn an_oauth2_path_error_carries_the_hint() {
    let auth = "    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"http://127.0.0.1:1/{{tokenPath}}\"\n    credentials:\n      clientId: app\n    scope: read\n    tokenConfig:\n      id: main\n      placement:\n        header: Bearer\n";
    let text = oauth2_support::request("a", 1, "http://127.0.0.1:1/r", auth);
    let f = Fixture::with(
        &[
            ("opencollection.yml", ROOT),
            (
                "environments/dev.yml",
                "name: dev\nvariables:\n  - secret: true\n    name: tokenPath\n",
            ),
            ("a.yml", &text),
        ],
        ContextOptions::new().env("dev"),
        TokenCache::in_memory(),
    );
    let plan = f.collection.plan(f.collection.root()).expect("plan");
    let report = run(
        plan,
        f.ctx.clone(),
        Arc::new(executor()),
        RunOptions::new(),
        CancelHandle::new(),
        Box::new(|_| {}),
    )
    .await;
    let error = report.results[0].error.as_ref().expect("errored");
    assert!(
        matches!(error, RunError::OAuth2(OAuth2Error::Resolve(_))),
        "{error:?}"
    );
    assert_eq!(
        error.hint(),
        Some(&UndefinedHint::SecretWithoutValue {
            environment: "dev".into()
        })
    );
    assert_eq!(
        error.cli_hint().as_deref(),
        Some(
            "tokenPath is a secret in environment dev with no value; pass --env-var tokenPath=VALUE or set it in the app's environment editor"
        )
    );
}
