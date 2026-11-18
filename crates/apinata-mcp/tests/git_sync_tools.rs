//! `git_branches`, `git_switch_branch`, `git_pull` and `git_push`. Every remote is a local bare
//! repository or an unreachable `example.invalid` URL, and ssh is replaced by `false`, so no
//! test reaches the network.

use crate::common;
use crate::git_support;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{ROOT, TestServer};
use git_support::{Sandbox, write};
use serde_json::json;

async fn serve(sb: &Sandbox) -> TestServer {
    let server = TestServer::start_with(&[("opencollection.yml", ROOT)], |options| {
        let git = sb
            .options()
            .env(
                "GIT_CEILING_DIRECTORIES",
                options.root().parent().expect("parent"),
            )
            .env("GIT_SSH_COMMAND", "false");
        options.git(git)
    })
    .await;
    sb.git(server.root(), &["init", "-q", "-b", "main"]);
    sb.commit_all(server.root(), "base");
    server
}

/// A served repository whose `main` tracks `origin/main` in a new local bare repository.
async fn tracking(sb: &Sandbox) -> (TestServer, PathBuf) {
    let server = serve(sb).await;
    let bare = sb.path().join("remote.git");
    sb.git(
        sb.path(),
        &["init", "-q", "--bare", "-b", "main", "remote.git"],
    );
    let url = bare.to_str().expect("utf-8");
    sb.git(server.root(), &["remote", "add", "origin", url]);
    sb.git(server.root(), &["push", "-q", "-u", "origin", "main"]);
    (server, bare)
}

