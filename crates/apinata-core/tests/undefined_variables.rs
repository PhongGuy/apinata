//! `undefined_variables`: the names a request's send would refuse, as `check` reports them.

use crate::common::{ROOT, write_tree};
use apinata_core::{
    Collection, ContextOptions, ItemPath, RuntimeVars, VarContext, undefined_variables,
};

const DEV: &str = "name: dev\nvariables:\n  - name: host\n    value: example.test\n";

fn req(url: &str, extra: &str) -> String {
    format!(
        "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n{extra}"
    )
}

fn script(code: &str) -> String {
    format!("runtime:\n  scripts:\n    - type: after-response\n      code: |-\n        {code}\n")
}

fn undefined(files: &[(&str, &str)], options: ContextOptions) -> Vec<String> {
    let dir = write_tree(files);
    let c = Collection::open(dir.path()).expect("opens");
    let ctx = VarContext::new(&c, options).expect("context");
    let req = c
        .request(&ItemPath::new("a.yml").unwrap())
        .expect("request");
    undefined_variables(&c, req, &ctx)
}

#[test]
fn every_undefined_name_is_listed_sorted_once() {
    let url = "http://{{host}}/{{zeta}}/{{alpha}}?q={{zeta}}";
    let found = undefined(
        &[("opencollection.yml", ROOT), ("a.yml", &req(url, ""))],
        ContextOptions::new(),
    );
    assert_eq!(found, ["alpha", "host", "zeta"]);
}

#[test]
fn a_name_the_environment_defines_is_not_listed() {
    let files = [
        ("opencollection.yml", ROOT),
        ("a.yml", &req("http://{{host}}/x", "") as &str),
        ("environments/dev.yml", DEV),
    ];
    assert_eq!(undefined(&files, ContextOptions::new()), ["host"]);
    assert!(undefined(&files, ContextOptions::new().env("dev")).is_empty());
}

#[test]
fn a_runtime_value_defines_the_name() {
    let files = [
        ("opencollection.yml", ROOT),
        ("a.yml", &req("http://x/{{token}}", "") as &str),
    ];
    let runtime = RuntimeVars::new();
    assert_eq!(
        undefined(&files, ContextOptions::new().runtime_vars(runtime.clone())),
        ["token"]
    );
    runtime.set_text("token", "t");
    assert!(undefined(&files, ContextOptions::new().runtime_vars(runtime)).is_empty());
}

#[test]
fn a_name_a_script_sets_is_not_listed() {
    let found = undefined(
        &[
            ("opencollection.yml", ROOT),
            (
                "a.yml",
                &req(
                    "http://x/{{later}}/{{other}}",
                    &script("bru.setVar('later', 1)"),
                ),
            ),
        ],
        ContextOptions::new(),
    );
    assert_eq!(found, ["other"]);
}

#[test]
fn a_process_env_name_is_not_listed() {
    let found = undefined(
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &req("http://x/{{process.env.HOME_X}}", "")),
        ],
        ContextOptions::new().process_env([("A", "b")]),
    );
    assert!(found.is_empty(), "{found:?}");
}
