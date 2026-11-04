use crate::support;

use std::sync::Arc;

use apinata_app::var_complete::{Choice, Query, accept, choices, filter, query_at};
use apinata_app::var_tokens::SECRET_DOTS;
use apinata_core::{Collection, ContextOptions, VarContext, VariableSet};
use support::*;

const VARS_ROOT: &str = "opencollection: 1.0.0\ninfo:\n  name: c\nrequest:\n  variables:\n    - name: host\n      value: c.test\n    - name: hostname\n      value: h.test\n    - name: empty\n      value: \"\"\n";
const DEV: &str = "name: dev\nvariables:\n  - name: region\n    value: eu\n";

/// Collection `host`, `hostname` and `empty`, `dev`'s `region` and `__name__`, a secret
/// `token`; no process environment.
fn vars() -> (tempfile::TempDir, Arc<VariableSet>) {
    let dir = write_tree(&[
        ("opencollection.yml", VARS_ROOT),
        ("environments/dev.yml", DEV),
        ("a.yml", &request("a", "x")),
    ]);
    let c = Collection::open(dir.path()).unwrap();
    let options = ContextOptions::new()
        .env("dev")
        .sensitive_env_var("token", "s3cret")
        .process_env(Vec::<(String, String)>::new());
    let ctx = VarContext::new(&c, options).unwrap();
    let set = ctx.variables(c.request(&ip("a.yml")).unwrap());
    (dir, Arc::new(set))
}

fn choice(name: &str, dynamic: bool) -> Choice {
    Choice {
        name: name.to_owned(),
        scope: if dynamic { "Dynamic" } else { "Collection" },
        preview: String::new(),
        dynamic,
    }
}

#[test]
fn a_query_is_the_partial_name_after_an_open_pair() {
    assert_eq!(
        query_at("https://{{ho", 12),
        Some(Query {
            start: 8,
            replace: 10..12,
            typed: "ho".into()
        })
    );
    assert_eq!(
        query_at("{{", 2),
        Some(Query {
            start: 0,
            replace: 2..2,
            typed: String::new()
        })
    );
    // Inside a name the rest of it is replaced too, up to the closing braces.
    assert_eq!(
        query_at("{{host}}", 4),
        Some(Query {
            start: 0,
            replace: 2..6,
            typed: "ho".into()
        })
    );
    assert_eq!(query_at("{{host}}", 8), None);
    assert_eq!(query_at("{{a b", 5), None);
    assert_eq!(query_at("{h", 2), None);
    assert_eq!(query_at("{{é", 3), None, "not a char boundary");
    // Names may use any letters, before and after the cursor.
    assert_eq!(
        query_at("{{pré", 6),
        Some(Query {
            start: 0,
            replace: 2..6,
            typed: "pré".into()
        })
    );
    assert_eq!(
        query_at("{{prénom}}", 4),
        Some(Query {
            start: 0,
            replace: 2..9,
            typed: "pr".into()
        })
    );
    // A quote after the name is not part of it, so accepting keeps it.
    assert_eq!(
        query_at(r#"{"a": "{{ite"}"#, 12),
        Some(Query {
            start: 7,
            replace: 9..12,
            typed: "ite".into()
        })
    );
}

#[test]
fn accepting_closes_the_pair_once() {
    let open = query_at("x{{ho", 5).unwrap();
    assert_eq!(accept("x{{ho", &open, "host"), (3..5, "host}}".into(), 9));
    let closed = query_at("x{{ho}}", 5).unwrap();
    assert_eq!(accept("x{{ho}}", &closed, "host"), (3..5, "host".into(), 9));
    let inside = query_at("{{hoxx}}/", 4).unwrap();
    assert_eq!(
        accept("{{hoxx}}/", &inside, "host"),
        (2..6, "host".into(), 8)
    );
    // One brace already there: add only the second one.
    let half = query_at("{{ho}", 4).unwrap();
    assert_eq!(accept("{{ho}", &half, "host"), (2..4, "host}".into(), 8));
}

#[test]
fn choices_preview_values_and_mask_secrets() {
    let (_dir, vars) = vars();
    let all = choices(&vars);
    let plain: Vec<(&str, &str, &str, bool)> = all
        .iter()
        .take(6)
        .map(|c| (c.name.as_str(), c.scope, c.preview.as_str(), c.dynamic))
        .collect();
    assert_eq!(
        plain,
        [
            ("__name__", "Environment", "dev", false),
            ("empty", "Collection", "", false),
            ("host", "Collection", "c.test", false),
            ("hostname", "Collection", "h.test", false),
            ("region", "Environment", "eu", false),
            ("token", "Environment", SECRET_DOTS, false),
        ]
    );
    let dynamic: Vec<&Choice> = all.iter().filter(|c| c.dynamic).collect();
    assert_eq!(dynamic.len(), all.len() - 6);
    assert_eq!(dynamic[0].name, "$guid");
    assert_eq!(dynamic[0].preview, "");
}

#[test]
fn filtering_ranks_prefixes_first_and_keeps_dynamic_names_last() {
    let all = [
        choice("ahost", false),
        choice("host", false),
        choice("Hostname", false),
        choice("region", false),
        choice("$hostish", true),
    ];
    let names = |typed: &str| -> Vec<String> {
        filter(&all, typed)
            .into_iter()
            .map(|c| c.name.clone())
            .collect()
    };
    assert_eq!(names("ho"), ["host", "Hostname", "ahost", "$hostish"]);
    assert_eq!(names("HOSTN"), ["Hostname"]);
    assert_eq!(names("$"), ["$hostish"]);
    assert_eq!(names("").len(), 5);
    assert!(names("zz").is_empty());
}
