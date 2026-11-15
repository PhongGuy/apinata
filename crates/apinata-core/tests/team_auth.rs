//! Team git env, token redaction, and reading git's output as a sync failure.

use crate::common::sse_server::{Script, slow_sse_server};
use crate::git_branch::cloned;
use crate::git_support::{Sandbox, write};
use crate::git_sync::local_commit;

use std::fs;

use apinata_core::team::{Identity, SyncFailure, TeamAuth, classify, redact_token, team_options};
use apinata_core::{CancelFlag, SyncError, commit_all, push};

pub const TOKEN: &str = "plk_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const UNAUTHORIZED: &str = "HTTP/1.1 401 Unauthorized\r\nwww-authenticate: Basic realm=\"potluck\"\r\ncontent-length: 0\r\n\r\n";

fn failed(stderr: &str) -> SyncError {
    SyncError::Failed {
        stderr: stderr.to_owned(),
    }
}

fn rejected(reason: &str, stderr: &str) -> SyncError {
    SyncError::Rejected {
        reason: reason.to_owned(),
        stderr: stderr.to_owned(),
    }
}

#[test]
fn git_output_for_each_http_refusal_maps_to_its_failure() {
    let cases = [
        (
            "fatal: could not read Username for 'http://127.0.0.1:8080': terminal prompts disabled",
            SyncFailure::Unauthenticated,
        ),
        (
            "fatal: unable to access 'http://127.0.0.1:8080/git/t/w.git/': The requested URL returned error: 401",
            SyncFailure::Unauthenticated,
        ),
        (
            "fatal: unable to access 'http://127.0.0.1:8080/git/t/w.git/': The requested URL returned error: 403",
            SyncFailure::Forbidden,
        ),
        (
            "fatal: repository 'http://127.0.0.1:8080/git/t/w.git/' not found",
            SyncFailure::NotFound,
        ),
    ];
    for (stderr, want) in cases {
        assert_eq!(classify(&failed(stderr)), want, "{stderr}");
    }
}

#[test]
fn unreachable_servers_read_as_offline() {
    for stderr in [
        "fatal: unable to access 'http://127.0.0.1:1/git/t/w.git/': Failed to connect to 127.0.0.1:1 after 0 ms: Could not connect to server",
        "fatal: unable to access 'http://nonexistent.invalid/git/t/w.git/': Could not resolve host: nonexistent.invalid",
    ] {
        assert!(
            matches!(classify(&failed(stderr)), SyncFailure::Offline { .. }),
            "{stderr}"
        );
    }
}

#[test]
fn non_fast_forward_rejections_ask_for_a_pull() {
    for reason in ["non-fast-forward", "fetch first"] {
        assert_eq!(classify(&rejected(reason, "")), SyncFailure::NonFastForward);
    }
}

