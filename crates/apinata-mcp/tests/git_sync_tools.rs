//! `git_branches`, `git_switch_branch`, `git_pull` and `git_push`. Every remote is a local bare
//! repository, a 127.0.0.1 server or an unreachable `.invalid` URL, and ssh is replaced by a
//! recorder that fails, so no test reaches the network.

use crate::common;
use crate::git_support;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use common::{ROOT, TestServer};
use git_support::{Sandbox, write};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn serve(sb: &Sandbox) -> TestServer {
    serve_with(sb, &[]).await
}

/// A served repository whose git runs with `env` added and ssh replaced by [`ssh_log`]'s
/// recorder, which exits 1.
async fn serve_with(sb: &Sandbox, env: &[(&str, &str)]) -> TestServer {
    let log = ssh_log(sb);
    let body = format!("#!/bin/sh\necho \"$@\" >> '{}'\nexit 1\n", log.display());
    let ssh = sb.fake_tool("ssh-bin", "ssh", &body).join("ssh");
    let server = TestServer::start_with(&[("opencollection.yml", ROOT)], |options| {
        let mut git = sb
            .options()
            .env(
                "GIT_CEILING_DIRECTORIES",
                options.root().parent().expect("parent"),
            )
            .env("GIT_SSH_COMMAND", &ssh);
        for (key, value) in env {
            git = git.env(key, value);
        }
        options.git(git)
    })
    .await;
    sb.git(server.root(), &["init", "-q", "-b", "main"]);
    sb.commit_all(server.root(), "base");
    server
}

/// Where the ssh recorder appends each call's arguments.
fn ssh_log(sb: &Sandbox) -> PathBuf {
    sb.path().join("ssh-calls.txt")
}

/// A 127.0.0.1 server answering every request with `reply` and counting the requests.
async fn http_server(reply: String) -> (u16, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let hits = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&hits);
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            counted.fetch_add(1, Ordering::SeqCst);
            let reply = reply.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let mut seen = Vec::new();
                while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => seen.extend_from_slice(&buf[..n]),
                    }
                }
                let _ = stream.write_all(reply.as_bytes()).await;
            });
        }
    });
    (port, hits)
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
    tracking_url_with(sb, url, &[]).await
}

