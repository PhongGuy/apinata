//! `bru.runRequest` paths, containment and the shape of a failure.

use crate::script_support;

use apinata_core::{ContextOptions, ExecOptions, Outcome};
use script_support::*;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn plain(name: &str, url: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n"
    )
}

fn invalid(p: &str) -> String {
    format!("bru.runRequest: invalid request path - {p}")
}

async fn mounted(s: &MockServer, route: &str, times: u64) {
    Mock::given(path(route))
        .respond_with(ResponseTemplate::new(200))
        .expect(times)
        .mount(s)
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bad_paths_resolve_to_a_message() {
    let s = MockServer::start().await;
    mounted(&s, "/main", 1).await;
    mounted(&s, "/outside", 0).await;
    let code = "const out = [];\nfor (const p of ['missing', 'f/folder', 'opencollection', '../outside', '/etc/hosts', '']) {\n  out.push((await bru.runRequest(p)).message);\n}\nconsole.log(JSON.stringify(out));";
    let dir = write_tree(&[
        ("col/opencollection.yml", ROOT),
        ("col/f/folder.yml", "info:\n  name: f\n  type: folder\n"),
        (
            "col/r.yml",
            &scripted(
                "R",
                &format!("{}/main", s.uri()),
                &script("before-request", code),
                "",
            ),
        ),
        ("outside.yml", &plain("O", &format!("{}/outside", s.uri()))),
    ]);
    let result = send(&dir.path().join("col"), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    let expected = serde_json::json!([
        invalid("missing"),
        invalid("f/folder"),
        invalid("opencollection"),
        invalid("../outside"),
        invalid("/etc/hosts"),
        invalid(""),
    ]);
    assert_eq!(console(&result), [format!("log:{expected}")]);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_symlink_out_of_the_root_is_refused() {
    let s = MockServer::start().await;
    mounted(&s, "/main", 1).await;
    mounted(&s, "/outside", 0).await;
    let code = "console.log((await bru.runRequest('link')).message);";
    let dir = write_tree(&[
        ("col/opencollection.yml", ROOT),
        (
            "col/r.yml",
            &scripted(
                "R",
                &format!("{}/main", s.uri()),
                &script("before-request", code),
                "",
            ),
        ),
        ("outside.yml", &plain("O", &format!("{}/outside", s.uri()))),
    ]);
    let col = dir.path().join("col");
    std::os::unix::fs::symlink(dir.path().join("outside.yml"), col.join("link.yml")).unwrap();
    let result = send(&col, "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    assert_eq!(console(&result), [format!("log:{}", invalid("link"))]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failures_resolve_rather_than_reject() {
    let s = MockServer::start().await;
    mounted(&s, "/main", 1).await;
    mounted(&s, "/boom", 0).await;
    mounted(&s, "/skip", 0).await;
    let code = "const boom = await bru.runRequest('boom');\nconst undef = await bru.runRequest('undef');\nconst down = await bru.runRequest('down');\nconst skip = await bru.runRequest('skip');\nconsole.log(boom.message.startsWith('pre-request script error') && boom.message.includes('boom'));\nconsole.log(undef.message);\nconsole.log(down.status === undefined, typeof down.error, 'message' in down);\nconsole.log(skip.message);";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/main", s.uri()),
                &script("before-request", code),
                "",
            ),
        ),
        (
            "boom.yml",
            &scripted(
                "B",
                &format!("{}/boom", s.uri()),
                &script("before-request", "throw new Error('boom');"),
                "",
            ),
        ),
        ("undef.yml", &plain("U", "{{nope}}/x")),
        // Port 1 on loopback refuses the connection: a transport error with no response.
        ("down.yml", &plain("D", "http://127.0.0.1:1/x")),
        (
            "skip.yml",
            &scripted(
                "S",
                &format!("{}/skip", s.uri()),
                &script("before-request", "bru.runner.skipRequest();"),
                "",
            ),
        ),
    ]);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    assert_eq!(
        console(&result),
        [
            "log:true",
            "log:variable `nope` is not defined",
            "log:true string false",
            "log:bru.runRequest: request skipped - skip",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn what_a_nested_run_reads_stays_sensitive() {
    let s = MockServer::start().await;
    mounted(&s, "/main", 1).await;
    mounted(&s, "/sens", 1).await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/main", s.uri()),
                &script("before-request", "await bru.runRequest('sens');"),
                "",
            ),
        ),
        (
            "sens.yml",
            &scripted(
                "S",
                &format!("{}/sens", s.uri()),
                &script(
                    "before-request",
                    "bru.setVar('x', bru.getProcessEnv('TOKEN'));",
                ),
                "",
            ),
        ),
    ]);
    let ctx = ContextOptions::new().process_env([("TOKEN", "sek-91")]);
    let (result, _) = send_full(dir.path(), "r.yml", on(), ctx, ExecOptions::new()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    assert!(
        result
            .script_sensitive_values()
            .contains(&"sek-91".to_owned()),
        "{:?}",
        result.script_sensitive_values()
    );
}
