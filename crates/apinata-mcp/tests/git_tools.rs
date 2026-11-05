//! The read-only `git_status` and `git_diff` tools return core's JSON shapes. Every server
//! here runs git with the sandbox's options, so the developer's git config never applies.

use crate::common;
use crate::git_support;

use std::path::Path;

use apinata_core::{GitOptions, collection_status_with, file_diff_with};
use common::{ROOT, TestServer, request};
use git_support::Sandbox;
use serde_json::json;

/// The sandbox options, with discovery also stopped above the served root.
fn git_options(sb: &Sandbox, root: &Path) -> GitOptions {
    sb.options()
        .env("GIT_CEILING_DIRECTORIES", root.parent().expect("parent"))
}

async fn serve(sb: &Sandbox, files: &[(&str, &str)]) -> TestServer {
    TestServer::start_with(files, |options| {
        let git = git_options(sb, options.root());
        options.git(git)
    })
    .await
}

/// A served collection that is a repository: users/get.yml committed, then its URL edited.
async fn edited(sb: &Sandbox) -> TestServer {
    let get = request("get", 1, "GET", "https://example.com/v1");
    let server = serve(
        sb,
        &[
            ("opencollection.yml", ROOT),
            ("users/get.yml", get.as_str()),
        ],
    )
    .await;
    sb.git(server.root(), &["init", "-q", "-b", "main"]);
    sb.commit_all(server.root(), "base");
    server.write("users/get.yml", &get.replace("/v1", "/v2"));
    server
}

#[tokio::test]
async fn git_tools_are_read_only_with_output_schemas() {
    let sb = Sandbox::new();
    let server = serve(&sb, &[("opencollection.yml", ROOT)]).await;
    let tools = server.tools().await;
    for name in ["git_status", "git_diff"] {
        let tool = tools
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("{name} listed"));
        let annotations = tool.annotations.as_ref().expect("annotations");
        assert_eq!(annotations.read_only_hint, Some(true), "{name}");
        assert!(tool.output_schema.is_some(), "{name}");
    }
}

#[tokio::test]
async fn git_status_returns_the_core_state() {
    let sb = Sandbox::new();
    let server = edited(&sb).await;
    let got = server.ok("git_status", json!({})).await;
    assert_eq!(got["state"], "repo");
    assert_eq!(got["branch"], "main");
    assert_eq!(got["files"], json!({"users/get.yml": {"kind": "modified"}}));
    let options = git_options(&sb, server.root());
    let core = collection_status_with(server.root(), &options).unwrap();
    assert_eq!(got, serde_json::to_value(core).unwrap());
}

#[tokio::test]
async fn git_status_outside_a_repository_is_not_a_repo() {
    let sb = Sandbox::new();
    let server = serve(&sb, &[("opencollection.yml", ROOT)]).await;
    assert_eq!(
        server.ok("git_status", json!({})).await,
        json!({"state": "not_a_repo"})
    );
}

#[tokio::test]
async fn git_diff_returns_the_core_file_diff() {
    let sb = Sandbox::new();
    let server = edited(&sb).await;
    let got = server
        .ok("git_diff", json!({"path": "users/get.yml"}))
        .await;
    assert_eq!(got["status"], json!({"kind": "modified"}));
    assert_eq!(got["sections"], json!(["url"]));
    let options = git_options(&sb, server.root());
    let core = file_diff_with(server.root(), Path::new("users/get.yml"), &options).unwrap();
    assert_eq!(got, serde_json::to_value(core).unwrap());
}

#[tokio::test]
async fn git_diff_refuses_a_file_that_is_not_a_request_and_shows_none_of_it() {
    let sb = Sandbox::new();
    let server = edited(&sb).await;
    server.write(".env", "TOKEN=hunter2\n");
    let err = server.err("git_diff", json!({"path": ".env"})).await;
    assert!(err.contains(".env"), "{err}");
    assert!(!err.contains("hunter2"), "{err}");
}

#[tokio::test]
async fn git_diff_outside_a_repository_says_so_as_the_cli_does() {
    let sb = Sandbox::new();
    let server = serve(
        &sb,
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", "info:\n  name: a\n"),
        ],
    )
    .await;
    let err = server.err("git_diff", json!({"path": "a.yml"})).await;
    assert_eq!(
        err,
        format!("{} is not in a git repository.", server.root().display())
    );
}

