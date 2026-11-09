//! The variables a request can see, as editors show them: scope, rendered value, secrecy.

use crate::common;

use apinata_core::{
    Collection, ContextOptions, RequestFile, RuntimeVars, VarContext, VarScope, VariableInfo,
    VariableSet, dynamic_variables, placeholders, resolve,
};
use common::{request, write_tree};
use tempfile::TempDir;

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nrequest:\n  variables:\n    - name: host\n      value: c.test\n    - name: shared\n      value: collection\n    - name: base\n      value: \"{{host}}/v1\"\n    - name: n\n      value:\n        type: number\n        data: \"42\"\n    - name: $guid\n      value: shadowed\n    - name: process\n      value: shadowed\n    - name: loop\n      value: \"{{loop}}\"\n    - name: dangling\n      value: \"{{nope}}\"\n";
const DEV: &str = "name: dev\nvariables:\n  - name: shared\n    value: env\n  - name: api\n    value: \"{{token}}-x\"\n  - secret: true\n    name: unset\n";
const FOLDER: &str = "info:\n  name: a\n  type: folder\nrequest:\n  variables:\n    - name: shared\n      value: folder\n";
const IN_FOLDER: &str = "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"https://{{base}}/{{own}}\"\nruntime:\n  variables:\n    - name: own\n      value: request\n";
const TOP: &str =
    "info:\n  name: s\n  type: http\nhttp:\n  method: GET\n  url: \"https://{{shared}}\"\n";

/// The collection above, `dev` selected, `token` given as a sensitive override, a
/// controlled process environment and a runtime scope the test can write to.
fn fixture() -> (TempDir, Collection, VarContext, RuntimeVars) {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("a/folder.yml", FOLDER),
        ("a/r.yml", IN_FOLDER),
        ("s.yml", TOP),
        (".env", "DOT_KEY=dotval\ndot.ted=x\n"),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    assert!(c.load_errors().is_empty(), "{:?}", c.load_errors());
    let runtime = RuntimeVars::new();
    let options = ContextOptions::new()
        .env("dev")
        .sensitive_env_var("token", "s3cret-token")
        .process_env([("OS_ONLY", "os")])
        .runtime_vars(runtime.clone());
    let ctx = VarContext::new(&c, options).expect("context");
    (dir, c, ctx, runtime)
}

fn set_for(c: &Collection, ctx: &VarContext, rel: &str) -> VariableSet {
    ctx.variables(request(c, rel))
}

/// `(scope, value, secret)` for a defined name; panics on an undefined one.
fn info(set: &VariableSet, name: &str) -> (VarScope, Option<String>, bool) {
    let i: VariableInfo = set
        .get(name)
        .unwrap_or_else(|| panic!("{name} is undefined"));
    (i.scope(), i.value().map(str::to_owned), i.is_secret())
}

fn plain(scope: VarScope, value: &str) -> (VarScope, Option<String>, bool) {
    (scope, Some(value.to_owned()), false)
}

#[test]
fn each_name_reports_the_layer_that_wins() {
    let (_dir, c, ctx, _) = fixture();
    let r = set_for(&c, &ctx, "a/r.yml");
    assert_eq!(info(&r, "host"), plain(VarScope::Collection, "c.test"));
    assert_eq!(info(&r, "n"), plain(VarScope::Collection, "42"));
    assert_eq!(info(&r, "shared"), plain(VarScope::Folder, "folder"));
    assert_eq!(info(&r, "own"), plain(VarScope::Request, "request"));
    assert_eq!(info(&r, "__name__"), plain(VarScope::Environment, "dev"));
    let s = set_for(&c, &ctx, "s.yml");
    assert_eq!(info(&s, "shared"), plain(VarScope::Environment, "env"));
    assert!(
        s.get("own").is_none(),
        "another request's variables are not seen"
    );
}

#[test]
fn runtime_captures_win_and_are_read_live() {
    let (_dir, c, ctx, runtime) = fixture();
    let r = set_for(&c, &ctx, "a/r.yml");
    let before = r.revision();
    runtime.set_text("shared", "captured");
    assert_ne!(r.revision(), before, "a capture is a new revision");
    runtime.set_text("late", "v");
    assert_eq!(info(&r, "shared"), plain(VarScope::Runtime, "captured"));
    assert_eq!(info(&r, "late"), plain(VarScope::Runtime, "v"));
    assert!(r.names().contains(&"late".to_owned()));
}

#[test]
fn values_render_as_the_interpolator_renders_them() {
    let (_dir, c, ctx, _) = fixture();
    let r = set_for(&c, &ctx, "a/r.yml");
    assert_eq!(info(&r, "base"), plain(VarScope::Collection, "c.test/v1"));
    let sent = resolve(request(&c, "a/r.yml"), &ctx).expect("resolves");
    assert_eq!(sent.url(), "https://c.test/v1/request");
    // A value that cannot be expanded is shown as written, and is still defined.
    assert_eq!(info(&r, "loop"), plain(VarScope::Collection, "{{loop}}"));
    assert_eq!(
        info(&r, "dangling"),
        plain(VarScope::Collection, "{{nope}}")
    );
}

