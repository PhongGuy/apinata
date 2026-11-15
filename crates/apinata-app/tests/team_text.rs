//! What the team dialogs say for each failure.

use apinata_app::team_text::{
    SyncKind, error_text, finding_line, problem_lines, problem_toast, read_only_toast,
    server_warning, sign_in_error_text, sync_status,
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

#[test]
fn every_other_state_has_its_own_line() {
    let line = |state: TeamState| problem_lines(&TeamStatus::new(state, 0, vec![]));
    assert_eq!(
        line(TeamState::Offline),
        ["The server cannot be reached. Changes are committed here and pushed once it is back."]
    );
    assert_eq!(
        line(TeamState::Blocked(Block::SignedOut)),
        [
            "You are signed out of this team server. Changes are committed here until you sign in again."
        ]
    );
    assert_eq!(
        line(TeamState::Blocked(Block::Gone)),
        ["This workspace is no longer on the server."]
    );
    assert_eq!(
        line(TeamState::Blocked(Block::Failed {
            message: " boom\n".into()
        })),
        ["Sync failed:", "boom"]
    );
    assert_eq!(line(TeamState::Committing), ["Syncing."]);
    assert_eq!(line(TeamState::Squashing), ["Syncing."]);
}

#[test]
fn each_state_has_one_status() {
    let at = |state, unpushed| TeamStatus::new(state, unpushed, vec![]);
    let cases = [
        (
            sync_status(&at(TeamState::Idle, 0), false, 0),
            (SyncKind::Synced, "synced"),
        ),
        (
            sync_status(&at(TeamState::ReadOnly { paused: false }, 0), true, 0),
            (SyncKind::Synced, "synced, read only"),
        ),
        (
            sync_status(&at(TeamState::Pulling, 0), false, 0),
            (SyncKind::Syncing, "syncing"),
        ),
        (
            sync_status(&at(TeamState::Offline, 0), false, 0),
            (SyncKind::Offline, "offline"),
        ),
        (
            sync_status(&at(TeamState::Offline, 3), false, 0),
            (SyncKind::Offline, "offline, 3 unpushed"),
        ),
        (
            sync_status(&at(TeamState::Blocked(Block::SignedOut), 0), false, 2),
            (SyncKind::Problem, "problem: signed out"),
        ),
        (
            sync_status(&at(TeamState::ReadOnly { paused: true }, 0), true, 0),
            (SyncKind::Problem, "problem: outside edits"),
        ),
        (
            sync_status(&at(TeamState::Idle, 0), false, 1),
            (SyncKind::Problem, "problem: 1 clash to settle"),
        ),
        (
            sync_status(&at(TeamState::Idle, 0), false, 2),
            (SyncKind::Problem, "problem: 2 clashes to settle"),
        ),
    ];
    for ((kind, text), (want_kind, want_text)) in cases {
        assert_eq!((kind, text.as_str()), (want_kind, want_text));
    }
}

#[test]
fn toasts_name_the_collection_and_the_reason() {
    let secret = Block::Secret {
        findings: vec![],
        more: 1,
    };
    assert_eq!(
        problem_toast("api", &secret),
        "Sync of api stopped: the server found secrets."
    );
    assert_eq!(
        read_only_toast("api"),
        "You can only read api, so your changes are not saved."
    );
}

#[test]
fn a_finding_outside_any_file_is_named_instead_of_showing_a_slash() {
    let file = Finding::new("secret_value", "refs/heads/main", "a.yml", "secret_value");
    let bare = Finding::new("secret_value", "refs/tags/v1", "/", "secret_value");
    let status = TeamStatus::new(
        TeamState::Blocked(Block::Secret {
            findings: vec![file, bare],
            more: 0,
        }),
        0,
        vec![],
    );
    let lines = problem_lines(&status);
    assert_eq!(
        lines[1],
        "a.yml: secret_value (secret_value on refs/heads/main)"
    );
    assert_eq!(
        lines[2],
        "not in a file (a commit or tag message, or a tag's target): secret_value (secret_value on refs/tags/v1)"
    );
}
