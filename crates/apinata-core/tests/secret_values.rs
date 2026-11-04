//! `VarContext::secret_values`: every secret value a collection's context knows.

use crate::common;

use apinata_core::{
    Collection, ContextOptions, MemoryStore, SecretStore, VarContext, known_secrets,
};
use common::{request, write_tree};
use std::sync::Arc;

const DEV: &str = "name: dev\nvariables:\n  - secret: true\n    name: token\n  - secret: true\n    name: pin\n  - secret: true\n    name: templ\n  - secret: true\n    name: broken\n  - name: suffix\n    value: tail\n  - name: shared\n    value: plain-env-value\n";
const FOLDER: &str = "info:\n  name: f\n  type: folder\n  seq: 1\nrequest:\n  variables:\n    - name: token\n      value: plain-folder-value\n";
const IN_FOLDER: &str = "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://127.0.0.1:9/{{token}}\"\n";

fn context() -> (tempfile::TempDir, Collection, VarContext) {
    let dir = write_tree(&[
        (
            "opencollection.yml",
            "opencollection: 1.0.0\ninfo:\n  name: c\n",
        ),
        ("environments/dev.yml", DEV),
        ("f/folder.yml", FOLDER),
        ("f/r.yml", IN_FOLDER),
        (".env", "DOTENV_KEY=dotenv-value-77\n"),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let store = Arc::new(MemoryStore::new());
    for (name, value) in [
        ("token", "env-token-value-1"),
        ("pin", "123"),
        ("templ", "tok-{{suffix}}-end"),
        ("broken", "raw-{{nowhere}}-text"),
    ] {
        store.set(&c.secret_key("dev", name), value).unwrap();
    }
    let options = ContextOptions::new()
        .env("dev")
        .secrets(store)
        .process_env([("HOST_ONLY", "host-process-value")])
        .sensitive_global_env_var("shared", "global-shadowed-value");
    let ctx = VarContext::new(&c, options).expect("builds");
    (dir, c, ctx)
}

#[test]
fn a_secret_shadowed_by_a_folder_variable_is_still_listed() {
    let (_dir, c, ctx) = context();
    let named: Vec<String> = known_secrets(request(&c, "f/r.yml"), &ctx)
        .unwrap()
        .iter()
        .map(|s| s.value().to_owned())
        .collect();
    assert!(named.contains(&"env-token-value-1".to_owned()), "{named:?}");
    assert!(
        ctx.secret_values()
            .contains(&"env-token-value-1".to_owned())
    );
}

#[test]
fn dotenv_values_and_shadowed_globals_are_listed_host_values_and_short_ones_are_not() {
    let (_dir, _c, ctx) = context();
    let values = ctx.secret_values();
    assert!(values.contains(&"dotenv-value-77".to_owned()), "{values:?}");
    assert!(
        values.contains(&"global-shadowed-value".to_owned()),
        "{values:?}"
    );
    assert!(!values.contains(&"host-process-value".to_owned()));
    assert!(!values.contains(&"plain-env-value".to_owned()));
    assert!(
        !values.contains(&"123".to_owned()),
        "under the 4-character floor"
    );
}

#[test]
fn a_secret_is_listed_rendered_and_one_that_cannot_render_as_written() {
    let (_dir, _c, ctx) = context();
    let values = ctx.secret_values();
    assert!(values.contains(&"tok-tail-end".to_owned()), "{values:?}");
    assert!(!values.contains(&"tok-{{suffix}}-end".to_owned()));
    assert!(
        values.contains(&"raw-{{nowhere}}-text".to_owned()),
        "{values:?}"
    );
}

#[test]
fn the_list_has_no_repeats_and_runs_longest_first() {
    let (_dir, _c, ctx) = context();
    let values = ctx.secret_values();
    let lengths: Vec<usize> = values.iter().map(|v| v.chars().count()).collect();
    assert!(lengths.windows(2).all(|w| w[0] >= w[1]), "{values:?}");
    let mut unique = values.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), values.len(), "{values:?}");
}