#[test]
fn secrets_never_carry_a_value() {
    let (_dir, c, ctx, _) = fixture();
    let r = set_for(&c, &ctx, "a/r.yml");
    assert_eq!(info(&r, "token"), (VarScope::Environment, None, true));
    // A plain variable whose value pulls in a secret is a secret too.
    assert_eq!(info(&r, "api"), (VarScope::Environment, None, true));
    // A secret with no value is undefined, as sending it would fail.
    assert!(r.get("unset").is_none());
    let shown = format!("{r:?} {:?} {:?}", r.get("token"), r.get("api"));
    assert!(!shown.contains("s3cret"), "{shown}");
}

#[test]
fn process_env_reads_the_dot_env_and_the_os_and_is_masked() {
    let (_dir, c, ctx, _) = fixture();
    let r = set_for(&c, &ctx, "a/r.yml");
    let masked = (VarScope::ProcessEnv, None, true);
    assert_eq!(info(&r, "process.env.DOT_KEY"), masked);
    assert_eq!(info(&r, "process.env.OS_ONLY"), masked);
    assert_eq!(info(&r, "process.env['dot.ted']"), masked);
    assert_eq!(
        info(&r, "process"),
        masked,
        "the namespace replaces a variable"
    );
    assert!(r.get("process.env.NOPE").is_none());
    let names = r.names();
    assert!(names.contains(&"process.env.DOT_KEY".to_owned()));
    for absent in ["process", "process.env.OS_ONLY", "process.env.dot.ted"] {
        assert!(!names.contains(&absent.to_owned()), "{absent} listed");
    }
}

#[test]
fn dynamic_names_win_and_are_listed_last() {
    let (_dir, c, ctx, _) = fixture();
    let r = set_for(&c, &ctx, "a/r.yml");
    assert_eq!(info(&r, "$guid"), (VarScope::Dynamic, None, false));
    assert!(r.get("$nope").is_none());
    let names = r.names();
    let dynamic: Vec<String> = dynamic_variables()
        .iter()
        .map(|n| format!("${n}"))
        .collect();
    assert_eq!(names[names.len() - dynamic.len()..], dynamic[..]);
    assert_eq!(names.iter().filter(|n| *n == "$guid").count(), 1);
}

#[test]
fn names_are_sorted_and_each_is_defined() {
    let (_dir, c, ctx, _) = fixture();
    let r = set_for(&c, &ctx, "a/r.yml");
    let names = r.names();
    let plain: Vec<&String> = names
        .iter()
        .filter(|n| !n.starts_with('$') && !n.starts_with("process.env."))
        .collect();
    let mut sorted = plain.clone();
    sorted.sort();
    assert_eq!(plain, sorted);
    for expected in ["host", "shared", "own", "token", "api", "__name__", "n"] {
        assert!(names.contains(&expected.to_owned()), "{expected} missing");
    }
    assert!(!names.contains(&"unset".to_owned()));
    for name in &names {
        assert!(r.get(name).is_some(), "{name} listed but undefined");
    }
}

#[test]
fn unknown_names_and_paths_are_undefined() {
    let (_dir, c, ctx, _) = fixture();
    let r = set_for(&c, &ctx, "a/r.yml");
    assert!(r.get("missing").is_none());
    assert!(r.get("host.deep").is_none());
}

#[test]
fn placeholders_match_the_resolvers_grammar() {
    assert_eq!(
        placeholders("a{{x}}b{{ y }}{{}}{{z"),
        vec![(1..6, "x"), (7..14, " y ")]
    );
    assert_eq!(placeholders("{{{a}}}"), vec![(0..6, "{a")]);
    assert_eq!(
        placeholders("{{$guid}}{{a.b[0]}}"),
        vec![(0..9, "$guid"), (9..19, "a.b[0]")]
    );
    assert_eq!(placeholders("é{{x}}"), vec![(2..7, "x")]);
    assert!(placeholders("no tokens { } }}").is_empty());
}

#[test]
fn a_set_crosses_threads() {
    fn shareable<T: Send + Sync + Clone + 'static>() {}
    shareable::<VariableSet>();
    shareable::<VariableInfo>();
}

/// The overview rows of `name`, top scope first, as `(scope, value, secret, shadowed)`.
fn rows(set: &VariableSet, name: &str) -> Vec<(VarScope, Option<String>, bool, bool)> {
    set.overview()
        .iter()
        .filter(|v| v.name() == name)
        .map(|v| {
            (
                v.scope(),
                v.value().map(str::to_owned),
                v.is_secret(),
                v.is_shadowed(),
            )
        })
        .collect()
}

