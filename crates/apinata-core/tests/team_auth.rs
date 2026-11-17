//! Team git env, token redaction, and reading git's output as a sync failure.

use crate::common::sse_server::{Script, slow_sse_server};
use crate::git_branch::cloned;
use crate::git_support::{Sandbox, write};
use crate::git_sync::local_commit;

use std::fs;

use apinata_core::team::{
    Finding, Identity, SyncFailure, TeamAuth, classify, redact_token, team_options,
};
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
        "fatal: unable to access 'http://127.0.0.1:18080/git/t/w.git/': Recv failure: Connection reset by peer",
        "error: RPC failed; curl 56 Recv failure: Connection reset by peer\nfatal: expected flush after ref listing",
        "fatal: unable to access 'https://team.example/git/t/w.git/': Failed to connect to team.example port 443 after 75003 ms: Couldn't connect to server",
        "fatal: unable to access 'https://team.example/git/t/w.git/': Connection timed out after 300000 milliseconds",
        "fatal: unable to access 'https://team.example/git/t/w.git/': Operation timed out after 300000 milliseconds with 0 bytes received",
        "fatal: unable to access 'http://127.0.0.1:18080/git/t/w.git/': Empty reply from server",
        "error: RPC failed; curl 18 transfer closed with outstanding read data remaining",
        "error: RPC failed; curl 55 Send failure: Broken pipe",
        "fatal: unable to access 'https://team.example/git/t/w.git/': OpenSSL SSL_connect: SSL_ERROR_SYSCALL in connection to team.example:443",
        "fatal: unable to access 'https://team.example/git/t/w.git/': The requested URL returned error: 502",
        "fatal: unable to access 'https://team.example/git/t/w.git/': The requested URL returned error: 503",
        "fatal: unable to access 'https://team.example/git/t/w.git/': The requested URL returned error: 504",
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
    let SyncFailure::Refused { findings, more } =
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

/// The (ref, path) pairs the findings of a refusal carry, for the lines given.
fn refused(lines: &[&str]) -> Vec<(String, String)> {
    let stderr = lines
        .iter()
        .map(|l| format!("remote: {l}        \n"))
        .collect::<String>()
        + "error: failed to push some refs";
    match classify(&rejected("pre-receive hook declined", &stderr)) {
        SyncFailure::Refused { findings, .. } => {
            findings.into_iter().map(|f| (f.git_ref, f.path)).collect()
        }
        SyncFailure::Other { .. } => Vec::new(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn encoded_spaces_and_percents_in_a_path_are_decoded() {
    let got = refused(&["potluck-reject secret_value refs/heads/a%20b my%20dir/50%25.yml rule"]);
    assert_eq!(
        got,
        [("refs/heads/a b".to_owned(), "my dir/50%.yml".to_owned())]
    );
}

#[test]
fn encoded_control_bytes_and_lower_case_hex_are_decoded() {
    let got = refused(&["potluck-reject c r a%0Ab%7fc%0a rule"]);
    assert_eq!(got, [("r".to_owned(), "a\nb\u{7f}c\n".to_owned())]);
}

#[test]
fn an_encoded_invalid_utf8_byte_is_shown_lossily() {
    let got = refused(&["potluck-reject c r bad%FFname rule"]);
    assert_eq!(got, [("r".to_owned(), "bad\u{fffd}name".to_owned())]);
}

#[test]
fn a_line_with_another_field_count_is_ignored() {
    assert!(refused(&["potluck-reject c r a b rule"]).is_empty());
    assert!(refused(&["potluck-reject c r rule"]).is_empty());
}

#[test]
fn a_line_with_a_malformed_escape_is_ignored() {
    for path in ["50%", "a%2", "a%zz", "%G1", "a%+1"] {
        let line = format!("potluck-reject c r {path} rule");
        assert!(refused(&[&line]).is_empty(), "{path}");
    }
    let got = refused(&[
        "potluck-reject c r bad%zz rule",
        "potluck-reject c r ok rule",
    ]);
    assert_eq!(got, [("r".to_owned(), "ok".to_owned())]);
}

/// The (rule, detail) pairs the findings of a refusal carry, for the lines given.
fn details(lines: &[&str]) -> Vec<(String, Option<String>)> {
    let stderr = lines
        .iter()
        .map(|l| format!("remote: {l}        \n"))
        .collect::<String>();
    match classify(&rejected("pre-receive hook declined", &stderr)) {
        SyncFailure::Refused { findings, .. } => {
            findings.into_iter().map(|f| (f.rule, f.detail)).collect()
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_detail_line_attaches_decoded_to_the_finding_before_it() {
    let got = details(&[
        "potluck-reject secret_value refs/heads/main environments/prod.yml secret_value",
        "potluck-reject-detail token%20and%20api%2Ekey",
        "potluck-reject format_unsupported refs/heads/main opencollection.yml format",
        "potluck-reject-detail format%3A%20v9",
        "potluck-reject safety_relaxation refs/heads/main a.yml scripts",
    ]);
    assert_eq!(
        got,
        [
            ("secret_value".into(), Some("token and api.key".into())),
            ("format".into(), Some("format: v9".into())),
            ("scripts".into(), None),
        ]
    );
}

#[test]
fn an_orphan_detail_line_is_ignored() {
    let got = details(&[
        "potluck-reject-detail before%20any",
        "potluck-reject c r a.yml one",
        "potluck-reject-detail first",
        "potluck-reject-detail second",
        "potluck-reject-more 2",
        "potluck-reject-detail after%20more",
    ]);
    assert_eq!(got, [("one".into(), Some("first".into()))]);
}

#[test]
fn a_detail_after_a_dropped_finding_does_not_reach_the_one_before() {
    let got = details(&[
        "potluck-reject c r a.yml one",
        "potluck-reject c r bad%zz two",
        "potluck-reject-detail for%20two",
    ]);
    assert_eq!(got, [("one".into(), None)]);
}

#[test]
fn a_malformed_detail_escape_drops_the_detail_and_keeps_the_finding() {
    let got = details(&[
        "potluck-reject c r a.yml one",
        "potluck-reject-detail bad%zz",
    ]);
    assert_eq!(got, [("one".into(), None)]);
}

#[test]
fn only_codes_starting_with_secret_are_secrets() {
    let finding = |code| Finding::new(code, "r", "p", "rule");
    assert!(finding("secret_value").is_secret());
    assert!(finding("secret_pattern").is_secret());
    assert!(!finding("format_unsupported").is_secret());
    assert!(!finding("safety_relaxation").is_secret());
    assert!(!finding("secretive").is_secret());
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

#[test]
fn the_team_options_debug_form_hides_a_token_without_the_usual_prefix() {
    let token = "unprefixed-s3cret-value";
    let opts = team_options(
        Sandbox::new().options(),
        &TeamAuth::new("http://127.0.0.1:9", token),
        &Identity::new("Ada", "ada@example.com"),
    );
    let shown = format!("{opts:?}");
    assert!(
        !shown.contains(token) && shown.contains("GIT_CONFIG_VALUE_0"),
        "{shown}"
    );
}
