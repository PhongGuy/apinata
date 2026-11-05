//! A Postman collection with scripts, imported and written: the translated code lands in the
//! right YAML sections, unsupported calls stay and are reported, and the written tree carries no
//! approval marker or script hash (`write_collection` has no trust parameter, so approval cannot
//! happen at this layer).

use crate::script_support::{console, on, tests};
use apinata_core::import::{NoteKind, parse_postman, write_collection};
use apinata_core::{
    CancelHandle, Collection, ContextOptions, EnvOverlay, ExecOptions, Executor, ItemPath,
    MemoryStore, Outcome, OverlayLayer, RequestFile, RequestResult, VarContext, send_one,
};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn read(root: &Path, rel: &str) -> String {
    fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

#[test]
fn imported_scripts_are_translated_placed_and_carry_no_approval_marker() {
    let bytes = fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/postman/own/scripts-subset.json"),
    )
    .expect("fixture");
    let imported = parse_postman(&bytes).expect("parses");
    let parent = TempDir::new().expect("tempdir");
    let (collection, report) =
        write_collection(parent.path(), &imported, &[], &MemoryStore::new()).expect("writes");
    let root = collection.root().to_path_buf();

    let root_yml = read(&root, "opencollection.yml");
    assert!(root_yml.contains("scripts:"), "root scripts: {root_yml}");
    assert!(root_yml.contains("bru.setEnvVar"), "{root_yml}");
    assert!(root_yml.contains("test("), "{root_yml}");
    assert!(!root_yml.contains("pm."), "{root_yml}");

    let folder = read(&root, "Group/folder.yml");
    assert!(folder.contains("bru.setEnvVar(\"folder_pre\""), "{folder}");

    let checked = read(&root, "Group/Checked.yml");
    assert!(checked.contains("runtime:"), "{checked}");
    assert!(checked.contains("bru.setEnvVar(\"token\""), "{checked}");
    assert!(
        checked.contains("expect(res.getStatus()).to.equal(200)"),
        "{checked}"
    );
    assert!(checked.contains("res.getBody()"), "{checked}");
    assert!(checked.contains("test(\"has id\""), "{checked}");
    assert!(!checked.contains("pm."), "{checked}");

    let unsupported = read(&root, "Group/Unsupported.yml");
    assert!(unsupported.contains("pm.visualizer.set("), "{unsupported}");
    assert!(
        report.notes.iter().any(|n| n.kind == NoteKind::Converted
            && n.detail.contains("pm.visualizer.set")
            && n.file.as_deref() == Some("Group/Unsupported.yml")),
        "{:?}",
        report.notes
    );

    // write_collection takes no trust path, so the written tree must carry no approval marker
    // and no script hash. A hash is 64 lowercase hex digits; no such run may appear anywhere.
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("read dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let text = fs::read_to_string(&path).unwrap_or_default();
            assert!(!text.contains("script_trust"), "{}", path.display());
            assert!(!text.contains("approved"), "{}", path.display());
            assert!(!has_hex64_run(&text), "{}", path.display());
        }
    }
}

/// True when `text` holds a run of at least 64 consecutive lowercase hex digits.
fn has_hex64_run(text: &str) -> bool {
    let mut run = 0;
    for c in text.chars() {
        run = if matches!(c, '0'..='9' | 'a'..='f') {
            run + 1
        } else {
            0
        };
        if run >= 64 {
            return true;
        }
    }
    false
}

/// A server at 127.0.0.1 that answers `GET /items/abc` with `x-n: 2` and nothing else.
async fn item_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/items/abc"))
        .and(header("x-n", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 7, "name": "seven"})))
        .mount(&server)
        .await;
    server
}

