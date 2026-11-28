//! A parallel plan is refused when a request shares state with the others.

use crate::common;

use apinata_core::{
    Collection, ContextOptions, MAX_PARALLEL, ParallelBlocker, ParallelError, RunPlan, ScriptPhase,
    VarContext,
};
use common::write_tree;

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

fn req(name: &str, seq: u32, extra: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"http://127.0.0.1:1/{name}\"\n{extra}"
    )
}

fn script(kind: &str, code: &str) -> String {
    format!("runtime:\n  scripts:\n    - type: {kind}\n      code: |-\n        {code}\n")
}

const CAPTURE: &str = "runtime:\n  actions:\n    - type: set-variable\n      phase: after-response\n      selector:\n        expression: res.body.v\n      variable:\n        name: v\n";

/// The plan and context of a collection holding `files` besides its root.
fn plan(files: &[(&str, &str)]) -> (tempfile::TempDir, RunPlan, VarContext) {
    let mut all = vec![("opencollection.yml", ROOT)];
    all.extend_from_slice(files);
    let dir = write_tree(&all);
    let c = Collection::open(dir.path()).expect("opens");
    let plan = c.plan(dir.path()).expect("plan");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    (dir, plan, ctx)
}

fn blocked(files: &[(&str, &str)]) -> (String, ParallelBlocker) {
    let (_dir, plan, ctx) = plan(files);
    match plan.parallel(4, &ctx, true) {
        Err(ParallelError::Blocked {
            request, reason, ..
        }) => (request.as_str().to_owned(), reason),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn independent_requests_may_run_in_parallel() {
    let (_dir, plan, ctx) = plan(&[
        (
            "a.yml",
            &req("a", 1, &script("tests", "test(\"ok\", () => {});")),
        ),
        (
            "b.yml",
            &req(
                "b",
                2,
                &script(
                    "before-request",
                    "req.setHeader(\"x\", \"1\"); // bru.setVar(\"no\")",
                ),
            ),
        ),
        (
            "c.yml",
            &req(
                "c",
                3,
                &script("after-response", "bru.runner.skipRequest();"),
            ),
        ),
    ]);
    let plan = plan.parallel(4, &ctx, true).expect("parallel");
    assert_eq!(plan.parallelism(), 4);
}

#[test]
fn the_degree_must_lie_between_one_and_the_maximum() {
    let (_dir, plan, ctx) = plan(&[("a.yml", &req("a", 1, ""))]);
    assert_eq!(MAX_PARALLEL, 64);
    for n in [0, MAX_PARALLEL + 1] {
        let err = plan
            .clone()
            .parallel(n, &ctx, true)
            .expect_err("out of range");
        assert!(
            matches!(err, ParallelError::OutOfRange { n: got, .. } if got == n),
            "{err:?}"
        );
        assert_eq!(
            err.to_string(),
            format!("a parallel run takes 1 to 64 requests at a time, not {n}")
        );
    }
    let one = plan.parallel(1, &ctx, true).expect("sequential");
    assert_eq!(one.parallelism(), 1);
}

#[test]
fn a_runtime_write_names_the_request_phase_and_call() {
    let (request, reason) = blocked(&[
        ("a.yml", &req("a", 1, "")),
        (
            "b.yml",
            &req(
                "b",
                2,
                &script("after-response", "bru.setVar(\"t\", res.body.t);"),
            ),
        ),
    ]);
    assert_eq!(request, "b.yml");
    assert_eq!(
        reason,
        ParallelBlocker::ScriptCall {
            phase: ScriptPhase::PostResponse,
            call: "setVar".to_owned()
        }
    );
}

#[test]
fn every_shared_state_call_is_refused() {
    for call in [
        "setVar",
        "deleteVar",
        "deleteAllVars",
        "setEnvVar",
        "deleteEnvVar",
        "deleteAllEnvVars",
        "setGlobalEnvVar",
        "deleteGlobalEnvVar",
        "deleteAllGlobalEnvVars",
        "setCollectionVar",
        "deleteCollectionVar",
        "deleteAllCollectionVars",
        "setNextRequest",
        "stopExecution",
        "runRequest",
        "resetOauth2Credential",
    ] {
        let code = format!("bru.{call}(\"x\");");
        let (_, reason) = blocked(&[("a.yml", &req("a", 1, &script("tests", &code)))]);
        assert_eq!(
            reason,
            ParallelBlocker::ScriptCall {
                phase: ScriptPhase::Tests,
                call: call.to_owned()
            },
            "{call}"
        );
    }
}

#[test]
fn a_call_hidden_in_a_template_or_string_still_refuses() {
    let (_, reason) = blocked(&[(
        "a.yml",
        &req(
            "a",
            1,
            &script("before-request", "const s = `${bru.setNextRequest(null)}`;"),
        ),
    )]);
    assert!(
        matches!(reason, ParallelBlocker::ScriptCall { ref call, .. } if call == "setNextRequest")
    );
    let (_, reason) = blocked(&[(
        "a.yml",
        &req(
            "a",
            1,
            &script("before-request", "bru[\"setVar\"](\"x\", 1);"),
        ),
    )]);
    assert!(matches!(reason, ParallelBlocker::ScriptCall { ref call, .. } if call == "setVar"));
}

#[test]
fn a_folder_script_refuses_the_requests_under_it() {
    let (request, reason) = blocked(&[
        ("a.yml", &req("a", 1, "")),
        (
            "f/folder.yml",
            "info:\n  name: f\n  type: folder\nrequest:\n  scripts:\n    - type: before-request\n      code: |-\n        bru.setEnvVar(\"x\", 1);\n",
        ),
        ("f/inner.yml", &req("inner", 1, "")),
    ]);
    assert_eq!(request, "f/inner.yml");
    assert!(
        matches!(reason, ParallelBlocker::ScriptCall { phase: ScriptPhase::PreRequest, ref call } if call == "setEnvVar")
    );
}

#[test]
fn post_response_variables_refuse_and_the_message_names_the_request() {
    let (_dir, plan, ctx) = plan(&[("login.yml", &req("login", 1, CAPTURE))]);
    let err = plan.parallel(2, &ctx, true).expect_err("refused");
    assert!(matches!(
        &err,
        ParallelError::Blocked {
            reason: ParallelBlocker::PostResponseVars,
            ..
        }
    ));
    assert_eq!(
        err.to_string(),
        "login.yml cannot run in parallel: it sets post-response variables the other requests may read"
    );
}

#[test]
fn a_script_call_message_names_the_request_and_call() {
    let (_dir, plan, ctx) = plan(&[(
        "a.yml",
        &req("a", 1, &script("tests", "bru.setNextRequest(\"b\");")),
    )]);
    let err = plan.parallel(2, &ctx, true).expect_err("refused");
    assert_eq!(
        err.to_string(),
        "a.yml cannot run in parallel: its tests script calls setNextRequest"
    );
}

#[test]
fn scripts_off_skips_the_script_scan_but_not_post_response_variables() {
    let (_dir, plan, ctx) = plan(&[(
        "a.yml",
        &req("a", 1, &script("tests", "bru.setVar(\"x\", 1);")),
    )]);
    assert!(plan.parallel(2, &ctx, false).is_ok());
    let (_dir, plan, ctx) = plan_with_capture();
    assert!(plan.parallel(2, &ctx, false).is_err());
}

fn plan_with_capture() -> (tempfile::TempDir, RunPlan, VarContext) {
    plan(&[("login.yml", &req("login", 1, CAPTURE))])
}

#[test]
fn a_degree_of_one_never_scans() {
    let (_dir, plan, ctx) = plan(&[("login.yml", &req("login", 1, CAPTURE))]);
    assert!(plan.parallel(1, &ctx, true).is_ok());
}
