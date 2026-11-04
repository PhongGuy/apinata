//! The `process.env` namespace: the process environment with the collection's `.env` over it.

use crate::common;

use apinata_core::{Collection, ContextError, ContextOptions, VarContext, resolve};
use common::{request, write_tree};
use std::fs;

/// A collection variable named `process`, which the namespace must replace.
const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nrequest:\n  variables:\n    - name: process\n      value: shadowed\n";

fn http(url: &str) -> String {
    format!("info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\n")
}

fn url_with(files: &[(&str, &str)], options: ContextOptions) -> String {
    let dir = write_tree(files);
    let c = Collection::open(dir.path()).expect("opens");
    let ctx = VarContext::new(&c, options).expect("context");
    resolve(request(&c, "r.yml"), &ctx)
        .expect("resolves")
        .url()
        .to_owned()
}

#[test]
fn process_env_is_namespaced_and_dot_env_overrides_it() {
    let req = http(
        "https://x.test/{{process.env.SHARED}}/{{process.env.PROC}}/{{process.env.DOT}}/{{process.env['a.b']}}",
    );
    let files = [
        ("opencollection.yml", ROOT),
        (".env", "DOT=from_dotenv\nSHARED=dotenv\na.b=dotted\n"),
        ("r.yml", req.as_str()),
    ];
    let options = ContextOptions::new().process_env([("SHARED", "process"), ("PROC", "p")]);
    assert_eq!(
        url_with(&files, options),
        "https://x.test/dotenv/p/from_dotenv/dotted"
    );
}

#[test]
fn environment_rows_can_read_the_process_env() {
    let dev = "name: dev\nvariables:\n  - name: token\n    value: \"{{process.env.TOKEN}}\"\n";
    let req = http("https://x.test/{{token}}");
    let files = [
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", dev),
        ("r.yml", req.as_str()),
    ];
    let options = ContextOptions::new()
        .env("dev")
        .process_env([("TOKEN", "tok")]);
    assert_eq!(url_with(&files, options), "https://x.test/tok");
}

#[test]
fn the_real_process_environment_is_the_default() {
    let (name, value) = std::env::vars_os()
        .find_map(|(k, v)| {
            let (k, v) = (k.into_string().ok()?, v.into_string().ok()?);
            let plain_name =
                !k.is_empty() && k.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_');
            (plain_name && !v.contains('{')).then_some((k, v))
        })
        .expect("the test process has a plainly named variable");
    // A header carries the value verbatim; the URL would be normalized.
    let req = format!(
        "{}  headers:\n    - name: X-V\n      value: \"{{{{process.env.{name}}}}}\"\n",
        http("https://x.test/")
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", req.as_str())]);
    let c = Collection::open(dir.path()).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let r = resolve(request(&c, "r.yml"), &ctx).expect("resolves");
    assert_eq!(r.headers(), [("X-V".to_owned(), value)]);
}

#[test]
fn a_non_utf8_dot_env_is_a_context_error() {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    fs::write(dir.path().join(".env"), [0xff, 0xfe, b'\n']).expect("write");
    let c = Collection::open(dir.path()).expect("opens");
    assert!(matches!(
        VarContext::new(&c, ContextOptions::new()),
        Err(ContextError::DotEnv { .. })
    ));
}
