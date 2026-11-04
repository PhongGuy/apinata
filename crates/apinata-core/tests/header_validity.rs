//! Headers that cannot be sent are named by resolve and refused at send, never quoting a value.

use crate::common;
use apinata_core::{
    Collection, ContextOptions, ExecError, HeaderNameProblem, ResolvedRequest, VarContext,
    invalid_header_name,
};
use common::{ROOT, at, write_tree};

/// One request with a single enabled header, resolved over `vars` (name, value, secret).
fn resolve_header(
    name: &str,
    value: &str,
    vars: &[(&str, &str)],
    sensitive: &[(&str, &str)],
) -> ResolvedRequest {
    let under = format!("  headers:\n    - name: \"{name}\"\n      value: \"{value}\"\n");
    resolve_under(&under, vars, sensitive)
}

/// Resolve one request whose `http:` block holds `under` after the method and URL.
fn resolve_under(
    under: &str,
    vars: &[(&str, &str)],
    sensitive: &[(&str, &str)],
) -> ResolvedRequest {
    let env = "name: dev\nvariables:\n".to_owned()
        + &vars
            .iter()
            .map(|(k, v)| format!("  - name: {k}\n    value: \"{v}\"\n"))
            .collect::<String>();
    let request = format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"http://x.test/\"\n{under}"
    );
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", &env),
        ("r.yml", &request),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let mut opts = ContextOptions::new().env("dev");
    for (k, v) in sensitive {
        opts = opts.sensitive_env_var(*k, *v);
    }
    let ctx = VarContext::new(&c, opts).expect("context");
    at(&c, &ctx, "r.yml").expect("resolves")
}

fn message(r: ResolvedRequest) -> String {
    r.header_problem().expect("must be flagged").to_owned()
}

#[test]
fn a_clean_header_resolves() {
    assert_eq!(
        resolve_header("X-A", "fine value", &[], &[]).header_problem(),
        None
    );
}

#[tokio::test]
async fn sending_a_flagged_request_fails_naming_the_header_before_any_connection() {
    let r = resolve_header("Bad Name", "v", &[], &[]);
    let err = common::executor().send(&r).await.expect_err("refused");
    assert!(matches!(err, ExecError::InvalidRequest { .. }), "{err:?}");
    assert!(
        err.to_string()
            .contains("header `Bad Name` contains a space in its name"),
        "{err}"
    );
}

#[test]
fn a_space_in_a_name_is_named() {
    assert_eq!(
        message(resolve_header("Bad Name", "v", &[], &[])),
        "header `Bad Name` contains a space in its name"
    );
}

#[test]
fn a_line_break_in_a_value_is_named() {
    assert_eq!(
        message(resolve_header("X-A", "a\\nb", &[], &[])),
        "value of header `X-A` contains a line break"
    );
}

#[test]
fn a_control_character_in_a_value_is_named() {
    assert_eq!(
        message(resolve_header("hv", "a\\x01b", &[], &[])),
        "value of header `hv` contains a control character"
    );
}

#[test]
fn a_name_that_expands_to_nothing_is_named() {
    assert_eq!(
        message(resolve_header("{{e}}", "v", &[("e", "")], &[])),
        "header `{{e}}` has an empty name"
    );
}

#[test]
fn another_invalid_name_byte_is_named() {
    assert_eq!(
        message(resolve_header("X(A)", "v", &[], &[])),
        "header `X(A)` contains the character `(` in its name, which header names cannot hold"
    );
}

#[test]
fn a_colon_in_a_name_is_named() {
    assert_eq!(
        message(resolve_header("X:A", "v", &[], &[])),
        "header `X:A` contains the character `:` in its name, which header names cannot hold"
    );
}

#[test]
fn a_value_that_is_bad_only_after_substitution_is_refused() {
    assert_eq!(
        message(resolve_header("X-A", "{{v}}", &[("v", "a\\nb")], &[])),
        "value of header `X-A` contains a line break"
    );
}

