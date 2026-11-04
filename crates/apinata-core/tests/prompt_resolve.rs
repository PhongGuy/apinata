//! Answers as the top variable layer, and PromptRequired before any other resolve error.

use crate::common;

use apinata_core::{
    Collection, ContextOptions, ResolveError, VarContext, prepare, prompt_labels, resolve,
    resolve_prepared,
};
use common::{request, setup};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

fn http(url: &str, under_http: &str) -> String {
    format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n{under_http}"
    )
}

fn prompt(labels: &[&str]) -> ResolveError {
    ResolveError::PromptRequired {
        labels: labels.iter().map(|l| (*l).to_owned()).collect(),
    }
}

const PIN: &str = "  headers:\n    - name: X-Pin\n      value: \"{{?pin}}\"\n";

#[test]
fn answers_substitute_and_an_empty_answer_sends_empty() {
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &http("http://h.test/{{?Order id}}", PIN)),
    ]);
    let ctx = ctx.with_prompt_values([("Order id", "42"), ("pin", "")]);
    let r = resolve(request(&c, "r.yml"), &ctx).expect("resolves");
    assert_eq!(r.url(), "http://h.test/42");
    assert!(
        r.headers()
            .iter()
            .any(|(n, v)| n == "X-Pin" && v.is_empty())
    );
    assert!(!r.sensitive_values().iter().any(|v| v == "42"));
}

#[test]
fn every_missing_label_is_named_before_an_undefined_variable() {
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &http("http://h.test/{{nope}}/{{?a}}", PIN)),
    ]);
    let err = resolve(request(&c, "r.yml"), &ctx).unwrap_err();
    assert_eq!(err, prompt(&["pin", "a"]));
    assert_eq!(err.to_string(), "no value for prompt variables: pin, a");
    let half = ctx.with_prompt_values([("a", "1")]);
    assert_eq!(
        resolve(request(&c, "r.yml"), &half).unwrap_err(),
        prompt(&["pin"])
    );
    let all = ctx.with_prompt_values([("a", "1"), ("pin", "2")]);
    assert_eq!(
        resolve(request(&c, "r.yml"), &all).unwrap_err(),
        ResolveError::UndefinedVariable {
            name: "nope".into(),
            hint: None
        }
    );
}

#[test]
fn other_layers_never_answer_a_prompt() {
    let (_dir, c, _) = setup(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &http("http://h.test/{{?a}}", "")),
    ]);
    let ctx = VarContext::new(&c, ContextOptions::new().env_var("?a", "env")).expect("context");
    ctx.runtime_vars().set_text("?a", "runtime");
    assert_eq!(
        resolve(request(&c, "r.yml"), &ctx).unwrap_err(),
        prompt(&["a"])
    );
    let answered = ctx.with_prompt_values([("a", "ans")]);
    assert_eq!(
        resolve(request(&c, "r.yml"), &answered)
            .expect("resolves")
            .url(),
        "http://h.test/ans"
    );
}

#[test]
fn an_invalid_label_stays_an_undefined_variable() {
    for (url, name) in [
        ("http://h.test/{{?}}", "?"),
        ("http://h.test/{{? x}}", "? x"),
    ] {
        let (_dir, c, ctx) = setup(&[("opencollection.yml", ROOT), ("r.yml", &http(url, ""))]);
        assert_eq!(
            resolve(request(&c, "r.yml"), &ctx).unwrap_err(),
            ResolveError::UndefinedVariable {
                name: name.into(),
                hint: None
            }
        );
    }
}

#[test]
fn a_prompt_only_in_process_env_fails_as_a_leftover() {
    let (_dir, c, _) = setup(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &http("http://h.test/{{process.env.P}}", "")),
    ]);
    let options = ContextOptions::new().process_env([("P".to_owned(), "{{?late}}".to_owned())]);
    let ctx = VarContext::new(&c, options).expect("context");
    assert_eq!(
        resolve(request(&c, "r.yml"), &ctx).unwrap_err(),
        prompt(&["late"])
    );
    let answered = ctx.with_prompt_values([("late", "v")]);
    assert_eq!(
        resolve(request(&c, "r.yml"), &answered)
            .expect("resolves")
            .url(),
        "http://h.test/v"
    );
}

#[test]
fn a_prompt_a_script_writes_into_the_request_fails_as_a_leftover() {
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &http("http://h.test/", "")),
    ]);
    let mut prepared = prepare(request(&c, "r.yml"), &ctx).expect("prepares");
    prepared.set_header("X-Late", "{{?edited}}");
    assert_eq!(
        resolve_prepared(prepared, &ctx).unwrap_err(),
        prompt(&["edited"])
    );
}

#[test]
fn answers_do_not_change_which_certificate_is_scanned() {
    let root = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nconfig:\n  clientCertificates:\n    - domain: api.example.com\n      type: pkcs12\n      pkcs12FilePath: c.p12\n      passphrase: \"{{?pp}}\"\n";
    let dir = common::write_tree(&[
        ("opencollection.yml", root),
        ("r.yml", &http("https://{{?h}}/x", "")),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let answered = ctx.with_prompt_values([("h", "api.example.com")]);
    assert_eq!(prompt_labels(request(&c, "r.yml"), &answered), ["h"]);
    assert_eq!(
        resolve(request(&c, "r.yml"), &answered).unwrap_err(),
        prompt(&["pp"])
    );
}

#[test]
fn prompt_answers_stay_out_of_debug() {
    let (_dir, _c, ctx) = setup(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &http("http://h.test/{{?code}}", "")),
    ]);
    let answered = ctx.with_prompt_values([("code", "s3cr3t-answer")]);
    let shown = format!("{answered:?} {answered:#?}");
    assert!(!shown.contains("s3cr3t-answer"), "{shown}");
}