/// A Postman collection whose one request takes its path and a header from what its
/// pre-request script sets, and whose test script asserts on the response and stores a value
/// from it; imported and written under `parent`, with an environment `dev` beside it.
fn import_scripted(parent: &Path, server: &MockServer) -> PathBuf {
    let collection = json!({
        "info": {
            "name": "Scripted",
            "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
        },
        "item": [{
            "name": "Get",
            "event": [
                {"listen": "prerequest", "script": {"exec": [
                    "pm.environment.set('token', 'abc');",
                    "pm.variables.set('n', 2);"
                ]}},
                {"listen": "test", "script": {"exec": [
                    "pm.test('status is 200', function () {",
                    "  pm.response.to.have.status(200);",
                    "});",
                    "pm.test('body has the id', () => {",
                    "  const body = pm.response.json();",
                    "  pm.expect(body.id).to.eql(7);",
                    "});",
                    "pm.test('token is visible', () => {",
                    "  pm.expect(pm.environment.get('token')).to.eql('abc');",
                    "});",
                    "pm.environment.set('seen', pm.response.json().name);",
                    "console.log(pm.environment.get('seen'));"
                ]}}
            ],
            "request": {
                "method": "GET",
                "url": format!("{}/items/{{{{token}}}}", server.uri()),
                "header": [{"key": "x-n", "value": "{{n}}"}]
            }
        }]
    });
    let imported = parse_postman(collection.to_string().as_bytes()).expect("parses");
    let (written, _) =
        write_collection(parent, &imported, &[], &MemoryStore::new()).expect("writes");
    let root = written.root().to_path_buf();
    let env = "name: dev\nvariables:\n  - name: token\n    value: file\n";
    fs::create_dir_all(root.join("environments")).expect("mkdir");
    fs::write(root.join("environments/dev.yml"), env).expect("env");
    root
}

/// Sends `Get.yml` of `root` through `ctx` with scripts on.
async fn send_get(root: &Path, ctx: &VarContext) -> RequestResult {
    let exec = Arc::new(Executor::new(ExecOptions::new()).expect("client"));
    let request = RequestFile::load(root.join("Get.yml")).expect("loads");
    let item = ItemPath::new("Get.yml").expect("item path");
    send_one(item, &request, ctx, &exec, &on(), &CancelHandle::new()).await
}

fn context(root: &Path, options: ContextOptions) -> VarContext {
    let collection = Collection::open(root).expect("opens");
    assert!(
        collection.load_errors().is_empty(),
        "{:?}",
        collection.load_errors()
    );
    VarContext::new(&collection, options).expect("context")
}

/// The script-set environment values `ctx` sees, as `name=value`.
fn env_values(ctx: &VarContext) -> Vec<String> {
    (ctx.env_overlay().entries().iter())
        .filter(|v| v.layer() == OverlayLayer::Environment)
        .map(|v| format!("{}={}", v.name(), v.value().unwrap_or("")))
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_scripts_run_against_a_local_server_and_update_the_active_environment() {
    let server = item_server().await;
    let parent = TempDir::new().expect("tempdir");
    let root = import_scripted(parent.path(), &server);
    let env_file = fs::read(root.join("environments/dev.yml")).expect("env");

    let ctx = context(&root, ContextOptions::new().env("dev"));
    let result = send_get(&root, &ctx).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    assert_eq!(
        tests(&result),
        [
            "post-response status is 200: pass",
            "post-response body has the id: pass",
            "post-response token is visible: pass",
        ]
    );
    assert_eq!(console(&result), ["log:seven"]);
    // The active environment carries what the scripts set, over the file's own value.
    assert_eq!(env_values(&ctx), ["seen=seven", "token=abc"]);
    assert_eq!(ctx.environment(), Some("dev"));
    // Script values live in the session, never in the environment file.
    assert_eq!(
        fs::read(root.join("environments/dev.yml")).expect("env"),
        env_file
    );
}

/// With no environment selected, an environment value a script sets reaches the rest of the
/// request and later requests of the same session, as in the reference's runner, but no
/// environment: selecting one afterwards does not show it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn with_no_environment_selected_script_values_last_for_the_session_only() {
    let server = item_server().await;
    let parent = TempDir::new().expect("tempdir");
    let root = import_scripted(parent.path(), &server);
    let overlay = EnvOverlay::new();

    let ctx = context(&root, ContextOptions::new().env_overlay(overlay.clone()));
    assert_eq!(ctx.environment(), None);
    let result = send_get(&root, &ctx).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    assert!(
        tests(&result).iter().all(|t| t.ends_with(": pass")),
        "{:?}",
        tests(&result)
    );
    assert_eq!(env_values(&ctx), ["seen=seven", "token=abc"]);

    // A later request of the same session still sees them.
    let again = send_get(&root, &ctx).await;
    assert_eq!(again.outcome, Outcome::Passed, "{:?}", again.error);
    assert_eq!(env_values(&ctx), ["seen=seven", "token=abc"]);

    let dev = context(&root, ContextOptions::new().env("dev").env_overlay(overlay));
    assert!(env_values(&dev).is_empty(), "{:?}", env_values(&dev));
    assert_eq!(server.received_requests().await.expect("recorded").len(), 2);
}
