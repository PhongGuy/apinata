//! Post-response variables: the merged list, the evaluated subset, where values go, and the run.

use crate::common;

use apinata_core::{
    ResolveError, Response, VarContext, VarOutcome, VarResult, apply_post_response_vars, resolve,
};
use common::{request, setup};

const ROOT_HEAD: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

/// One `set-variable` item under `actions:`; `extra` adds keys at the item's level.
fn action(name: &str, expression: &str, extra: &str) -> String {
    format!(
        "    - type: set-variable\n      phase: after-response\n      selector:\n        expression: \"{expression}\"\n        method: jsonq\n      variable:\n        name: \"{name}\"\n        scope: runtime\n{extra}"
    )
}

fn http(url: &str, under_http: &str, actions: &str) -> String {
    let runtime = if actions.is_empty() {
        String::new()
    } else {
        format!("runtime:\n  actions:\n{actions}")
    };
    format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\n{under_http}{runtime}"
    )
}

fn shape(o: &VarOutcome) -> (String, &'static str, Option<String>) {
    match o.result() {
        VarResult::Set => (o.name().to_owned(), "set", None),
        VarResult::Undefined => (o.name().to_owned(), "undefined", None),
        VarResult::Skipped { reason } => (o.name().to_owned(), "skipped", Some(reason.clone())),
        other => panic!("unexpected result {other:?}"),
    }
}

fn js(expression: &str) -> Option<String> {
    Some(format!("`{expression}` needs a JavaScript runtime"))
}

fn listed(ctx: &VarContext) -> Vec<(String, Option<String>)> {
    ctx.runtime_vars()
        .entries()
        .iter()
        .map(|e| (e.name().to_owned(), e.value().map(str::to_owned)))
        .collect()
}

#[test]
fn the_supported_subset_is_stored_and_the_rest_is_skipped() {
    let actions = [
        action("id", "res.body.id", ""),
        action("user", "res.body.user", ""),
        action("trace", "res.headers['x-trace']", ""),
        action("missing", "res.body.missing", ""),
        action("sum", "res.body.id + 1", ""),
        action("query", "res('id')", ""),
    ]
    .concat();
    let root = format!(
        "{ROOT_HEAD}request:\n  variables:\n    - name: missing\n      value: fallback\n  actions:\n{}",
        action("code", "res.status", "")
    );
    let use_them =
        "  headers:\n    - name: X-Use\n      value: \"{{id}}|{{code}}|{{user}}|{{trace}}\"\n";
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", &root),
        ("capture.yml", &http("http://h/login", "", &actions)),
        ("use.yml", &http("http://h/next", use_them, "")),
        ("gone.yml", &http("http://h/{{missing}}", "", "")),
    ]);
    let res = Response::new(201)
        .with_header("X-Trace", "t1")
        .with_text_body("{\"id\":7,\"user\":{\"name\":\"ann\"}}");
    let outcomes = apply_post_response_vars(request(&c, "capture.yml"), &res, &ctx);
    assert_eq!(
        outcomes.iter().map(shape).collect::<Vec<_>>(),
        [
            ("code".to_owned(), "set", None),
            ("id".to_owned(), "set", None),
            ("user".to_owned(), "set", None),
            ("trace".to_owned(), "set", None),
            ("missing".to_owned(), "undefined", None),
            ("sum".to_owned(), "skipped", js("res.body.id + 1")),
            ("query".to_owned(), "skipped", js("res('id')")),
        ]
    );
    let used = resolve(request(&c, "use.yml"), &ctx).expect("resolves");
    let value = used
        .headers()
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("x-use"))
        .map(|(_, v)| v.clone());
    assert_eq!(value.as_deref(), Some("7|201|{\"name\":\"ann\"}|t1"));
    assert!(matches!(
        resolve(request(&c, "gone.yml"), &ctx),
        Err(ResolveError::UndefinedVariable { name, .. }) if name == "missing"
    ));
}

#[test]
fn levels_replace_by_name_in_first_position_and_disabled_ones_do_not_count() {
    let root = format!(
        "{ROOT_HEAD}request:\n  actions:\n{}{}",
        action("code", "res.status", ""),
        action("first", "res.body.a", "")
    );
    let folder = format!(
        "info:\n  name: f\n  type: folder\nrequest:\n  actions:\n{}",
        action("code", "res.statusText", "")
    );
    let own = [
        action("code", "res.body.a", "      disabled: true\n"),
        action("own", "res.body.a", ""),
        action("off", "res.body.a", "      disabled: true\n"),
    ]
    .concat();
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", &root),
        ("f/folder.yml", &folder),
        ("f/r.yml", &http("http://h/", "", &own)),
    ]);
    let res = Response::new(200)
        .with_status_text("OK")
        .with_text_body("{\"a\":1}");
    let outcomes = apply_post_response_vars(request(&c, "f/r.yml"), &res, &ctx);
    let pairs: Vec<_> = outcomes
        .iter()
        .map(|o| (o.name(), o.expression()))
        .collect();
    assert_eq!(
        pairs,
        [
            ("code", "res.statusText"),
            ("first", "res.body.a"),
            ("own", "res.body.a")
        ]
    );
    assert_eq!(
        listed(&ctx),
        [
            ("code".to_owned(), Some("OK".to_owned())),
            ("first".to_owned(), Some("1".to_owned())),
            ("own".to_owned(), Some("1".to_owned())),
        ]
    );
}