#[test]
fn potluck_findings_are_parsed_from_the_remote_lines() {
    let stderr = "remote: potluck-reject secret_env_file refs/heads/main .env env_file        \n\
                  remote: potluck-reject secret_value refs/heads/main environments/dev.yml secret_value        \n\
                  remote: potluck-reject-more 3        \n\
                  error: failed to push some refs to 'http://127.0.0.1:8080/git/t/w.git'";
    let SyncFailure::Secret { findings, more } =
        classify(&rejected("pre-receive hook declined", stderr))
    else {
        panic!("a secret refusal");
    };
    let got: Vec<_> = findings
        .iter()
        .map(|f| {
            (
                f.code.as_str(),
                f.git_ref.as_str(),
                f.path.as_str(),
                f.rule.as_str(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            ("secret_env_file", "refs/heads/main", ".env", "env_file"),
            (
                "secret_value",
                "refs/heads/main",
                "environments/dev.yml",
                "secret_value"
            ),
        ]
    );
    assert_eq!(more, 3);
}

#[test]
fn a_hook_refusal_without_findings_is_another_failure() {
    let got = classify(&rejected(
        "pre-receive hook declined",
        "remote: nope from hook",
    ));
    let SyncFailure::Other { message } = got else {
        panic!("{got:?}")
    };
    assert!(message.contains("pre-receive hook declined"), "{message}");
}

#[test]
fn messages_never_carry_a_token() {
    let got = classify(&failed(&format!("fatal: boom for Bearer {TOKEN}")));
    let SyncFailure::Other { message } = got else {
        panic!("{got:?}")
    };
    assert!(
        !message.contains(TOKEN) && message.contains("plk_[redacted]"),
        "{message}"
    );
}

#[test]
fn redact_token_masks_each_token_and_keeps_the_rest() {
    assert_eq!(
        redact_token(&format!("a {TOKEN} b plk_x")),
        "a plk_[redacted] b plk_[redacted]"
    );
    assert_eq!(redact_token("no token here"), "no token here");
}

#[test]
fn the_auth_debug_form_hides_the_token() {
    let shown = format!("{:?}", TeamAuth::new("http://127.0.0.1:8080/", TOKEN));
    assert!(
        !shown.contains(TOKEN) && shown.contains("http://127.0.0.1:8080"),
        "{shown}"
    );
    assert_eq!(
        TeamAuth::new("http://127.0.0.1:8080/", TOKEN).public_url(),
        "http://127.0.0.1:8080"
    );
}

#[test]
fn team_commits_are_authored_and_committed_as_the_identity() {
    let sb = Sandbox::new();
    let col = cloned(&sb);
    write(&col, "new.txt", "n\n");
    let opts = team_options(
        sb.options(),
        &TeamAuth::new("http://127.0.0.1:9", TOKEN),
        &Identity::new("Ada", "ada@example.com"),
    );
    commit_all(&col, "Save changes", &opts).unwrap();
    let who = sb.git(&col, &["log", "-1", "--format=%an <%ae>|%cn <%ce>"]);
    assert_eq!(who, "Ada <ada@example.com>|Ada <ada@example.com>\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_push_sends_the_token_only_as_a_header_and_a_401_reads_as_signed_out() {
    let server = slow_sse_server(Script::raw_head(UNAUTHORIZED.to_owned(), Vec::new())).await;
    let sb = Sandbox::new();
    let col = cloned(&sb);
    let public = server.url("");
    sb.git(
        &col,
        &[
            "remote",
            "set-url",
            "origin",
            &format!("{public}/git/t/w.git"),
        ],
    );
    local_commit(&sb, &col, "c.txt");
    let trace = sb.path().join("curl.trace");
    let base = sb.options().env("GIT_TRACE_CURL", &trace);
    let opts = team_options(
        base,
        &TeamAuth::new(public.clone(), TOKEN),
        &Identity::new("Ada", "ada@example.com"),
    );
    let err = tokio::task::spawn_blocking(move || push(&col, None, &CancelFlag::new(), &opts))
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(classify(&err), SyncFailure::Unauthenticated);
    assert!(!format!("{err:?}").contains(TOKEN));
    let heads = server.heads();
    assert!(
        heads[0].starts_with("GET /git/t/w.git/info/refs?service=git-receive-pack"),
        "{heads:?}"
    );
    assert!(
        heads[0].contains(&format!("Authorization: Bearer {TOKEN}")),
        "{heads:?}"
    );
    let traced = fs::read_to_string(&trace).unwrap();
    assert!(!traced.contains(TOKEN) && traced.contains("Authorization: Bearer <redacted>"));
}

#[test]
fn the_team_options_debug_form_hides_the_token() {
    let opts = team_options(
        Sandbox::new().options(),
        &TeamAuth::new("http://127.0.0.1:9", TOKEN),
        &Identity::new("Ada", "ada@example.com"),
    );
    let shown = format!("{opts:?}");
    assert!(
        !shown.contains(TOKEN) && shown.contains("GIT_CONFIG_VALUE_0"),
        "{shown}"
    );
}
