//! The error for an unknown environment names the ones that exist.

use crate::common;

use apinata_core::{
    Collection, ContextOptions, EnvError, GlobalEnvironments, SecretStore, VarContext,
};
use common::{ROOT, write_tree};

const ENV: &str = "name: x\nvariables: []\n";

fn message(files: &[(&str, &str)], wanted: &str) -> String {
    let dir = write_tree(files);
    let c = Collection::open(dir.path()).expect("open");
    VarContext::new(&c, ContextOptions::new().env(wanted))
        .expect_err("unknown environment")
        .to_string()
}

fn three() -> Vec<(&'static str, &'static str)> {
    vec![
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", ENV),
        ("environments/staging.yml", ENV),
        ("environments/prod.yml", ENV),
    ]
}

#[test]
fn the_context_error_lists_names_and_suggests_for_a_typo() {
    assert_eq!(
        message(&three(), "stagin"),
        "environment \"stagin\" not found; available: dev, prod, staging; did you mean staging?"
    );
}

#[test]
fn a_case_typo_is_suggested() {
    let text = message(&three(), "Dev");
    assert!(text.ends_with("; did you mean dev?"), "{text}");
}

#[test]
fn a_far_name_lists_without_a_suggestion() {
    assert_eq!(
        message(&three(), "nosuch"),
        "environment \"nosuch\" not found; available: dev, prod, staging"
    );
}

#[test]
fn a_collection_without_environments_says_so() {
    assert_eq!(
        message(&[("opencollection.yml", ROOT)], "x"),
        "environment \"x\" not found; this collection has no environments"
    );
}

#[test]
fn the_global_error_lists_global_names() {
    let profile = write_tree(&[("shared.yml", ENV), ("other.yml", ENV)]);
    let globals = std::sync::Arc::new(GlobalEnvironments::open(profile.path()).expect("globals"));
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    let c = Collection::open(dir.path()).expect("open");
    let err = VarContext::new(&c, ContextOptions::new().global_env(globals, "shard"))
        .expect_err("unknown");
    assert_eq!(
        err.to_string(),
        "global environment \"shard\" not found; available: other, shared; did you mean shared?"
    );
    let empty = write_tree(&[]);
    let none = std::sync::Arc::new(GlobalEnvironments::open(empty.path()).expect("globals"));
    let err = VarContext::new(&c, ContextOptions::new().global_env(none, "x")).expect_err("none");
    assert_eq!(
        err.to_string(),
        "global environment \"x\" not found; no global environments"
    );
}

#[test]
fn the_environment_ops_error_lists_names() {
    let dir = write_tree(&three());
    let mut c = Collection::open(dir.path()).expect("open");
    let store: &dyn SecretStore = &apinata_core::MemoryStore::new();
    let err = c.delete_environment("prd", store).expect_err("unknown");
    assert!(matches!(err, EnvError::NotFound { .. }));
    assert_eq!(
        err.to_string(),
        "environment \"prd\" not found; available: dev, prod, staging; did you mean prod?"
    );
}