/// As [`tracking_url`], with git running under `env` too.
async fn tracking_url_with(sb: &Sandbox, url: &str, env: &[(&str, &str)]) -> TestServer {
    let server = serve_with(sb, env).await;
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

#[tokio::test]
async fn a_pull_never_fetches_a_submodule_from_its_own_remote() {
    let sb = Sandbox::new();
    let (server, bare) = tracking(&sb).await;
    let root = server.root();
    let sub = sb.repo("sub");
    write(&sub, "s.txt", "1\n");
    sb.commit_all(&sub, "s1");
    let sub_url = sub.to_str().expect("utf-8");
    let add = ["-c", "protocol.file.allow=always", "submodule", "add", "-q"];
    sb.git(root, &[&add[..], &[sub_url, "sub"]].concat());
    sb.git(root, &["commit", "-q", "-m", "add sub"]);
    sb.git(root, &["push", "-q", "origin", "main"]);
    // The checked-out submodule fetches from an ssh host the trust check never saw.
    let set_url = ["remote", "set-url", "origin", "ssh://evil.invalid/sub"];
    sb.git(&root.join("sub"), &set_url);
    // Another clone moves the submodule to a commit this checkout lacks.
    write(&sub, "s.txt", "2\n");
    sb.commit_all(&sub, "s2");
    let bumped = format!("160000,{},sub", rev(&sb, &sub, "HEAD"));
    let other = sb.path().join("other");
    sb.git(
        sb.path(),
        &["clone", "-q", bare.to_str().expect("utf-8"), "other"],
    );
    sb.git(&other, &["update-index", "--cacheinfo", &bumped]);
    sb.git(&other, &["commit", "-q", "-m", "bump"]);
    sb.git(&other, &["push", "-q", "origin", "main"]);

    let result = server.call("git_pull", json!({})).await;
    assert!(!ssh_log(&sb).exists(), "ssh ran: {result:?}");
    assert_ne!(result.is_error, Some(true), "{result:?}");
}

#[tokio::test]
async fn a_local_http_remote_cannot_redirect_a_pull_or_push_elsewhere() {
    let sb = Sandbox::new();
    let (elsewhere, followed) = http_server(
        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned(),
    )
    .await;
    let redirect = format!(
        "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{elsewhere}/x\r\nContent-Length: 0\r\n\
         Connection: close\r\n\r\n"
    );
    let (port, asked) = http_server(redirect).await;
    let server = tracking_url(&sb, &format!("http://127.0.0.1:{port}/r.git")).await;
    for tool in ["git_pull", "git_push"] {
        server.err(tool, json!({})).await;
    }
    assert!(asked.load(Ordering::SeqCst) >= 2);
    assert_eq!(
        followed.load(Ordering::SeqCst),
        0,
        "a redirect was followed"
    );
}

/// Require `tool` to be refused over the untrusted proxy host `proxy.invalid`, filed for it.
async fn refused_proxy(server: &TestServer, tool: &str, nothing: &str) {
    let message = server.err(tool, json!({})).await;
    assert!(
        message.contains("proxy host `proxy.invalid` is not trusted for this collection"),
        "{message}"
    );
    assert!(message.contains(nothing), "{message}");
    assert!(!message.contains("secret-pw"), "{message}");
    let pending = server.inbox().pending().expect("inbox");
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(pending[0].host, "proxy.invalid");
}

#[tokio::test]
async fn a_configured_http_proxy_must_be_trusted_too() {
    let sb = Sandbox::new();
    let server = tracking_url(&sb, "http://127.0.0.1:9/r.git").await;
    let proxy = "http://user:secret-pw@proxy.invalid:3128";
    sb.git(server.root(), &["config", "http.proxy", proxy]);
    refused_proxy(&server, "git_pull", "nothing was pulled").await;
}

#[tokio::test]
async fn a_remote_proxy_must_be_trusted_too() {
    let sb = Sandbox::new();
    let server = tracking_url(&sb, "https://127.0.0.1:9/r.git").await;
    let proxy = "proxy.invalid:3128";
    sb.git(server.root(), &["config", "remote.origin.proxy", proxy]);
    refused_proxy(&server, "git_push", "nothing was pushed").await;
}

#[tokio::test]
async fn a_proxy_from_the_environment_must_be_trusted_too() {
    for name in ["HTTPS_PROXY", "https_proxy", "all_proxy", "ALL_PROXY"] {
        let sb = Sandbox::new();
        let env = [(name, "http://proxy.invalid:3128")];
        let server = tracking_url_with(&sb, "https://127.0.0.1:9/r.git", &env).await;
        refused_proxy(&server, "git_pull", "nothing was pulled").await;
    }
}

#[tokio::test]
async fn a_local_proxy_needs_no_approval() {
    let sb = Sandbox::new();
    let server = tracking_url(&sb, "http://127.0.0.1:9/r.git").await;
    sb.git(
        server.root(),
        &["config", "http.proxy", "http://127.0.0.1:9"],
    );
    let message = server.err("git_pull", json!({})).await;
    assert!(!message.contains("not trusted"), "{message}");
    assert!(server.inbox().pending().expect("inbox").is_empty());
}

#[tokio::test]
async fn a_remote_that_runs_its_own_command_is_refused() {
    for (key, tool, nothing) in [
        ("remote.origin.uploadpack", "git_pull", "nothing was pulled"),
        (
            "remote.origin.receivepack",
            "git_push",
            "nothing was pushed",
        ),
        ("remote.origin.vcs", "git_pull", "nothing was pulled"),
    ] {
        let sb = Sandbox::new();
        let (server, _) = tracking(&sb).await;
        sb.git(server.root(), &["config", key, "touch pwned"]);
        let message = server.err(tool, json!({})).await;
        assert!(message.contains(key), "{key}: {message}");
        assert!(message.contains(nothing), "{key}: {message}");
        assert!(!server.root().join("pwned").exists(), "{key}");
    }
}

#[tokio::test]
async fn a_repo_setting_that_follows_redirects_is_refused() {
    let sb = Sandbox::new();
    let (elsewhere, followed) = http_server(
        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned(),
    )
    .await;
    let redirect = format!(
        "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{elsewhere}/x\r\nContent-Length: 0\r\n\
         Connection: close\r\n\r\n"
    );
    let (port, _) = http_server(redirect).await;
    let base = format!("http://127.0.0.1:{port}/");
    let server = tracking_url(&sb, &format!("{base}r.git")).await;
    let key = format!("http.{base}.followRedirects");
    sb.git(server.root(), &["config", &key, "true"]);
    for (tool, nothing) in [
        ("git_pull", "nothing was pulled"),
        ("git_push", "nothing was pushed"),
    ] {
        let message = server.err(tool, json!({})).await;
        assert!(message.contains("http.followRedirects"), "{message}");
        assert!(message.contains(nothing), "{message}");
    }
    assert_eq!(
        followed.load(Ordering::SeqCst),
        0,
        "a redirect was followed"
    );
}

#[tokio::test]
async fn a_git_protocol_proxy_command_is_refused() {
    let sb = Sandbox::new();
    let marker = sb.path().join("proxied");
    let body = format!("#!/bin/sh\ntouch '{}'\nexit 1\n", marker.display());
    let proxy = sb.fake_tool("proxy-bin", "proxy", &body).join("proxy");
    let proxy = proxy.to_str().expect("utf-8");
    let server = tracking_url(&sb, "git://127.0.0.1:9/r.git").await;
    sb.git(server.root(), &["config", "core.gitProxy", proxy]);
    let message = server.err("git_pull", json!({})).await;
    assert!(message.contains("core.gitProxy"), "{message}");
    assert!(message.contains("nothing was pulled"), "{message}");
    assert!(!marker.exists());

    let sb = Sandbox::new();
    let env = [("GIT_PROXY_COMMAND", proxy)];
    let server = tracking_url_with(&sb, "git://127.0.0.1:9/r.git", &env).await;
    let message = server.err("git_push", json!({})).await;
    assert!(message.contains("GIT_PROXY_COMMAND"), "{message}");
    assert!(message.contains("nothing was pushed"), "{message}");
    assert!(!marker.exists());
}
