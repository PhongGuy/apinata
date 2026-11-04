//! The runtime scope: precedence, sharing between context clones, listing and redaction.

use crate::common;

use apinata_core::{Collection, ContextOptions, RuntimeVars, VarContext, resolve};
use common::{request, write_tree};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nrequest:\n  variables:\n    - name: token\n      value: from-collection\n";

fn req(url: &str, rest: &str) -> String {
    format!("info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\n{rest}")
}

fn url(c: &Collection, ctx: &VarContext) -> String {
    resolve(request(c, "r.yml"), ctx)
        .expect("resolves")
        .url()
        .to_owned()
}

#[test]
fn runtime_variables_beat_the_request_layer_until_cleared() {
    let own = "runtime:\n  variables:\n    - name: token\n      value: from-request\n";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &req("http://h/{{token}}", own)),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let runtime = RuntimeVars::new();
    let ctx =
        VarContext::new(&c, ContextOptions::new().runtime_vars(runtime.clone())).expect("context");
    assert_eq!(url(&c, &ctx), "http://h/from-request");
    runtime.set_text("token", "from-runtime");
    assert_eq!(url(&c, &ctx), "http://h/from-runtime");
    runtime.clear();
    assert_eq!(url(&c, &ctx), "http://h/from-request");
}

#[test]
fn a_context_owns_a_fresh_store_that_its_clones_share() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &req("http://h/{{token}}", "")),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let other = VarContext::new(&c, ContextOptions::new()).expect("context");
    let clone = ctx.clone();
    ctx.runtime_vars().set_text("token", "shared");
    assert_eq!(url(&c, &clone), "http://h/shared");
    assert_eq!(url(&c, &other), "http://h/from-collection");
}

#[test]
fn entries_list_by_name_and_debug_shows_no_values() {
    let vars = RuntimeVars::new();
    assert!(vars.is_empty());
    vars.set_text("b", "2");
    vars.set_text("a", "hush-hush");
    let listed: Vec<(String, Option<String>)> = vars
        .entries()
        .iter()
        .map(|e| (e.name().to_owned(), e.value().map(str::to_owned)))
        .collect();
    assert_eq!(
        listed,
        [
            ("a".to_owned(), Some("hush-hush".to_owned())),
            ("b".to_owned(), Some("2".to_owned()))
        ]
    );
    assert_eq!(vars.len(), 2);
    let debug = format!("{vars:?} {:?}", vars.entries());
    assert!(
        debug.contains("\"a\"") && !debug.contains("hush-hush"),
        "{debug}"
    );
}

#[test]
fn a_runtime_variable_shadows_a_sensitive_override() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &req("http://h/{{token}}", "")),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let ctx = VarContext::new(
        &c,
        ContextOptions::new().sensitive_env_var("token", "s3cret"),
    )
    .expect("context");
    assert_eq!(
        resolve(request(&c, "r.yml"), &ctx)
            .expect("resolves")
            .sensitive_values(),
        ["s3cret"]
    );
    ctx.runtime_vars().set_text("token", "captured");
    let resolved = resolve(request(&c, "r.yml"), &ctx).expect("resolves");
    assert_eq!(resolved.url(), "http://h/captured");
    assert!(resolved.sensitive_values().is_empty());
}

#[test]
fn unset_forgets_one_variable_and_says_whether_it_was_set() {
    let runtime = RuntimeVars::new();
    runtime.set_text("a", "1");
    runtime.set_text("b", "2");
    assert!(runtime.unset("a"));
    assert!(!runtime.unset("a"));
    let names: Vec<String> = runtime
        .entries()
        .iter()
        .map(|v| v.name().to_owned())
        .collect();
    assert_eq!(names, ["b"]);
}

// A sensitive mark follows its variable: unsetting the name or clearing the store forgets it,
// and every clone of the store sees it.
#[test]
fn a_sensitive_mark_lasts_until_the_variable_is_forgotten() {
    let vars = RuntimeVars::new();
    vars.set_text("a", "1");
    vars.set_text("b", "2");
    let clone = vars.clone();
    vars.mark_sensitive("a");
    vars.mark_sensitive("b");
    assert!(clone.is_sensitive("a") && clone.is_sensitive("b"));
    assert!(!clone.is_sensitive("c"));
    assert!(vars.unset("a"));
    assert!(!vars.is_sensitive("a") && vars.is_sensitive("b"));
    vars.clear();
    assert!(!vars.is_sensitive("b"));
}
