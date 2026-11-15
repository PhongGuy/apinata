//! What the team dialogs say for each failure.

use apinata_app::team_text::{
    error_text, finding_line, problem_lines, server_warning, sign_in_error_text,
};
use apinata_core::team::{Block, Finding, ServerUrl, TeamError, TeamState, TeamStatus};

fn all() -> Vec<TeamError> {
    vec![
        TeamError::BadServerUrl("Enter the server address.".into()),
        TeamError::PasswordSignInOff,
        TeamError::Unauthenticated,
        TeamError::Forbidden("x".into()),
        TeamError::NotFound("x".into()),
        TeamError::Conflict("x".into()),
        TeamError::Invalid {
            field: Some("slug".into()),
            message: "must be lowercase".into(),
        },
        TeamError::RateLimited {
            retry_after_secs: Some(30),
        },
        TeamError::RateLimited {
            retry_after_secs: None,
        },
        TeamError::Unavailable,
        TeamError::Server {
            status: 500,
            code: "internal".into(),
            message: "x".into(),
        },
        TeamError::Offline("refused".into()),
        TeamError::Timeout,
        TeamError::Protocol("x".into()),
        TeamError::Secrets("locked".into()),
        TeamError::Store("disk full".into()),
    ]
}

#[test]
fn a_401_reads_as_a_wrong_password_at_sign_in_and_as_signed_out_elsewhere() {
    assert_eq!(
        sign_in_error_text(&TeamError::Unauthenticated),
        "Sign-in failed. Check the username and password."
    );
    assert_eq!(
        error_text(&TeamError::Unauthenticated),
        "You are signed out of this server. Sign in again."
    );
    assert_eq!(
        sign_in_error_text(&TeamError::Timeout),
        error_text(&TeamError::Timeout)
    );
}

#[test]
fn rate_limits_say_when_to_retry_and_field_errors_name_the_field() {
    assert_eq!(
        error_text(&TeamError::RateLimited {
            retry_after_secs: Some(30)
        }),
        "Too many attempts. Try again in 30 seconds."
    );
    assert_eq!(
        error_text(&TeamError::RateLimited {
            retry_after_secs: None
        }),
        "Too many attempts. Try again shortly."
    );
    assert_eq!(
        error_text(&TeamError::Invalid {
            field: Some("slug".into()),
            message: "must be lowercase".into()
        }),
        "slug: must be lowercase"
    );
    assert_eq!(
        error_text(&TeamError::Server {
            status: 500,
            code: String::new(),
            message: String::new()
        }),
        "The server answered with an error (HTTP 500)."
    );
}

#[test]
fn no_text_uses_a_double_dash_an_emoji_or_another_client_name() {
    for e in all() {
        for text in [error_text(&e), sign_in_error_text(&e)] {
            assert!(!text.is_empty(), "{e:?}");
            assert!(!text.contains(" -- "), "{text}");
            assert!(!text.to_lowercase().contains("bruno"), "{text}");
            assert!(text.chars().all(|c| (c as u32) < 0x2000), "{text}");
        }
    }
}

#[test]
fn only_plain_http_to_another_host_warns() {
    let warn = |s: &str| server_warning(&ServerUrl::parse(s).unwrap());
    assert!(warn("http://team.example.com").is_some());
    assert!(warn("https://team.example.com").is_none());
    assert!(warn("http://127.0.0.1:8080").is_none());
}

#[test]
fn secret_lines_name_path_rule_code_and_ref_and_count_the_rest() {
    let found = Finding::new(
        "secret_value",
        "refs/heads/main",
        "environments/dev.yml",
        "secret_value",
    );
    let status = TeamStatus::new(
        TeamState::Blocked(Block::Secret {
            findings: vec![found.clone()],
            more: 3,
        }),
        2,
        vec![],
    );
    assert_eq!(
        finding_line(&found),
        "environments/dev.yml: secret_value (secret_value on refs/heads/main)"
    );
    assert_eq!(
        problem_lines(&status),
        [
            "The server refused the push because it found secrets:",
            "environments/dev.yml: secret_value (secret_value on refs/heads/main)",
            "and 3 more",
            "Remove them, save, then Retry.",
            "2 local commits are not on the server yet.",
        ]
    );
}

#[test]
fn a_paused_read_only_folder_explains_discard_and_counts_one_commit() {
    let status = TeamStatus::new(TeamState::ReadOnly { paused: true }, 1, vec![]);
    assert_eq!(
        problem_lines(&status),
        [
            "You can only read this workspace, so changes are not saved.",
            "Files changed here outside the app hold back updates. Discard them to keep pulling.",
            "1 local commit is not on the server yet.",
        ]
    );
}

#[test]
fn quiet_states_say_so_in_one_line() {
    assert_eq!(
        problem_lines(&TeamStatus::new(TeamState::Idle, 0, vec![])),
        ["In sync."]
    );
    assert_eq!(
        problem_lines(&TeamStatus::new(TeamState::Pushing, 4, vec![])),
        ["Syncing."]
    );
    let hook = TeamStatus::new(
        TeamState::Blocked(Block::Hook {
            output: "nope\n".into(),
        }),
        0,
        vec![],
    );
    assert_eq!(
        problem_lines(&hook),
        ["A commit hook refused the commit:", "nope"]
    );
}
