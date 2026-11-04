//! The bru variable functions, the overlay and sensitive reads.

use crate::script_support;

use apinata_core::{ContextOptions, ExecOptions, Outcome};
use script_support::*;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const VARS_ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nrequest:\n  variables:\n    - name: greet\n      value: hello\n";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_var_reaches_the_url_of_the_same_request() {
    let s = MockServer::start().await;
    Mock::given(path("/t/abc3"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&s)
        .await;
    let url = format!("{}/t/{{{{t}}}}", s.uri());
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted(
                "R",
                &url,
                &script("before-request", "bru.setVar('t', 'abc' + (1 + 2))"),
                "",
            ),
        ),
    ]);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    assert_eq!(result.response.as_ref().map(|r| r.status()), Some(200));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn variable_functions_follow_the_reference_semantics() {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let code = r#"bru.setVar("a", {n: 1}); console.log(JSON.stringify(bru.getVar("a")), bru.hasVar("a"), bru.hasVar("zz"));
bru.setVar("u", undefined); console.log(bru.hasVar("u"), JSON.stringify(bru.getAllVars()));
bru.deleteVar("u"); console.log(bru.hasVar("u"));
try { bru.setVar("bad name", 1) } catch (e) { console.log(e.message) }
try { bru.setVar("", 1) } catch (e) { console.log(e.message) }
bru.setVar("g", "{{greet}} there {{nope}}"); console.log(bru.getVar("g"));
console.log(bru.getEnvVar("e"), bru.hasEnvVar("e")); bru.setEnvVar("e", "script"); console.log(bru.getEnvVar("e"));
bru.deleteEnvVar("e"); console.log(bru.hasEnvVar("e"));
bru.setCollectionVar("c", 2); console.log(bru.getCollectionVar("c"), bru.getCollectionVar("greet"));
console.log(bru.getFolderVar("fv"), bru.getRequestVar("rv"));
console.log(bru.isSafeMode(), bru.getCollectionName(), bru.getEnvName(), bru.interpolate("{{greet}}!"), JSON.stringify(bru.interpolate({x: "{{greet}}"})));"#;
    let request = scripted(
        "R",
        &format!("{}/ok", s.uri()),
        &script("before-request", code),
        "  variables:\n    - name: rv\n      value: req\n",
    );
    let folder_yml = "info:\n  name: f\n  type: folder\nrequest:\n  variables:\n    - name: fv\n      value: fold\n";
    let dir = write_tree(&[
        ("opencollection.yml", VARS_ROOT),
        ("f/folder.yml", folder_yml),
        ("f/r.yml", &request),
    ]);
    let options = ContextOptions::new().env_var("e", "file");
    let (result, ctx) = send_full(dir.path(), "f/r.yml", on(), options, ExecOptions::new()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    let invalid = r#"Variable name: "bad name" contains invalid characters! Names must only contain alpha-numeric characters, "-", "_", ".""#;
    assert_eq!(
        console(&result),
        [
            r#"log:{"n":1} true false"#.to_owned(),
            r#"log:true {"a":{"n":1}}"#.to_owned(),
            "log:false".to_owned(),
            format!("log:{invalid}"),
            "log:Creating a variable without specifying a name is not allowed.".to_owned(),
            "log:hello there {{nope}}".to_owned(),
            "log:file true".to_owned(),
            "log:script".to_owned(),
            "log:false".to_owned(),
            "log:2 hello".to_owned(),
            "log:fold req".to_owned(),
            r#"log:true t undefined hello! {"x":"hello"}"#.to_owned(),
        ]
    );
    assert!(ctx.runtime_vars().entries().iter().any(|v| v.name() == "a"));
    assert_eq!(ctx.env_overlay().collection_names(), ["c"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn overlay_writes_never_reach_disk() {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let env = "name: dev\nvariables:\n  - name: e\n    value: file\n";
    let code = "bru.setEnvVar('e', 'script'); bru.setCollectionVar('greet', 'x')";
    let dir = write_tree(&[
        ("opencollection.yml", VARS_ROOT),
        ("environments/dev.yml", env),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/ok?e={{{{e}}}}", s.uri()),
                &script("before-request", code),
                "",
            ),
        ),
    ]);
    let before = (
        std::fs::read(dir.path().join("environments/dev.yml")).expect("env"),
        std::fs::read(dir.path().join("opencollection.yml")).expect("root"),
    );
    let (result, _) = send_full(
        dir.path(),
        "r.yml",
        on(),
        ContextOptions::new().env("dev"),
        ExecOptions::new(),
    )
    .await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    let received = s.received_requests().await.expect("recorded");
    assert_eq!(received[0].url.query(), Some("e=script"));
    let after = (
        std::fs::read(dir.path().join("environments/dev.yml")).expect("env"),
        std::fs::read(dir.path().join("opencollection.yml")).expect("root"),
    );
    assert_eq!(before, after);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sensitive_reads_join_the_sensitive_values() {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let code = "console.log(bru.getEnvVar('tok').length, bru.getProcessEnv('PX'), bru.getEnvVar('plain'), bru.interpolate('{{tok2}}').length, Object.keys(bru.getAllEnvVars()).length > 0)";
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
    let options = ContextOptions::new()
        .sensitive_env_var("tok", "s3cret")
        .sensitive_env_var("tok2", "an0ther")
        .env_var("plain", "open")
        .process_env([("PX", "pv")]);
    let (result, _) = send_full(dir.path(), "r.yml", on(), options, ExecOptions::new()).await;
    assert_eq!(console(&result), ["log:6 pv open 7 true"]);
    let mut values = result.script_sensitive_values().to_vec();
    values.sort();
    values.dedup();
    assert_eq!(values, ["an0ther", "pv", "s3cret"]);
    assert!(!format!("{result:?}").contains("s3cret"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unparsable_bridge_arguments_surface_the_parse_error() {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    // A lone surrogate is not valid JSON text once it crosses the bridge.
    let code = r#"try { bru.setVar("s", "\ud800") } catch (e) { console.log(e.message) }"#;
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
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    let lines = console(&result);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(!lines[0].contains("unknown bru operation"), "{lines:?}");
    assert!(lines[0].contains("invalid arguments"), "{lines:?}");
}

// `dev` holds an enabled secret `tok` and a disabled secret `off`, and the store holds neither.
// Scripts read the enabled one as the empty string, like the reference; the URL still refuses it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unstored_secret_reads_as_empty_in_scripts_but_stays_undefined_in_the_url() {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let code = "console.log(JSON.stringify(bru.getEnvVar('tok')), typeof bru.getEnvVar('off'))";
    let env = "name: dev\nvariables:\n  - secret: true\n    name: tok\n  - secret: true\n    name: off\n    disabled: true\n";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", env),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/ok", s.uri()),
                &script("before-request", code),
                "",
            ),
        ),
        (
            "u.yml",
            &scripted("U", &format!("{}/ok/{{{{tok}}}}", s.uri()), "", ""),
        ),
    ]);
    let options = || {
        ContextOptions::new()
            .env("dev")
            .secrets(std::sync::Arc::new(apinata_core::MemoryStore::new()))
    };
    let (result, _) = send_full(dir.path(), "r.yml", on(), options(), ExecOptions::new()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    assert_eq!(console(&result), [r#"log:"" undefined"#]);
    let (result, _) = send_full(dir.path(), "u.yml", on(), options(), ExecOptions::new()).await;
    assert!(
        matches!(
            &result.error,
            Some(apinata_core::RunError::Resolve(apinata_core::ResolveError::UndefinedVariable { name, .. })) if name == "tok"
        ),
        "{:?}",
        result.error
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_scripts_own_set_wins_over_an_unstored_secret() {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let code = "console.log(bru.hasEnvVar('tok')); bru.setEnvVar('tok', 'mine'); console.log(bru.getEnvVar('tok'))";
    let env = "name: dev\nvariables:\n  - secret: true\n    name: tok\n";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", env),
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
    let options = ContextOptions::new()
        .env("dev")
        .secrets(std::sync::Arc::new(apinata_core::MemoryStore::new()));
    let (result, _) = send_full(dir.path(), "r.yml", on(), options, ExecOptions::new()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    assert_eq!(console(&result), ["log:true", "log:mine"]);
}
