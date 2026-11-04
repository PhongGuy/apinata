//! `ResolveError::UndefinedVariable` carries an optional hint, as data, that never
//! changes the error's message.

use apinata_core::{ResolveError, RunError, UndefinedHint};

fn undefined(hint: Option<UndefinedHint>) -> ResolveError {
    ResolveError::UndefinedVariable {
        name: "token".into(),
        hint,
    }
}

fn secret(environment: &str) -> UndefinedHint {
    UndefinedHint::SecretWithoutValue {
        environment: environment.into(),
    }
}

#[test]
fn hint_reads_back_and_display_is_unchanged() {
    let hinted = undefined(Some(secret("dev")));
    assert_eq!(hinted.hint(), Some(&secret("dev")));
    assert_eq!(hinted.to_string(), "variable `token` is not defined");
    assert_eq!(undefined(None).hint(), None);
    assert_eq!(
        undefined(None).to_string(),
        "variable `token` is not defined"
    );
}

#[test]
fn with_hint_fills_once_and_never_overwrites() {
    let filled = undefined(None).with_hint(|_| Some(secret("dev")));
    assert_eq!(filled.hint(), Some(&secret("dev")));
    let kept = filled.with_hint(|_| Some(secret("prod")));
    assert_eq!(kept.hint(), Some(&secret("dev")));
    assert_eq!(undefined(None).with_hint(|_| None).hint(), None);
}

#[test]
fn with_hint_ignores_other_variants() {
    let cyclic = ResolveError::CyclicVariable { name: "a".into() };
    let out = cyclic.with_hint(|_| Some(secret("dev")));
    assert_eq!(out.hint(), None);
}

#[test]
fn run_error_forwards_the_hint() {
    let run = RunError::Resolve(undefined(Some(secret("dev"))));
    assert_eq!(run.hint(), Some(&secret("dev")));
    let refused = RunError::Refused {
        message: "no".into(),
    };
    assert_eq!(refused.hint(), None);
}

#[test]
fn cli_text_names_the_variable_and_only_working_remedies() {
    assert_eq!(
        secret("dev").cli_text("tok"),
        "tok is a secret in environment dev with no value; pass --env-var tok=VALUE or set it in the app's environment editor"
    );
    let one = UndefinedHint::DefinedElsewhere {
        environments: vec!["dev".into()],
    };
    assert_eq!(
        one.cli_text("base"),
        "base is defined in environment dev; pass --env dev"
    );
    let many = UndefinedHint::DefinedElsewhere {
        environments: vec!["dev".into(), "prod".into()],
    };
    assert_eq!(
        many.cli_text("base"),
        "base is defined in environments dev, prod; pass --env with one of them"
    );
    let rendered = undefined(Some(secret("dev"))).cli_hint();
    assert_eq!(
        rendered.as_deref(),
        Some(
            "token is a secret in environment dev with no value; pass --env-var token=VALUE or set it in the app's environment editor"
        )
    );
    assert_eq!(RunError::Resolve(undefined(None)).cli_hint(), None);
}

#[test]
fn mcp_text_names_only_remedies_the_server_offers() {
    assert_eq!(
        secret("dev").mcp_text("tok"),
        "tok is a secret in environment dev with no value; set it in the app's environment editor"
    );
    let one = UndefinedHint::DefinedElsewhere {
        environments: vec!["dev".into()],
    };
    assert_eq!(
        one.mcp_text("base"),
        "base is defined in environment dev; select that environment"
    );
    let many = UndefinedHint::DefinedElsewhere {
        environments: vec!["dev".into(), "prod".into()],
    };
    assert_eq!(
        many.mcp_text("base"),
        "base is defined in environments dev, prod; select one of them"
    );
    let rendered = undefined(Some(secret("dev"))).mcp_hint();
    assert_eq!(
        rendered.as_deref(),
        Some(
            "token is a secret in environment dev with no value; set it in the app's environment editor"
        )
    );
    assert_eq!(RunError::Resolve(undefined(None)).mcp_hint(), None);
    assert_eq!(
        RunError::Resolve(undefined(Some(secret("dev")))).mcp_hint(),
        rendered
    );
}
