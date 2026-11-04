//! Helpers shared by the script integration tests.
#![allow(dead_code)]

use apinata_core::{
    CancelHandle, Collection, ContextOptions, ExecOptions, Executor, ItemPath, RequestResult,
    RunError, RunOptions, RunReport, ScriptOptions, VarContext, Verdict, run, send_one,
};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

pub const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

pub fn write_tree(files: &[(&str, &str)]) -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    for (rel, text) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(path, text).expect("write");
    }
    dir
}

/// One `scripts` list item (`kind` is before-request, after-response or tests).
pub fn script(kind: &str, code: &str) -> String {
    let body: String = code.lines().map(|l| format!("        {l}\n")).collect();
    format!("    - type: {kind}\n      code: |-\n{body}")
}

/// A GET request to `url`; `scripts` holds `script` items, `extra` more top-level YAML.
pub fn scripted(name: &str, url: &str, scripts: &str, extra: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\nruntime:\n  scripts:\n{scripts}{extra}"
    )
}

pub fn folder(scripts: &str) -> String {
    format!("info:\n  name: f\n  type: folder\nrequest:\n  scripts:\n{scripts}")
}

/// `opencollection.yml` with collection scripts and an optional flow.
pub fn root(scripts: &str, flow: Option<&str>) -> String {
    let mut text = format!("{ROOT}request:\n  scripts:\n{scripts}");
    if let Some(flow) = flow {
        text.push_str(&format!(
            "extensions:\n  bruno:\n    scripts:\n      flow: {flow}\n"
        ));
    }
    text
}

pub fn on() -> RunOptions {
    RunOptions::new().scripts(ScriptOptions::new().enabled(true))
}

pub async fn send_full(
    dir: &Path,
    rel: &str,
    opts: RunOptions,
    ctx: ContextOptions,
    exec: ExecOptions,
) -> (RequestResult, VarContext) {
    let c = Collection::open(dir).expect("opens");
    assert!(c.load_errors().is_empty(), "{:?}", c.load_errors());
    let ctx = VarContext::new(&c, ctx).expect("context");
    let exec = Arc::new(Executor::new(exec).expect("client"));
    let request = apinata_core::RequestFile::load(dir.join(rel)).expect("loads");
    let path = ItemPath::new(rel).expect("item path");
    let result = send_one(path, &request, &ctx, &exec, &opts, &CancelHandle::new()).await;
    (result, ctx)
}

pub async fn send(dir: &Path, rel: &str, opts: RunOptions) -> RequestResult {
    send_full(dir, rel, opts, ContextOptions::new(), ExecOptions::new())
        .await
        .0
}

pub async fn run_dir(dir: &Path, opts: RunOptions) -> RunReport {
    let c = Collection::open(dir).expect("opens");
    let plan = c.plan(dir).expect("plan");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let exec = Arc::new(Executor::new(ExecOptions::new()).expect("client"));
    run(plan, ctx, exec, opts, CancelHandle::new(), Box::new(|_| {})).await
}

/// `level:text` per console entry.
pub fn console(result: &RequestResult) -> Vec<String> {
    result
        .console
        .iter()
        .map(|c| format!("{}:{}", c.level.as_str(), c.text))
        .collect()
}

/// `phase name: pass`, `phase name: fail <detail>` or `phase name: skipped <reason>`.
pub fn tests(result: &RequestResult) -> Vec<String> {
    result
        .tests
        .iter()
        .map(|t| {
            let verdict = match &t.verdict {
                Verdict::Pass => "pass".to_owned(),
                Verdict::Fail { detail, .. } => {
                    format!("fail {}", detail.clone().unwrap_or_default())
                }
                Verdict::Skipped { reason, .. } => format!("skipped {reason}"),
                _ => "other".to_owned(),
            };
            format!("{} {}: {verdict}", t.phase.as_str(), t.name)
        })
        .collect()
}

/// Scripts on with a phase time limit of `ms`.
pub fn limited(ms: u64) -> RunOptions {
    RunOptions::new().scripts(
        ScriptOptions::new()
            .enabled(true)
            .timeout(Duration::from_millis(ms)),
    )
}

/// The message of a pre-request script error.
pub fn script_error(result: &RequestResult) -> String {
    match &result.error {
        Some(RunError::Script { message, .. }) => message.clone(),
        other => panic!("expected a script error, got {other:?}"),
    }
}
