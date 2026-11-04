//! A valid `{{?label}}` gets its own highlight, distinct from an undefined variable.

use crate::support;

use std::sync::Arc;

use apinata_app::theme::{VarKind, var_tint_hex};
use apinata_app::var_tokens::tokens;
use apinata_core::{Collection, ContextOptions, VarContext, VariableSet};
use gpui_kit::component::ThemeMode;
use support::*;

const VARS_ROOT: &str = "opencollection: 1.0.0\ninfo:\n  name: c\nrequest:\n  variables:\n    - name: host\n      value: c.test\n";
const TEXT: &str = "{{?Order id}}/{{?}}/{{? x}}/{{host}}";

fn vars() -> (tempfile::TempDir, Arc<VariableSet>) {
    let dir = write_tree(&[
        ("opencollection.yml", VARS_ROOT),
        ("a.yml", &request("a", TEXT)),
    ]);
    let c = Collection::open(dir.path()).unwrap();
    let options = ContextOptions::new().process_env(Vec::<(String, String)>::new());
    let ctx = VarContext::new(&c, options).unwrap();
    let set = ctx.variables(c.request(&ip("a.yml")).unwrap());
    (dir, Arc::new(set))
}

#[test]
fn a_valid_prompt_is_its_own_kind_and_an_invalid_one_is_undefined() {
    let (_dir, vars) = vars();
    let found: Vec<(VarKind, String)> = tokens(TEXT, &vars)
        .into_iter()
        .map(|t| (t.kind, t.tip))
        .collect();
    assert_eq!(
        found,
        [
            (VarKind::Prompt, "Prompt · asked on send".to_owned()),
            (VarKind::Undefined, "Undefined variable".to_owned()),
            (VarKind::Undefined, "Undefined variable".to_owned()),
            (VarKind::Defined, "Collection · c.test".to_owned()),
        ]
    );
}

#[test]
fn the_prompt_tint_is_listed_and_differs_from_undefined() {
    assert!(VarKind::ALL.contains(&VarKind::Prompt));
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        assert_ne!(
            var_tint_hex(VarKind::Prompt, mode),
            var_tint_hex(VarKind::Undefined, mode)
        );
    }
}