#[test]
fn a_name_that_is_bad_only_after_substitution_keeps_the_written_name() {
    let m = message(resolve_header("{{n}}", "v", &[("n", "x y")], &[]));
    assert_eq!(m, "header `{{n}}` contains a space in its name");
}

#[test]
fn a_secret_value_never_appears_in_the_message() {
    let m = message(resolve_header(
        "X-A",
        "{{s}}",
        &[],
        &[("s", "hunter2\nsecret-tail")],
    ));
    assert_eq!(m, "value of header `X-A` contains a line break");
    assert!(!m.contains("hunter2") && !m.contains("secret-tail"), "{m}");
}

#[test]
fn a_secret_name_never_appears_in_the_message() {
    let m = message(resolve_header("{{s}}", "v", &[], &[("s", "tok en-9")]));
    assert!(m.contains("{{s}}") && !m.contains("tok en"), "{m}");
}

#[test]
fn an_auth_header_named_by_a_secret_never_echoes_the_name() {
    let under =
        "  auth:\n    type: apikey\n    key: \"{{s}}\"\n    value: \"v\"\n    placement: header\n";
    let m = message(resolve_under(under, &[], &[("s", "tok(en-9")]));
    assert!(!m.contains("tok") && !m.contains('('), "{m}");
}

#[test]
fn an_auth_header_with_a_secret_value_is_named_without_the_value() {
    let under =
        "  auth:\n    type: apikey\n    key: X-Key\n    value: \"{{s}}\"\n    placement: header\n";
    let m = message(resolve_under(under, &[], &[("s", "tok\nen-9")]));
    assert_eq!(m, "value of header `X-Key` contains a line break");
}

#[test]
fn a_secret_name_with_a_bad_character_does_not_reveal_which_character() {
    let m = message(resolve_header("{{s}}", "v", &[], &[("s", "tok(en-9")]));
    assert!(
        m.contains("{{s}}") && !m.contains('(') && !m.contains("tok"),
        "{m}"
    );
}

#[test]
fn a_later_header_that_folds_into_an_earlier_one_is_named_as_written() {
    let under = "  headers:\n    - name: X-A\n      value: ok\n    - name: \"{{n}}\"\n      value: \"a\\nb\"\n";
    let m = message(resolve_under(under, &[("n", "x-a")], &[]));
    assert_eq!(m, "value of header `{{n}}` contains a line break");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_header_a_script_sets_is_named_when_it_cannot_be_sent() {
    use crate::script_support::{ROOT, on, script, scripted, send, write_tree};
    let code = "req.setHeader(\"Bad Name\", \"v\");";
    let text = scripted(
        "r",
        "http://127.0.0.1:9/",
        &script("before-request", code),
        "",
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    let result = send(dir.path(), "r.yml", on()).await;
    let error = format!("{:?}", result.error);
    assert!(
        error.contains("header `Bad Name` contains a space in its name"),
        "{error}"
    );
}

#[test]
fn every_token_character_class_is_accepted_in_a_written_name() {
    for ok in [
        "Accept",
        "x-trace_id-2",
        "X9",
        "!#$%&'*+-.^_`|~",
        "",
        "{{name}}",
        "X-{{a}}",
        "{{a}}{{b}}",
    ] {
        assert_eq!(invalid_header_name(ok), None, "{ok:?}");
    }
}

#[test]
fn a_character_outside_the_token_set_is_named_by_the_function() {
    for (name, bad) in [
        ("X Bad", ' '),
        ("X:Bad", ':'),
        ("X\tBad", '\t'),
        ("X\nBad", '\n'),
        ("X(Bad", '('),
        ("X@Bad", '@'),
        ("X\"Bad", '"'),
        ("Xé", 'é'),
        ("X {{a}}", ' '),
        ("{{a}", '{'),
    ] {
        assert_eq!(
            invalid_header_name(name),
            Some(HeaderNameProblem::Char(bad)),
            "{name:?}"
        );
    }
}
