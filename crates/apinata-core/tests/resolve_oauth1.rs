//! OAuth 1.0 through resolve: variables, secrets, the private key and its errors.

use crate::common;

use apinata_core::{Collection, ContextOptions, ResolveError, VarContext, named_secrets};
use common::{at, request, setup, write_tree};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nrequest:\n  variables:\n    - name: ck\n      value: consumer\n";
const PKCS8: &str = include_str!("fixtures/oauth1/rsa-pkcs8.pem");
const PKCS1: &str = include_str!("fixtures/oauth1/rsa-pkcs1.pem");

fn http(lines: &str) -> String {
    format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"https://x.test/\"\n  auth:\n    type: oauth1\n{lines}"
    )
}

fn resolves(files: &[(&str, &str)]) -> Result<(), ResolveError> {
    let (_d, c, ctx) = setup(files);
    at(&c, &ctx, "r.yml").map(|_| ())
}

fn reason(files: &[(&str, &str)]) -> String {
    match resolves(files) {
        Err(ResolveError::OAuth1 { reason }) => reason,
        other => panic!("expected an OAuth 1.0 error: {other:?}"),
    }
}

fn rsa_text(value: &str) -> String {
    format!(
        "    signatureMethod: RSA-SHA256\n    privateKey:\n      type: text\n      value: '{value}'\n"
    )
}

#[test]
fn hmac_resolves_own_and_inherited() {
    let own = http("    consumerKey: \"{{ck}}\"\n    consumerSecret: cs\n");
    assert_eq!(
        resolves(&[("opencollection.yml", ROOT), ("r.yml", &own)]),
        Ok(())
    );
    let root = format!("{ROOT}  auth:\n    type: oauth1\n    consumerKey: \"{{{{ck}}}}\"\n");
    let inherit = "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"https://x.test/\"\n  auth: inherit\n";
    assert_eq!(
        resolves(&[("opencollection.yml", &root), ("r.yml", inherit)]),
        Ok(())
    );
}

#[test]
fn an_undefined_variable_in_any_field_fails() {
    let fields = [
        "consumerKey",
        "consumerSecret",
        "accessToken",
        "accessTokenSecret",
        "callbackUrl",
        "verifier",
        "timestamp",
        "nonce",
        "version",
        "realm",
    ];
    for field in fields {
        let r = http(&format!("    {field}: \"{{{{nope}}}}\"\n"));
        let got = resolves(&[("opencollection.yml", ROOT), ("r.yml", &r)]);
        assert!(
            matches!(&got, Err(ResolveError::UndefinedVariable { name, .. }) if name == "nope"),
            "{field}: {got:?}"
        );
    }
    let r = http(&rsa_text("{{nope}}"));
    assert!(matches!(
        resolves(&[("opencollection.yml", ROOT), ("r.yml", &r)]),
        Err(ResolveError::UndefinedVariable { .. })
    ));
}

#[test]
fn unknown_methods_placements_and_missing_keys_fail_before_sending() {
    let files = |r: &str| [("opencollection.yml", ROOT.to_owned()), ("r.yml", http(r))];
    let run = |r: &str| {
        let f = files(r);
        reason(&[(f[0].0, f[0].1.as_str()), (f[1].0, f[1].1.as_str())])
    };
    assert!(run("    signatureMethod: HMAC-MD5\n").contains("`HMAC-MD5`"));
    assert!(run("    placement: cookie\n").contains("`cookie`"));
    assert!(run("    signatureMethod: RSA-SHA1\n").contains("RSA-SHA1 needs a private key"));
    let bad = run(&rsa_text("not a key"));
    assert!(bad.contains("not an RSA private key"), "{bad}");
    assert!(!bad.contains("not a key"), "{bad}");
    let missing = run(
        "    signatureMethod: RSA-SHA512\n    privateKey:\n      type: file\n      value: keys/none.pem\n",
    );
    assert!(missing.contains("keys/none.pem"), "{missing}");
    assert!(
        ResolveError::OAuth1 { reason: "x".into() }
            .to_string()
            .starts_with("OAuth 1.0: ")
    );
}

#[test]
fn keys_load_from_text_with_escaped_newlines_and_from_files() {
    let escaped = PKCS8.trim().replace('\n', "\\n");
    let r = http(&rsa_text(&escaped));
    assert_eq!(
        resolves(&[("opencollection.yml", ROOT), ("r.yml", &r)]),
        Ok(())
    );
    let file = |path: &str| {
        http(&format!(
            "    signatureMethod: RSA-SHA1\n    privateKey:\n      type: file\n      value: \"{path}\"\n"
        ))
    };
    for (key, text) in [("keys/p8.pem", PKCS8), ("keys/p1.pem", PKCS1)] {
        let r = file(key);
        assert_eq!(
            resolves(&[("opencollection.yml", ROOT), (key, text), ("r.yml", &r)]),
            Ok(()),
            "{key}"
        );
    }
    let outside = write_tree(&[("abs.pem", PKCS1)]);
    let r = file(&outside.path().join("abs.pem").display().to_string());
    assert_eq!(
        resolves(&[("opencollection.yml", ROOT), ("r.yml", &r)]),
        Ok(())
    );
    // HMAC ignores the key, even one that would not parse.
    let r = http("    privateKey: garbage\n");
    assert_eq!(
        resolves(&[("opencollection.yml", ROOT), ("r.yml", &r)]),
        Ok(())
    );
}

#[test]
fn credentials_are_sensitive_and_secret_variables_are_named() {
    let env = "name: e\nvariables:\n  - name: cs\n    secret: true\n";
    let r = http(
        "    consumerSecret: \"{{cs}}\"\n    accessToken: tk\n    accessTokenSecret: plain-token-secret\n",
    );
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/e.yml", env),
        ("r.yml", &r),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let ctx = VarContext::new(
        &c,
        ContextOptions::new()
            .env("e")
            .sensitive_env_var("cs", "env-consumer-secret"),
    )
    .expect("context");
    let found = named_secrets(request(&c, "r.yml"), &ctx).unwrap();
    let named = found
        .iter()
        .find(|s| s.value() == "env-consumer-secret")
        .expect("named");
    assert_eq!(named.label(), "cs");
    assert!(
        found.iter().any(|s| s.value() == "plain-token-secret"),
        "{found:?}"
    );
    let r = at(&c, &ctx, "r.yml").expect("resolves");
    let debug = format!("{r:?}");
    assert!(
        !debug.contains("env-consumer-secret") && !debug.contains("plain-token-secret"),
        "{debug}"
    );
}

#[test]
fn hmac_ignores_a_stale_private_key() {
    let r = http(
        "    signatureMethod: HMAC-SHA1\n    privateKey:\n      type: text\n      value: \"{{nope}}\"\n",
    );
    assert_eq!(
        resolves(&[("opencollection.yml", ROOT), ("r.yml", &r)]),
        Ok(())
    );
}

#[test]
fn plaintext_without_secrets_registers_no_sensitive_value() {
    // A token secret signs only beside a token, so the second request also signs as a bare `&`.
    for lines in ["", "    accessTokenSecret: token-secret\n"] {
        let r = http(&format!(
            "    consumerKey: ck\n    signatureMethod: PLAINTEXT\n{lines}"
        ));
        let (_d, c, ctx) = setup(&[("opencollection.yml", ROOT), ("r.yml", &r)]);
        let resolved = at(&c, &ctx, "r.yml").expect("resolves");
        let values = resolved.sensitive_values();
        assert!(!values.iter().any(|v| v == "&" || v == "%26"), "{values:?}");
    }
}
