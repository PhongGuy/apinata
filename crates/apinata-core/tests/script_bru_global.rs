//! The bru global environment functions over the script overlay.

use crate::script_support;

use apinata_core::{
    ContextOptions, EnvOverlay, ExecOptions, GlobalEnvironments, Outcome, OverlayLayer,
};
use script_support::*;
use std::sync::Arc;
use tempfile::TempDir;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const VARS_ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nrequest:\n  variables:\n    - name: greet\n      value: hello\n";

fn globals(body: &str) -> (TempDir, Arc<GlobalEnvironments>) {
    let dir = write_tree(&[("g.yml", body)]);
    let g = GlobalEnvironments::open(dir.path()).expect("globals");
    (dir, Arc::new(g))
}

async fn ok_server() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    s
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_functions_follow_the_reference_semantics() {
    let s = ok_server().await;
    let code = r#"console.log(bru.getGlobalEnvVar("g"), bru.hasGlobalEnvVar("g"), bru.hasGlobalEnvVar("zz"));
bru.setGlobalEnvVar("g", "script"); console.log(bru.getGlobalEnvVar("g"));
bru.setGlobalEnvVar("n", {a: 1}); console.log(JSON.stringify(bru.getAllGlobalEnvVars()));
bru.deleteGlobalEnvVar("n"); console.log(bru.hasGlobalEnvVar("n"));
bru.setGlobalEnvVar("t", "{{greet}}!"); console.log(bru.getGlobalEnvVar("t"));
try { bru.setGlobalEnvVar("bad name", 1) } catch (e) { console.log(e.message) }
try { bru.setGlobalEnvVar("", 1) } catch (e) { console.log(e.message) }
console.log(bru.getEnvName());"#;
    let (gdir, g) = globals("name: g\nvariables:\n  - name: g\n    value: file\n");
    let before = std::fs::read(gdir.path().join("g.yml")).expect("global");
    let dir = write_tree(&[
        ("opencollection.yml", VARS_ROOT),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/ok?g={{{{g}}}}", s.uri()),
                &script("before-request", code),
                "",
            ),
        ),
    ]);
    let options = ContextOptions::new().global_env(g, "g");
    let (result, ctx) = send_full(dir.path(), "r.yml", on(), options, ExecOptions::new()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    let invalid = r#"Variable name: "bad name" contains invalid characters! Names must only contain alpha-numeric characters, "-", "_", ".""#;
    let expected: Vec<String> = vec![
        "log:file true false".into(),
        "log:script".into(),
        r#"log:{"g":"script","n":{"a":1}}"#.into(),
        "log:false".into(),
        "log:hello!".into(),
        format!("log:{invalid}"),
        "log:Creating a env variable without specifying a name is not allowed.".into(),
        // A global environment is not the environment `getEnvName` names.
        "log:undefined".into(),
    ];
    assert_eq!(console(&result), expected);
    let received = s.received_requests().await.expect("recorded");
    assert_eq!(received[0].url.query(), Some("g=script"));
    assert_eq!(
        std::fs::read(gdir.path().join("g.yml")).expect("global"),
        before
    );
    assert!(
        ctx.env_overlay()
            .entries()
            .iter()
            .any(|v| v.layer() == OverlayLayer::Global && v.name() == "g")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_global_write_without_a_selected_global_environment_lives_in_memory() {
    let s = ok_server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/ok?w={{{{w}}}}", s.uri()),
                &script("before-request", "bru.setGlobalEnvVar('w', 'v')"),
                "",
            ),
        ),
    ]);
    let (result, _) = send_full(
        dir.path(),
        "r.yml",
        on(),
        ContextOptions::new(),
        ExecOptions::new(),
    )
    .await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    let received = s.received_requests().await.expect("recorded");
    assert_eq!(received[0].url.query(), Some("w=v"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_all_removes_every_global_from_the_script_view() {
    let s = ok_server().await;
    let code = "bru.deleteAllGlobalEnvVars(); console.log(bru.hasGlobalEnvVar('g'), Object.keys(bru.getAllGlobalEnvVars()).length)";
    let (_gdir, g) =
        globals("name: g\nvariables:\n  - name: g\n    value: a\n  - name: h\n    value: b\n");
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/ok", s.uri()),
                &script("before-request", code),
                "",
            ),
        ),
    ]);
    let (result, _) = send_full(
        dir.path(),
        "r.yml",
        on(),
        ContextOptions::new().global_env(g, "g"),
        ExecOptions::new(),
    )
    .await;
    assert_eq!(console(&result), ["log:false 0".to_owned()]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sensitive_global_read_joins_the_sensitive_values() {
    let s = ok_server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/ok", s.uri()),
                &script(
                    "before-request",
                    "console.log(bru.getGlobalEnvVar('gt').length)",
                ),
                "",
            ),
        ),
    ]);
    let options = ContextOptions::new().sensitive_global_env_var("gt", "s3cret");
    let (result, _) = send_full(dir.path(), "r.yml", on(), options, ExecOptions::new()).await;
    assert_eq!(console(&result), ["log:6".to_owned()]);
    assert!(
        result
            .script_sensitive_values()
            .iter()
            .any(|v| v == "s3cret")
    );
}

#[test]
fn the_global_overlay_layer_is_one_map_cleared_on_its_own() {
    let o = EnvOverlay::new();
    o.set_text(OverlayLayer::Global, "a", "1");
    assert!(!o.is_empty());
    o.set_text(OverlayLayer::Collection, "c", "2");
    o.select_environment(Some("dev"));
    assert!(
        o.entries()
            .iter()
            .any(|v| v.layer() == OverlayLayer::Global && v.name() == "a")
    );
    o.clear_layer(OverlayLayer::Global);
    assert!(
        o.entries()
            .iter()
            .all(|v| v.layer() != OverlayLayer::Global)
    );
    assert_eq!(o.collection_names(), ["c"]);
    o.set_text(OverlayLayer::Global, "a", "1");
    o.clear();
    assert!(o.is_empty());
}
