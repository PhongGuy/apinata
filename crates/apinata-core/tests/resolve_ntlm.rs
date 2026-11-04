//! NTLM through resolve: the whitelist, inheritance, errors, secrets and codegen.

use crate::common;

use apinata_core::{ResolveError, Target, generate, named_secrets, prepare};
use common::{at, request, resolved_one, setup};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nrequest:\n  variables:\n    - name: pw\n      value: ntlm-pass-1\n    - name: usr\n      value: alice\n    - name: dom\n      value: CORP\n";

fn http(auth: &str) -> String {
    format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: https://x.test/p\n  auth:\n{auth}"
    )
}

fn ntlm(fields: &str) -> String {
    format!("    type: ntlm\n{fields}")
}

const FULL: &str = "    username: \"{{usr}}\"\n    password: \"{{pw}}\"\n    domain: \"{{dom}}\"\n";

#[test]
fn ntlm_resolves_where_it_was_unsupported_and_adds_no_header() {
    let (_d, c, ctx) = setup(&[("opencollection.yml", ROOT), ("r.yml", &http(&ntlm(FULL)))]);
    let r = at(&c, &ctx, "r.yml").expect("resolves");
    let p = prepare(request(&c, "r.yml"), &ctx).expect("prepares");
    assert_eq!(p.auth_mode(), "ntlm");
    assert!(r.headers().is_empty(), "{:?}", r.headers());
}

#[test]
fn a_collection_level_ntlm_is_inherited() {
    let root = format!("{ROOT}  auth:\n{}", ntlm(FULL));
    let inherit = "info:\n  name: i\n  type: http\nhttp:\n  method: GET\n  url: https://x.test/p\n  auth: inherit\n";
    let (_d, c, ctx) = setup(&[("opencollection.yml", &root), ("i.yml", inherit)]);
    at(&c, &ctx, "i.yml").expect("resolves");
    let p = prepare(request(&c, "i.yml"), &ctx).expect("prepares");
    assert_eq!(p.auth_mode(), "ntlm");
}

#[test]
fn an_unresolved_variable_in_any_field_is_an_error_never_a_literal() {
    for (fields, name) in [
        (
            "    username: \"{{nope}}\"\n    password: p\n    domain: d\n",
            "nope",
        ),
        (
            "    username: u\n    password: \"{{nope2}}\"\n    domain: d\n",
            "nope2",
        ),
        (
            "    username: u\n    password: p\n    domain: \"{{nope3}}\"\n",
            "nope3",
        ),
    ] {
        let (_d, c, ctx) = setup(&[
            ("opencollection.yml", ROOT),
            ("r.yml", &http(&ntlm(fields))),
        ]);
        match at(&c, &ctx, "r.yml") {
            Err(ResolveError::UndefinedVariable { name: got, .. }) => assert_eq!(got, name),
            other => panic!("expected UndefinedVariable({name}), got {other:?}"),
        }
    }
}

#[test]
fn an_unknown_auth_type_is_still_unsupported() {
    let (_d, c, ctx) = setup(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &http("    type: kerberos\n")),
    ]);
    assert!(
        matches!(at(&c, &ctx, "r.yml"), Err(ResolveError::UnsupportedAuth { kind }) if kind == "kerberos")
    );
}

#[test]
fn only_the_password_is_sensitive_named_and_kept_out_of_debug() {
    let (_d, c, ctx) = setup(&[("opencollection.yml", ROOT), ("r.yml", &http(&ntlm(FULL)))]);
    let r = at(&c, &ctx, "r.yml").expect("resolves");
    let named = named_secrets(request(&c, "r.yml"), &ctx).expect("names");
    assert!(r.sensitive_values().iter().any(|v| v == "ntlm-pass-1"));
    assert!(named.iter().any(|n| n.value() == "ntlm-pass-1"));
    assert!(
        !r.sensitive_values()
            .iter()
            .any(|v| v == "alice" || v == "CORP")
    );
    assert!(!format!("{r:?}").contains("ntlm-pass-1"));
}

#[test]
fn codegen_names_the_mode_and_signs_nothing() {
    let r = resolved_one(
        "GET",
        "https://x.test/p",
        "  auth:\n    type: ntlm\n    username: u\n    password: pw-secret-9\n    domain: d\n",
        "",
    );
    for t in Target::ALL {
        let code = generate(&r, *t);
        assert!(
            code.contains("NTLM auth is not included in this snippet."),
            "{}: {code}",
            t.id()
        );
        assert!(!code.contains("pw-secret-9"), "{}", t.id());
        assert!(!code.to_lowercase().contains("authorization"), "{}", t.id());
    }
}
