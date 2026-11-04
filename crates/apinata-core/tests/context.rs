//! Variable context: layer precedence, environments, overrides and URL resolution.

use crate::common;

use apinata_core::{
    Collection, ContextError, ContextOptions, IterationData, RequestFile, ResolveError,
    UndefinedHint, VarContext, resolve,
};
use common::{request, write_tree};
use std::fs;
use tempfile::TempDir;

fn url_of(c: &Collection, ctx: &VarContext, rel: &str) -> Result<String, ResolveError> {
    resolve(request(c, rel), ctx).map(|r| r.url().to_owned())
}

fn http(url: &str, rest: &str) -> String {
    format!("info:\n  name: r\n  type: http\nhttp:\n  method: post\n  url: \"{url}\"\n{rest}")
}

fn runtime_var(name: &str, value: &str, disabled: bool) -> String {
    let off = if disabled {
        "      disabled: true\n"
    } else {
        ""
    };
    format!("runtime:\n  variables:\n    - name: {name}\n      value: {value}\n{off}")
}

fn folder(name: &str, v: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: folder\nrequest:\n  variables:\n    - name: v\n      value: {v}\n"
    )
}

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nrequest:\n  variables:\n    - name: v\n      value: collection\n    - name: num\n      value:\n        type: number\n        data: \"42\"\n";
const DEV: &str =
    "name: dev\nvariables:\n  - name: v\n    value: env\n  - secret: true\n    name: secret_key\n";
const BROKEN: &str = "name: broken\nextends: nope\nvariables: []\n";

fn layered() -> (TempDir, Collection) {
    let (outer, inner) = (folder("a", "outer"), folder("b", "inner"));
    let plain = http("https://x.test/{{v}}", "");
    let own = http("https://x.test/{{v}}", &runtime_var("v", "request", false));
    let off = http("https://x.test/{{v}}", &runtime_var("v", "off", true));
    let secret = http("https://x.test/{{secret_key}}", "");
    let named = http("https://x.test/{{num}}/{{__name__}}", "");
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("environments/broken.yml", BROKEN),
        ("a/folder.yml", &outer),
        ("a/b/folder.yml", &inner),
        ("s.yml", &plain),
        ("k.yml", &secret),
        ("n.yml", &named),
        ("a/p.yml", &plain),
        ("a/b/q.yml", &plain),
        ("a/b/r.yml", &own),
        ("a/b/d.yml", &off),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    assert!(c.load_errors().is_empty(), "{:?}", c.load_errors());
    (dir, c)
}

fn dev() -> ContextOptions {
    ContextOptions::new().env("dev")
}

#[test]
fn variables_follow_brunos_precedence() {
    let (_dir, c) = layered();
    let ctx = VarContext::new(&c, dev()).expect("context");
    let cases = [
        ("s.yml", "env"),
        ("a/p.yml", "outer"),
        ("a/b/q.yml", "inner"),
        ("a/b/r.yml", "request"),
        ("a/b/d.yml", "inner"),
    ];
    for (rel, expected) in cases {
        assert_eq!(
            url_of(&c, &ctx, rel),
            Ok(format!("https://x.test/{expected}")),
            "{rel}"
        );
    }
    let bare = VarContext::new(&c, ContextOptions::new()).expect("context");
    assert_eq!(
        url_of(&c, &bare, "s.yml"),
        Ok("https://x.test/collection".to_owned())
    );
    let named = resolve(request(&c, "n.yml"), &ctx).expect("resolves");
    assert_eq!(
        (named.url(), named.method(), ctx.environment()),
        ("https://x.test/42/dev", "POST", Some("dev"))
    );
}

#[test]
fn env_var_overrides_sit_at_the_environment_level_and_supply_secrets() {
    let (_dir, c) = layered();
    let ctx = VarContext::new(&c, dev().env_var("v", "cli")).expect("context");
    assert_eq!(
        url_of(&c, &ctx, "s.yml"),
        Ok("https://x.test/cli".to_owned())
    );
    assert_eq!(
        url_of(&c, &ctx, "a/p.yml"),
        Ok("https://x.test/outer".to_owned())
    );
    let no_env = VarContext::new(&c, ContextOptions::new().env_var("v", "cli")).expect("context");
    assert_eq!(
        url_of(&c, &no_env, "s.yml"),
        Ok("https://x.test/cli".to_owned())
    );
    // A secret has no value on disk: it resolves only through an override.
    let bare = VarContext::new(&c, dev()).expect("context");
    assert_eq!(
        url_of(&c, &bare, "k.yml"),
        Err(ResolveError::UndefinedVariable {
            name: "secret_key".to_owned(),
            hint: Some(UndefinedHint::SecretWithoutValue {
                environment: "dev".to_owned()
            })
        })
    );
    let supplied = VarContext::new(&c, dev().env_var("secret_key", "s3")).expect("context");
    assert_eq!(
        url_of(&c, &supplied, "k.yml"),
        Ok("https://x.test/s3".to_owned())
    );
}