#[test]
fn names_are_checked_after_the_expression_and_skips_keep_earlier_values() {
    let actions = [
        action("", "res.status", ""),
        action("a b", "res.status", ""),
        action("café", "res.status", ""),
        action("ok.name-1_x", "res.status", ""),
        action("kept", "res.body.id + 1", ""),
    ]
    .concat();
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", ROOT_HEAD),
        ("r.yml", &http("http://h/", "", &actions)),
    ]);
    ctx.runtime_vars().set_text("kept", "before");
    let outcomes = apply_post_response_vars(request(&c, "r.yml"), &Response::new(204), &ctx);
    let invalid = |n: &str| {
        Some(format!(
            "`{n}` is not a valid variable name; use ASCII letters, digits, `-`, `_` and `.`"
        ))
    };
    assert_eq!(
        outcomes.iter().map(shape).collect::<Vec<_>>(),
        [
            (
                String::new(),
                "skipped",
                Some("the variable has no name".to_owned())
            ),
            ("a b".to_owned(), "skipped", invalid("a b")),
            ("café".to_owned(), "skipped", invalid("café")),
            ("ok.name-1_x".to_owned(), "set", None),
            ("kept".to_owned(), "skipped", js("res.body.id + 1")),
        ]
    );
    assert_eq!(
        listed(&ctx),
        [
            ("kept".to_owned(), Some("before".to_owned())),
            ("ok.name-1_x".to_owned(), Some("204".to_owned())),
        ]
    );
}

#[test]
fn reading_through_undefined_stores_the_type_error_object() {
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", ROOT_HEAD),
        (
            "r.yml",
            &http(
                "http://h/",
                "",
                &action("deep", "res.body.missing.deeper", ""),
            ),
        ),
    ]);
    let res = Response::new(200).with_text_body("{}");
    let outcomes = apply_post_response_vars(request(&c, "r.yml"), &res, &ctx);
    assert_eq!(
        outcomes.iter().map(shape).collect::<Vec<_>>(),
        [("deep".to_owned(), "set", None)]
    );
    assert_eq!(
        listed(&ctx),
        [(
            "deep".to_owned(),
            Some("{\"name\":\"TypeError\",\"message\":\"cannot read property 'deeper' of undefined\"}".to_owned())
        )]
    );
}

#[test]
fn outcomes_read_as_one_line_each() {
    let actions = [
        action("a", "res.status", ""),
        action("b", "res.body.nope", ""),
        action("c", "res.status + 1", ""),
    ]
    .concat();
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", ROOT_HEAD),
        ("r.yml", &http("http://h/", "", &actions)),
    ]);
    let res = Response::new(200).with_text_body("{}");
    let lines: Vec<String> = apply_post_response_vars(request(&c, "r.yml"), &res, &ctx)
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        lines,
        [
            "post-response variable `a` set",
            "post-response variable `b` set to undefined",
            "post-response variable `c` not set: `res.status + 1` needs a JavaScript runtime",
        ]
    );
}

#[test]
fn a_truncated_body_skips_body_reads_but_not_status() {
    let actions = [
        action("id", "res.body.id", ""),
        action("code", "res.status", ""),
    ]
    .concat();
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", ROOT_HEAD),
        ("r.yml", &http("http://h/", "", &actions)),
    ]);
    let res = Response::new(200)
        .with_text_body("{\"id\":1}")
        .with_truncated(4);
    let outcomes = apply_post_response_vars(request(&c, "r.yml"), &res, &ctx);
    assert_eq!(
        outcomes.iter().map(shape).collect::<Vec<_>>(),
        [
            (
                "id".to_owned(),
                "skipped",
                Some("the response body was truncated at the size limit, so `res.body.id` was not evaluated".to_owned())
            ),
            ("code".to_owned(), "set", None),
        ]
    );
}

#[test]
fn nested_folders_merge_outer_before_inner_and_the_inner_expression_wins() {
    let root = format!(
        "{ROOT_HEAD}request:\n  actions:\n{}",
        action("x", "res.status", "")
    );
    let outer = format!(
        "info:\n  name: a\n  type: folder\nrequest:\n  actions:\n{}{}",
        action("x", "res.statusText", ""),
        action("outer", "res.status", "")
    );
    let inner = format!(
        "info:\n  name: b\n  type: folder\nrequest:\n  actions:\n{}{}",
        action("x", "res.body.a", ""),
        action("inner", "res.status", "")
    );
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", &root),
        ("a/folder.yml", &outer),
        ("a/b/folder.yml", &inner),
        ("a/b/r.yml", &http("http://h/", "", "")),
    ]);
    let res = Response::new(200)
        .with_status_text("OK")
        .with_text_body("{\"a\":1}");
    let outcomes = apply_post_response_vars(request(&c, "a/b/r.yml"), &res, &ctx);
    let pairs: Vec<_> = outcomes
        .iter()
        .map(|o| (o.name(), o.expression()))
        .collect();
    assert_eq!(
        pairs,
        [
            ("x", "res.body.a"),
            ("outer", "res.status"),
            ("inner", "res.status")
        ]
    );
}