#[test]
fn overview_lists_every_layer_and_marks_the_shadowed_ones() {
    let (_dir, c, ctx, runtime) = fixture();
    let r = set_for(&c, &ctx, "a/r.yml");
    let row = |s, v: &str, sh| (s, Some(v.to_owned()), false, sh);
    assert_eq!(
        rows(&r, "shared"),
        vec![
            row(VarScope::Folder, "folder", false),
            row(VarScope::Environment, "env", true),
            row(VarScope::Collection, "collection", true),
        ]
    );
    runtime.set_text("shared", "captured");
    assert_eq!(
        rows(&r, "shared")[0],
        row(VarScope::Runtime, "captured", false)
    );
    assert_eq!(rows(&r, "shared").len(), 4);
    assert_eq!(
        rows(&r, "own"),
        vec![row(VarScope::Request, "request", false)]
    );
}

#[test]
fn overview_masks_secrets_in_every_row() {
    let (_dir, c, ctx, _) = fixture();
    let r = set_for(&c, &ctx, "a/r.yml");
    assert_eq!(
        rows(&r, "token"),
        vec![(VarScope::Environment, None, true, false)]
    );
    for v in r.overview() {
        assert!(!format!("{v:?}").contains("s3cret"), "{v:?}");
    }
}

#[test]
fn overview_effective_values_equal_what_a_send_interpolates() {
    let names = ["host", "shared", "base", "n", "own", "__name__"];
    let headers: String = names
        .iter()
        .map(|n| format!("    - name: h-{n}\n      value: \"{{{{{n}}}}}\"\n"))
        .collect();
    let doc = format!(
        "info:\n  name: p\n  type: http\nhttp:\n  method: GET\n  url: \"https://x.test\"\n  headers:\n{headers}runtime:\n  variables:\n    - name: own\n      value: request\n"
    );
    let (dir, _, _, _) = fixture();
    std::fs::write(dir.path().join("a/probe.yml"), doc).expect("writes");
    let c = Collection::open(dir.path()).expect("opens");
    let options = ContextOptions::new().env("dev");
    let ctx = VarContext::new(&c, options).expect("context");
    let probe = request(&c, "a/probe.yml");
    let sent = resolve(probe, &ctx).expect("resolves");
    let set = ctx.variables(probe);
    for name in names {
        let effective: Vec<_> = set
            .overview()
            .into_iter()
            .filter(|v| v.name() == name && !v.is_shadowed())
            .collect();
        assert_eq!(effective.len(), 1, "{name}");
        let sent_value = sent
            .headers()
            .iter()
            .find(|(k, _)| *k == format!("h-{name}"))
            .map(|(_, v)| v.as_str());
        assert_eq!(effective[0].value(), sent_value, "{name}");
    }
}

const BODY_A: &str = "info:\n  name: r\n  type: http\nhttp:\n  method: POST\n  url: \"https://{{own}}\"\n  body:\n    type: text\n    data: a\nruntime:\n  variables:\n    - name: own\n      value: \"1\"\n";

/// The set `ctx` builds for the request file at `rel` once it holds `yaml`.
fn set_holding(dir: &TempDir, ctx: &VarContext, rel: &str, yaml: &str) -> VariableSet {
    let path = dir.path().join(rel);
    std::fs::write(&path, yaml).unwrap();
    ctx.variables(&RequestFile::load(&path).unwrap())
}

#[test]
fn a_body_edit_keeps_the_request_inputs_of_a_set() {
    let (dir, _c, ctx, _) = fixture();
    let before = set_holding(&dir, &ctx, "a/r.yml", BODY_A);
    let after = set_holding(
        &dir,
        &ctx,
        "a/r.yml",
        &BODY_A.replace("data: a", "data: ab"),
    );
    assert!(before.same_request_inputs(&after));
}

#[test]
fn a_request_variable_edit_changes_the_request_inputs() {
    let (dir, _c, ctx, _) = fixture();
    let before = set_holding(&dir, &ctx, "a/r.yml", BODY_A);
    let after = set_holding(&dir, &ctx, "a/r.yml", &BODY_A.replace("\"1\"", "\"2\""));
    assert!(!before.same_request_inputs(&after));
}

#[test]
fn a_request_variable_of_another_yaml_type_changes_the_request_inputs() {
    let (dir, _c, ctx, _) = fixture();
    let before = set_holding(&dir, &ctx, "a/r.yml", BODY_A);
    let after = set_holding(&dir, &ctx, "a/r.yml", &BODY_A.replace("\"1\"", "1"));
    assert!(!before.same_request_inputs(&after));
}

#[test]
fn another_path_changes_the_request_inputs() {
    let (dir, _c, ctx, _) = fixture();
    let here = set_holding(&dir, &ctx, "a/r.yml", BODY_A);
    let there = set_holding(&dir, &ctx, "s.yml", BODY_A);
    assert!(!here.same_request_inputs(&there));
}
