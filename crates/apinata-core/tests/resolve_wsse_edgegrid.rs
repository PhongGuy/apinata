//! WSSE and Akamai EdgeGrid through resolve: the whitelist, errors, secrets and codegen.

use crate::common;

use apinata_core::{ResolveError, ResolvedRequest, Target, generate, named_secrets};
use common::{at, request, resolved_one, setup};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nrequest:\n  variables:\n    - name: pw\n      value: wsse-pass-1\n";
const CREDS: &str = "    accessToken: akab-access-1\n    clientToken: akab-client-1\n    clientSecret: eg-secret-1\n";

fn http(auth: &str) -> String {
    format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: https://x.test/p\n  auth:\n{auth}"
    )
}

fn eg(fields: &str) -> String {
    format!("    type: akamai-edgegrid\n{fields}")
}

fn resolve_one(auth: &str) -> Result<ResolvedRequest, ResolveError> {
    let (_d, c, ctx) = setup(&[("opencollection.yml", ROOT), ("r.yml", &http(auth))]);
    at(&c, &ctx, "r.yml")
}

#[test]
fn both_modes_resolve_where_they_were_unsupported() {
    resolve_one("    type: wsse\n    username: u\n    password: \"{{pw}}\"\n")
        .expect("wsse resolves");
    resolve_one(&eg(CREDS)).expect("edgegrid resolves");
}

#[test]
fn missing_credentials_name_the_field_like_the_reference() {
    // Messages from vectors.json `missing-*`.
    for (fields, want) in [
        (
            "    accessToken: \"\"\n    clientToken: ct\n    clientSecret: cs\n",
            "EdgeGrid: accessToken is required",
        ),
        (
            "    accessToken: at\n    clientToken: \"   \"\n    clientSecret: cs\n",
            "EdgeGrid: clientToken is required",
        ),
        (
            "    accessToken: at\n    clientToken: ct\n    clientSecret: undefined\n",
            "EdgeGrid: clientSecret is required",
        ),
        ("", "EdgeGrid: accessToken is required"),
    ] {
        let err = resolve_one(&eg(fields)).expect_err("refused");
        assert!(matches!(err, ResolveError::EdgeGrid { .. }), "{err:?}");
        assert_eq!(err.to_string(), want);
    }
}

#[test]
fn a_base_url_that_does_not_parse_is_a_resolve_error() {
    let err =
        resolve_one(&eg(&format!("{CREDS}    baseURL: \"https://[::1\"\n"))).expect_err("refused");
    assert_eq!(err.to_string(), "EdgeGrid: baseURL is not a valid URL");
}

#[test]
fn an_unresolved_variable_fails_before_anything_is_sent() {
    for auth in [
        "    type: wsse\n    username: u\n    password: \"{{nope}}\"\n".to_owned(),
        eg("    accessToken: at\n    clientToken: ct\n    clientSecret: \"{{nope}}\"\n"),
        eg(&format!("{CREDS}    baseURL: \"{{{{nope}}}}\"\n")),
    ] {
        assert_eq!(
            resolve_one(&auth).err(),
            Some(ResolveError::UndefinedVariable {
                name: "nope".into(),
                hint: None
            }),
            "{auth}"
        );
    }
}

#[test]
fn credentials_are_sensitive_named_and_kept_out_of_debug() {
    for (auth, secrets) in [
        (
            "    type: wsse\n    username: u\n    password: wsse-literal-1\n".to_owned(),
            vec!["wsse-literal-1"],
        ),
        (
            "    type: wsse\n    username: u\n    password: \"{{pw}}\"\n".to_owned(),
            vec!["wsse-pass-1"],
        ),
        (
            eg(CREDS),
            vec!["akab-access-1", "akab-client-1", "eg-secret-1"],
        ),
    ] {
        let (_d, c, ctx) = setup(&[("opencollection.yml", ROOT), ("r.yml", &http(&auth))]);
        let r = at(&c, &ctx, "r.yml").expect("resolves");
        let named = named_secrets(request(&c, "r.yml"), &ctx).expect("names");
        for secret in secrets {
            assert!(r.sensitive_values().iter().any(|v| v == secret), "{secret}");
            assert!(named.iter().any(|n| n.value() == secret), "{secret}");
            assert!(!format!("{r:?}").contains(secret), "{secret}");
        }
    }
}

#[test]
fn codegen_names_both_modes_and_signs_neither() {
    let wsse = resolved_one(
        "GET",
        "https://x.test/p",
        "  auth:\n    type: wsse\n    username: u\n    password: p\n",
        "",
    );
    let edge = resolved_one(
        "GET",
        "https://x.test/p",
        &format!("  auth:\n{}", eg(CREDS)),
        "",
    );
    for t in Target::ALL {
        let code = generate(&wsse, *t);
        assert!(
            code.contains("WSSE auth is not included in this snippet."),
            "{}: {code}",
            t.id()
        );
        assert!(!code.contains("UsernameToken"), "{}", t.id());
        let code = generate(&edge, *t);
        assert!(
            code.contains("Akamai EdgeGrid auth is not included in this snippet."),
            "{}: {code}",
            t.id()
        );
        assert!(!code.contains("EG1-HMAC-SHA256"), "{}", t.id());
    }
}