/// A server whose git is missing from `PATH`, holding one request file `a.yml`.
async fn without_git(sb: &Sandbox) -> TestServer {
    TestServer::start_with(
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", "info:\n  name: a\n"),
        ],
        |options| {
            let git = git_options(sb, options.root()).env("PATH", sb.empty_bin());
            options.git(git)
        },
    )
    .await
}

#[tokio::test]
async fn git_diff_without_git_says_git_was_not_found() {
    let sb = Sandbox::new();
    let server = without_git(&sb).await;
    assert_eq!(
        server.ok("git_status", json!({})).await,
        json!({"state": "unavailable"})
    );
    let err = server.err("git_diff", json!({"path": "a.yml"})).await;
    assert_eq!(
        err,
        "git was not found on PATH, so changes cannot be shown."
    );
}

#[tokio::test]
async fn git_diff_without_git_still_names_a_refused_path() {
    let sb = Sandbox::new();
    let server = without_git(&sb).await;
    server.write(".env", "TOKEN=hunter2\n");
    for path in [".env", "../a.yml"] {
        let err = server.err("git_diff", json!({"path": path})).await;
        assert!(err.contains("is not a request file"), "{path}: {err}");
        assert!(!err.contains("git was not found"), "{path}: {err}");
    }
}

#[tokio::test]
async fn git_diff_reads_a_doubled_slash_or_leading_dot_as_the_plain_path() {
    let sb = Sandbox::new();
    let server = edited(&sb).await;
    let plain = server
        .ok("git_diff", json!({"path": "users/get.yml"}))
        .await;
    for spelled in ["users//get.yml", "users/./get.yml", "./users/get.yml"] {
        let got = server.ok("git_diff", json!({"path": spelled})).await;
        assert_eq!(got, plain, "{spelled}");
    }
}

/// The subject of the newest commit.
fn head_subject(sb: &Sandbox, root: &Path) -> String {
    sb.git(root, &["log", "-1", "--format=%s"])
        .trim()
        .to_owned()
}

#[tokio::test]
async fn git_commit_is_listed_as_a_local_write_that_is_not_destructive() {
    let sb = Sandbox::new();
    let server = serve(&sb, &[("opencollection.yml", ROOT)]).await;
    let tools = server.tools().await;
    let tool = tools
        .iter()
        .find(|t| t.name == "git_commit")
        .expect("git_commit listed");
    let annotations = tool.annotations.as_ref().expect("annotations");
    assert_ne!(annotations.read_only_hint, Some(true));
    assert_eq!(annotations.destructive_hint, Some(false));
    assert_eq!(annotations.open_world_hint, Some(false));
    let description = tool.description.as_deref().unwrap_or_default();
    assert!(description.contains("local"), "{description}");
    assert!(description.contains("hooks"), "{description}");
}

#[tokio::test]
async fn git_commit_stages_a_path_and_commits_it() {
    let sb = Sandbox::new();
    let server = edited(&sb).await;
    server.write("other.yml", "info:\n  name: other\n");
    let got = server
        .ok(
            "git_commit",
            json!({"message": "change url\n\nbody", "paths": ["users/get.yml"]}),
        )
        .await;
    assert_eq!(got["subject"], "change url");
    assert_eq!(got["short_id"], sb.short_head(server.root()));
    assert_eq!(head_subject(&sb, server.root()), "change url");
    // The other file was not named, so it stays untracked.
    let files = sb.git(server.root(), &["status", "--porcelain"]);
    assert!(files.contains("?? other.yml"), "{files}");
}

#[tokio::test]
async fn git_commit_with_a_dot_commits_everything() {
    let sb = Sandbox::new();
    let server = edited(&sb).await;
    server.write("other.yml", "info:\n  name: other\n");
    server
        .ok("git_commit", json!({"message": "all", "paths": ["."]}))
        .await;
    assert_eq!(sb.git(server.root(), &["status", "--porcelain"]), "");
}

#[tokio::test]
async fn git_commit_without_paths_commits_what_is_staged() {
    let sb = Sandbox::new();
    let server = edited(&sb).await;
    sb.git(server.root(), &["add", "users/get.yml"]);
    server.ok("git_commit", json!({"message": "staged"})).await;
    assert_eq!(head_subject(&sb, server.root()), "staged");
}

#[tokio::test]
async fn git_commit_with_nothing_staged_is_refused() {
    let sb = Sandbox::new();
    let server = edited(&sb).await;
    let err = server.err("git_commit", json!({"message": "m"})).await;
    assert!(err.contains("nothing"), "{err}");
}