/// A served repository whose `main` tracks `origin/main` at `url`, with the tracking ref in
/// place so nothing but the remote's reach stops a pull or push.
async fn tracking_url(sb: &Sandbox, url: &str) -> TestServer {
    let server = serve(sb).await;
    let root = server.root();
    sb.git(root, &["remote", "add", "origin", url]);
    sb.git(root, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    sb.git(root, &["branch", "-q", "--set-upstream-to", "origin/main"]);
    server
}

fn rev(sb: &Sandbox, repo: &Path, name: &str) -> String {
    sb.git(repo, &["rev-parse", name]).trim().to_owned()
}

#[tokio::test]
async fn git_branches_lists_local_and_remote_tracking_branches() {
    let sb = Sandbox::new();
    let (server, _) = tracking(&sb).await;
    let root = server.root();
    sb.git(root, &["branch", "-q", "local-only"]);
    sb.git(root, &["push", "-q", "origin", "main:remote-only"]);
    sb.git(root, &["fetch", "-q", "origin"]);
    let got = server.ok("git_branches", json!({})).await;
    assert_eq!(
        got["branches"],
        json!([
            {"local": "local-only", "upstream": null, "head": false},
            {"local": "main", "upstream": "origin/main", "head": true},
            {"local": null, "upstream": "origin/remote-only", "head": false},
        ])
    );
}

#[tokio::test]
async fn git_branches_is_read_only_and_never_fetches() {
    let sb = Sandbox::new();
    let server = serve(&sb).await;
    let tools = server.tools().await;
    let tool = tools
        .iter()
        .find(|t| t.name == "git_branches")
        .expect("listed");
    let annotations = tool.annotations.as_ref().expect("annotations");
    assert_eq!(annotations.read_only_hint, Some(true));
    assert!(tool.output_schema.is_some());
    let schema = serde_json::to_value(&tool.input_schema).expect("schema");
    assert!(schema["properties"].get("fetch").is_none(), "{schema}");
}

#[tokio::test]
async fn git_switch_branch_switches_to_a_local_or_remote_branch() {
    let sb = Sandbox::new();
    let (server, _) = tracking(&sb).await;
    let root = server.root();
    sb.git(root, &["switch", "-q", "-c", "work"]);
    write(root, "w.txt", "w\n");
    sb.commit_all(root, "work");
    sb.git(root, &["push", "-q", "origin", "work:shared"]);
    sb.git(root, &["switch", "-q", "main"]);

    let got = server
        .ok("git_switch_branch", json!({"branch": "work"}))
        .await;
    assert_eq!(got, json!({"branch": "work", "upstream": "unchanged"}));
    assert_eq!(sb.git(root, &["branch", "--show-current"]).trim(), "work");

    let got = server
        .ok("git_switch_branch", json!({"branch": "origin/shared"}))
        .await;
    assert_eq!(got, json!({"branch": "shared", "upstream": "created"}));
    assert_eq!(sb.git(root, &["branch", "--show-current"]).trim(), "shared");
}

#[tokio::test]
async fn git_switch_branch_refuses_uncommitted_changes_with_cores_message() {
    let sb = Sandbox::new();
    let (server, _) = tracking(&sb).await;
    let root = server.root();
    sb.git(root, &["branch", "-q", "other"]);
    write(root, "scratch.txt", "not committed\n");
    let message = server
        .err("git_switch_branch", json!({"branch": "other"}))
        .await;
    assert_eq!(
        message,
        "commit or discard your changes before switching branches"
    );
    assert_eq!(sb.git(root, &["branch", "--show-current"]).trim(), "main");
}

#[tokio::test]
async fn git_switch_branch_refuses_an_unknown_branch() {
    let sb = Sandbox::new();
    let (server, _) = tracking(&sb).await;
    let message = server
        .err("git_switch_branch", json!({"branch": "nope"}))
        .await;
    assert!(message.contains("no branch named nope"), "{message}");
}

#[tokio::test]
async fn git_pull_and_push_reach_a_local_remote_without_approval() {
    let sb = Sandbox::new();
    let (server, bare) = tracking(&sb).await;
    let root = server.root();
    write(root, "n.txt", "n\n");
    sb.commit_all(root, "next");
    let head = rev(&sb, root, "HEAD");
    let pushed = server.ok("git_push", json!({})).await;
    assert_eq!(pushed["branch"], "main");
    assert_eq!(pushed["upstream"], "origin/main");
    assert_eq!(pushed["to"], head.as_str());
    assert_eq!(rev(&sb, &bare, "refs/heads/main"), head);

    // Another clone moves the remote on, so the pull has something to fast-forward to.
    let other = sb.path().join("other");
    let url = bare.to_str().expect("utf-8");
    sb.git(sb.path(), &["clone", "-q", url, "other"]);
    write(&other, "o.txt", "o\n");
    sb.commit_all(&other, "other");
    sb.git(&other, &["push", "-q", "origin", "main"]);
    let pulled = server.ok("git_pull", json!({})).await;
    assert_eq!(pulled["from"], head.as_str());
    assert_eq!(pulled["to"], rev(&sb, &other, "HEAD").as_str());
    assert!(root.join("o.txt").is_file());
    assert!(server.inbox().pending().expect("inbox").is_empty());
}

#[tokio::test]
async fn a_file_url_remote_counts_as_local() {
    let sb = Sandbox::new();
    let (server, bare) = tracking(&sb).await;
    let root = server.root();
    let url = format!("file://{}", bare.display());
    sb.git(root, &["remote", "set-url", "origin", &url]);
    let pulled = server.ok("git_pull", json!({})).await;
    assert_eq!(pulled["upstream"], "origin/main");
}

#[tokio::test]
async fn git_push_takes_no_set_upstream_and_is_never_read_only() {
    let sb = Sandbox::new();
    let server = serve(&sb).await;
    let tools = server.tools().await;
    for name in ["git_pull", "git_push"] {
        let tool = tools.iter().find(|t| t.name == name).expect("listed");
        let schema = serde_json::to_value(&tool.input_schema).expect("schema");
        let props = schema.get("properties").cloned().unwrap_or(json!({}));
        assert_eq!(props, json!({}), "{name}: {schema}");
        let annotations = tool.annotations.as_ref().expect("annotations");
        assert_eq!(annotations.read_only_hint, Some(false), "{name}");
        assert_eq!(annotations.open_world_hint, Some(true), "{name}");
    }
    let message = server
        .err("git_push", json!({"set_upstream": "origin"}))
        .await;
    assert!(message.contains("set_upstream"), "{message}");
}

/// Call `tool` against a remote on an untrusted host and require the refusal, filed in the
/// inbox and returned before git could have tried the network.
async fn refused(server: &TestServer, tool: &str, nothing: &str) -> String {
    let started = Instant::now();
    let message = server.err(tool, json!({})).await;
    assert!(started.elapsed() < Duration::from_secs(5), "{message}");
    assert!(
        message.contains("host `example.invalid` is not trusted for this collection"),
        "{message}"
    );
    assert!(message.contains(nothing), "{message}");
    assert!(!message.contains("nothing was sent"), "{message}");
    assert!(!message.contains("fatal"), "{message}");
    let pending = server.inbox().pending().expect("inbox");
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(pending[0].host, "example.invalid");
    assert_eq!(pending[0].request, tool);
    message
}

#[tokio::test]
async fn git_pull_from_an_untrusted_https_host_is_refused_and_filed() {
    let sb = Sandbox::new();
    let server = tracking_url(&sb, "https://example.invalid/r.git").await;
    let message = refused(&server, "git_pull", "nothing was pulled").await;
    assert!(
        message.contains("retry after the user approves"),
        "{message}"
    );
}

#[tokio::test]
async fn git_push_to_an_untrusted_scp_host_is_refused_and_filed() {
    let sb = Sandbox::new();
    let server = tracking_url(&sb, "git@example.invalid:r.git").await;
    refused(&server, "git_push", "nothing was pushed").await;
}

#[tokio::test]
async fn git_push_judges_the_push_url_not_the_fetch_url() {
    let sb = Sandbox::new();
    let (server, _) = tracking(&sb).await;
    let root = server.root();
    let push_url = "https://example.invalid/r.git";
    sb.git(root, &["remote", "set-url", "--push", "origin", push_url]);
    refused(&server, "git_push", "nothing was pushed").await;
}

#[tokio::test]
async fn a_denied_host_is_refused_without_filing() {
    let sb = Sandbox::new();
    let server = tracking_url(&sb, "git@example.invalid:r.git").await;
    server.deny("example.invalid");
    let message = server.err("git_pull", json!({})).await;
    assert!(
        message.contains("is denied for this collection; nothing was pulled"),
        "{message}"
    );
    assert!(server.inbox().pending().expect("inbox").is_empty());
}

#[tokio::test]
async fn a_remote_helper_transport_is_refused() {
    let sb = Sandbox::new();
    let server = tracking_url(&sb, "gcrypt://example.invalid/r").await;
    let message = server.err("git_push", json!({})).await;
    assert!(message.contains("nothing was pushed"), "{message}");
    assert!(!message.contains("fatal"), "{message}");
    assert!(server.inbox().pending().expect("inbox").is_empty());
}

#[tokio::test]
async fn a_trusted_host_goes_on_to_git() {
    let sb = Sandbox::new();
    let server = tracking_url(&sb, "git@example.invalid:r.git").await;
    server.trust("example.invalid");
    for tool in ["git_pull", "git_push"] {
        let message = server.err(tool, json!({})).await;
        assert!(!message.contains("not trusted"), "{tool}: {message}");
        assert!(
            message.contains("Could not read from remote repository"),
            "{tool}: {message}"
        );
    }
    assert!(server.inbox().pending().expect("inbox").is_empty());
}