#[test]
fn unknown_environments_are_an_error_and_broken_chains_warn() {
    let (_dir, c) = layered();
    match VarContext::new(&c, ContextOptions::new().env("prod")) {
        Err(ContextError::EnvironmentNotFound { name, .. }) => assert_eq!(name, "prod"),
        other => panic!("expected EnvironmentNotFound, got {other:?}"),
    }
    let ctx = VarContext::new(&c, ContextOptions::new().env("broken")).expect("context");
    assert!(
        ctx.warnings().iter().any(|w| w.contains("nope")),
        "{:?}",
        ctx.warnings()
    );
}

#[test]
fn unresolved_and_cyclic_variables_fail_the_request() {
    let missing = http("https://x.test/{{missing}}", "");
    let cyclic = http(
        "https://x.test/{{a}}",
        "runtime:\n  variables:\n    - name: a\n      value: \"{{b}}\"\n    - name: b\n      value: \"{{a}}\"\n",
    );
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("m.yml", &missing),
        ("c.yml", &cyclic),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    assert_eq!(
        url_of(&c, &ctx, "m.yml"),
        Err(ResolveError::UndefinedVariable {
            name: "missing".to_owned(),
            hint: None
        })
    );
    assert_eq!(
        url_of(&c, &ctx, "c.yml"),
        Err(ResolveError::CyclicVariable {
            name: "a".to_owned()
        })
    );
}

#[test]
fn folder_layers_survive_a_different_spelling_of_the_root() {
    let (dir, _) = layered();
    fs::create_dir(dir.path().join("x")).expect("mkdir");
    let c = Collection::open(dir.path().join("x").join("..")).expect("opens");
    let ctx = VarContext::new(&c, dev()).expect("context");
    let loose = RequestFile::load(dir.path().join("a/b/q.yml")).expect("loads");
    assert_eq!(
        resolve(&loose, &ctx).map(|r| r.url().to_owned()),
        Ok("https://x.test/inner".to_owned())
    );
}

#[test]
fn a_request_outside_the_collection_gets_no_folder_layers() {
    let (_dir, c) = layered();
    let other = write_tree(&[("a/p.yml", &http("https://x.test/{{v}}", ""))]);
    let loose = RequestFile::load(other.path().join("a/p.yml")).expect("loads");
    let ctx = VarContext::new(&c, dev()).expect("context");
    assert_eq!(
        resolve(&loose, &ctx).map(|r| r.url().to_owned()),
        Ok("https://x.test/env".to_owned())
    );
}

#[test]
fn debug_output_redacts_override_and_process_values() {
    let (_dir, c) = layered();
    let options = dev()
        .env_var("secret_key", "s3cr3t-override")
        .process_env([("TOKEN", "hunter2-process")]);
    let shown = format!("{options:?}");
    assert!(shown.contains("secret_key"), "{shown}");
    assert!(
        !shown.contains("s3cr3t-override") && !shown.contains("hunter2-process"),
        "{shown}"
    );
    let ctx = VarContext::new(&c, options).expect("context");
    let shown = format!("{ctx:?}");
    assert!(
        !shown.contains("s3cr3t-override") && !shown.contains("hunter2-process"),
        "{shown}"
    );
}

#[test]
fn a_selected_environment_that_failed_to_load_says_so() {
    let dir = write_tree(&[
        (
            "opencollection.yml",
            "opencollection: \"1.0.0\"\ninfo:\n  name: t\n",
        ),
        ("environments/broken.yml", "variables: [unclosed\n"),
    ]);
    let c = Collection::open(dir.path()).expect("open");
    let err = VarContext::new(&c, ContextOptions::new().env("broken")).expect_err("fails");
    assert!(
        matches!(&err, ContextError::EnvironmentFailedToLoad { name, .. } if name == "broken"),
        "{err:?}"
    );
    let shown = err.to_string();
    assert!(
        shown.starts_with("environment `broken` failed to load: "),
        "{shown}"
    );
}

#[test]
fn an_iteration_context_binds_the_row_and_the_index_variables() {
    let url = "https://x.test/{{host}}/{{iteration}}/{{iterationCount}}";
    let dir = write_tree(&[("opencollection.yml", ROOT), ("i.yml", &http(url, ""))]);
    let c = Collection::open(dir.path()).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let data = IterationData::from_csv("host,iterationCount\na,x\nb,y\n").expect("rows");
    assert_eq!(
        url_of(&c, &ctx.for_iteration(Some(&data), 1, 2), "i.yml"),
        Ok("https://x.test/b/1/y".to_owned())
    );
    // Without data, a pass binds only the index variables, so `host` stays undefined.
    assert!(url_of(&c, &ctx.for_iteration(None, 0, 1), "i.yml").is_err());
    let counted = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "j.yml",
            &http("https://x.test/{{iteration}}/{{iterationCount}}", ""),
        ),
    ]);
    let c = Collection::open(counted.path()).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    assert_eq!(
        url_of(&c, &ctx.for_iteration(None, 2, 3), "j.yml"),
        Ok("https://x.test/2/3".to_owned())
    );
}