#[tokio::test]
async fn git_commit_with_paths_refuses_when_other_files_are_staged() {
    let sb = Sandbox::new();
    let server = edited(&sb).await;
    server.write("other.yml", "info:\n  name: other\n");
    sb.git(server.root(), &["add", "other.yml"]);
    let err = server
        .err(
            "git_commit",
            json!({"message": "m", "paths": ["users/get.yml"]}),
        )
        .await;
    assert!(err.contains("other.yml"), "{err}");
    // Nothing was staged or committed by the refused call.
    assert_eq!(head_subject(&sb, server.root()), "base");
    let status = sb.git(server.root(), &["status", "--porcelain"]);
    assert!(status.contains(" M users/get.yml"), "{status}");
}

#[tokio::test]
async fn git_commit_allows_paths_that_are_already_staged_themselves() {
    let sb = Sandbox::new();
    let server = edited(&sb).await;
    sb.git(server.root(), &["add", "users/get.yml"]);
    server
        .ok(
            "git_commit",
            json!({"message": "m", "paths": ["users/get.yml"]}),
        )
        .await;
    assert_eq!(head_subject(&sb, server.root()), "m");
}

#[tokio::test]
async fn git_commit_refuses_an_empty_message() {
    let sb = Sandbox::new();
    let server = edited(&sb).await;
    let err = server
        .err("git_commit", json!({"message": "  ", "paths": ["."]}))
        .await;
    assert!(err.contains("empty"), "{err}");
    assert_eq!(head_subject(&sb, server.root()), "base");
}

#[tokio::test]
async fn git_commit_outside_a_repository_says_so() {
    let sb = Sandbox::new();
    let server = serve(&sb, &[("opencollection.yml", ROOT)]).await;
    let err = server
        .err("git_commit", json!({"message": "m", "paths": ["."]}))
        .await;
    assert_eq!(
        err,
        format!("{} is not in a git repository.", server.root().display())
    );
}

#[tokio::test]
async fn git_commit_with_paths_refuses_when_the_status_list_is_truncated() {
    let sb = Sandbox::new();
    let server = TestServer::start_with(
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", "info:\n  name: a\n"),
            ("b.yml", "info:\n  name: b\n"),
        ],
        |options| {
            let git = git_options(&sb, options.root()).file_cap(1);
            options.git(git)
        },
    )
    .await;
    sb.git(server.root(), &["init", "-q", "-b", "main"]);
    sb.commit_all(server.root(), "base");
    server.write("a.yml", "info:\n  name: a2\n");
    server.write("b.yml", "info:\n  name: b2\n");
    sb.git(server.root(), &["add", "a.yml", "b.yml"]);
    let err = server
        .err("git_commit", json!({"message": "m", "paths": ["a.yml"]}))
        .await;
    assert!(err.contains("too many changes"), "{err}");
    assert_eq!(head_subject(&sb, server.root()), "base");
}

#[tokio::test]
async fn git_commit_counts_a_rename_from_outside_the_paths_as_other_staged() {
    let sb = Sandbox::new();
    let server = serve(
        &sb,
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", "info:\n  name: a\n"),
        ],
    )
    .await;
    sb.git(server.root(), &["init", "-q", "-b", "main"]);
    sb.commit_all(server.root(), "base");
    server.write("users/keep.yml", "info:\n  name: keep\n");
    sb.git(server.root(), &["mv", "a.yml", "users/x.yml"]);
    let err = server
        .err("git_commit", json!({"message": "m", "paths": ["users"]}))
        .await;
    assert!(err.contains("a.yml"), "{err}");
    assert_eq!(head_subject(&sb, server.root()), "base");
}

#[tokio::test]
async fn git_commit_with_empty_paths_is_refused() {
    let sb = Sandbox::new();
    let server = edited(&sb).await;
    let err = server
        .err("git_commit", json!({"message": "m", "paths": []}))
        .await;
    assert!(err.contains("paths must not be empty"), "{err}");
}

#[tokio::test]
async fn git_commit_refuses_parent_and_absolute_paths() {
    let sb = Sandbox::new();
    let server = edited(&sb).await;
    let outside = sb.path().display().to_string();
    for path in ["../x.yml", outside.as_str()] {
        let err = server
            .err("git_commit", json!({"message": "m", "paths": [path]}))
            .await;
        assert!(
            err.contains("not a path inside the collection"),
            "{path}: {err}"
        );
    }
    assert_eq!(head_subject(&sb, server.root()), "base");
}
