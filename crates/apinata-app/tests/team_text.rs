//! What the team dialogs say for each failure.

use apinata_app::team_text::{
    SyncKind, clone_stopped_text, error_text, finding_line, folder_exists_text, problem_lines,
    problem_toast, read_only_toast, server_warning, sign_in_error_text, sync_status,
    unopenable_text,
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
        TeamError::Unavailable(String::new()),
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
        TeamError::Busy,
        TeamError::Client("no TLS backend".into()),
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
fn a_client_that_cannot_start_is_not_blamed_on_the_connection() {
    let text = error_text(&TeamError::Client("no TLS backend".into()));
    assert_ne!(text, error_text(&TeamError::Offline("x".into())));
    assert_eq!(
        text,
        "Apinata could not start its connection to servers: no TLS backend"
    );
}

#[test]
fn the_workspace_list_texts_name_the_workspace_and_the_folder() {
    assert_eq!(
        unopenable_text("the slug is not a folder name"),
        "This workspace cannot be opened: the slug is not a folder name."
    );
    assert_eq!(
        folder_exists_text("api", std::path::Path::new("/w")),
        "A folder named api already exists in /w."
    );
    assert_eq!(
        clone_stopped_text(),
        "The clone stopped unexpectedly. Try again."
    );
}

#[test]
fn a_busy_team_file_asks_to_try_again() {
    assert_eq!(
        error_text(&TeamError::Busy),
        "Another Apinata window or command is changing the team server list. Try again."
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
fn hidden_characters_in_a_finding_are_shown_escaped() {
    let found = Finding::new(
        "secret\u{202E}x",
        "refs/heads/a\u{2028}b",
        "x\u{202E}lmy.yml\u{85}\u{200D}",
        "rule\u{2029}",
    )
    .with_detail("key\u{200B}name");
    assert_eq!(
        finding_line(&found),
        "x\\u{202E}lmy.yml\\u{85}\\u{200D}: rule\\u{2029} \
         (secret\\u{202E}x: key\\u{200B}name) on refs/\u{2060}heads/\u{2060}a\\u{2028}b"
    );
}

#[test]
fn ordinary_non_ascii_in_a_finding_is_shown_unchanged() {
    let found = Finding::new(
        "secret_value",
        "refs/heads/main",
        "café/日本語.yml",
        "secret_value",
    );
    assert_eq!(
        finding_line(&found),
        "café/日本語.yml: secret_value on refs/\u{2060}heads/\u{2060}main"
    );
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
        TeamState::Blocked(Block::Refused {
            findings: vec![found.clone()],
            more: 3,
        }),
        2,
        vec![],
    );
    assert_eq!(
        finding_line(&found),
        "environments/dev.yml: secret_value on refs/\u{2060}heads/\u{2060}main"
    );
    assert_eq!(
        problem_lines(&status, 0, 0),
        [
            "The server refused the push because it found secrets:",
            "environments/dev.yml: secret_value on refs/\u{2060}heads/\u{2060}main",
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
        problem_lines(&status, 0, 0),
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
        problem_lines(&TeamStatus::new(TeamState::Idle, 0, vec![]), 0, 0),
        ["In sync."]
    );
    assert_eq!(
        problem_lines(&TeamStatus::new(TeamState::Pushing, 4, vec![]), 0, 0),
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
        problem_lines(&hook, 0, 0),
        ["A commit hook refused the commit:", "nope"]
    );
}

#[test]
fn every_other_state_has_its_own_line() {
    let line = |state: TeamState| problem_lines(&TeamStatus::new(state, 0, vec![]), 0, 0);
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
            sync_status(&at(TeamState::Idle, 0), false, 0, 0),
            (SyncKind::Synced, "synced"),
        ),
        (
            sync_status(&at(TeamState::ReadOnly { paused: false }, 0), true, 0, 0),
            (SyncKind::Synced, "synced, read only"),
        ),
        (
            sync_status(&at(TeamState::Pulling, 0), false, 0, 0),
            (SyncKind::Syncing, "syncing"),
        ),
        (
            sync_status(&at(TeamState::Offline, 0), false, 0, 0),
            (SyncKind::Offline, "offline"),
        ),
        (
            sync_status(&at(TeamState::Offline, 3), false, 0, 0),
            (SyncKind::Offline, "offline, 3 unpushed"),
        ),
        (
            sync_status(&at(TeamState::Blocked(Block::SignedOut), 0), false, 2, 0),
            (SyncKind::Problem, "problem: signed out"),
        ),
        (
            sync_status(&at(TeamState::ReadOnly { paused: true }, 0), true, 0, 0),
            (SyncKind::Problem, "problem: outside edits"),
        ),
        (
            sync_status(&at(TeamState::Idle, 0), false, 1, 0),
            (SyncKind::Problem, "problem: 1 clash to settle"),
        ),
        (
            sync_status(&at(TeamState::Idle, 0), false, 2, 0),
            (SyncKind::Problem, "problem: 2 clashes to settle"),
        ),
        (
            sync_status(&at(TeamState::Idle, 0), false, 0, 1),
            (SyncKind::Problem, "problem: 1 secret to move"),
        ),
        (
            sync_status(&at(TeamState::Idle, 0), false, 0, 2),
            (SyncKind::Problem, "problem: 2 secrets to move"),
        ),
        (
            sync_status(&at(TeamState::Idle, 0), false, 1, 2),
            (
                SyncKind::Problem,
                "problem: 1 clash to settle, 2 secrets to move",
            ),
        ),
    ];
    for ((kind, text), (want_kind, want_text)) in cases {
        assert_eq!((kind, text.as_str()), (want_kind, want_text));
    }
}

#[test]
fn toasts_name_the_collection_and_the_reason() {
    let secret = Block::Refused {
        findings: vec![Finding::new("secret_value", "r", "a.yml", "secret_value")],
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
        TeamState::Blocked(Block::Refused {
            findings: vec![file, bare],
            more: 0,
        }),
        0,
        vec![],
    );
    let lines = problem_lines(&status, 0, 0);
    assert_eq!(
        lines[1],
        "a.yml: secret_value on refs/\u{2060}heads/\u{2060}main"
    );
    assert_eq!(
        lines[2],
        "not in a file (a commit or tag message, or a tag's target): secret_value on refs/\u{2060}tags/\u{2060}v1"
    );
}

#[test]
fn a_finding_names_its_code_only_when_it_differs_from_the_rule() {
    let found = Finding::new("secret_pattern", "refs/heads/main", "notes.txt", "aws");
    assert_eq!(
        finding_line(&found),
        "notes.txt: aws (secret_pattern) on refs/\u{2060}heads/\u{2060}main"
    );
}

#[test]
fn an_idle_folder_with_clashes_says_they_wait_instead_of_in_sync() {
    let idle = TeamStatus::new(TeamState::Idle, 0, vec![]);
    assert_eq!(problem_lines(&idle, 1, 0), ["1 clash to settle."]);
    assert_eq!(problem_lines(&idle, 2, 0), ["2 clashes to settle."]);
}

#[test]
fn an_idle_folder_with_secrets_to_move_says_they_wait_instead_of_in_sync() {
    let idle = TeamStatus::new(TeamState::Idle, 0, vec![]);
    assert_eq!(problem_lines(&idle, 0, 1), ["1 secret to move."]);
    assert_eq!(problem_lines(&idle, 0, 2), ["2 secrets to move."]);
}

#[test]
fn an_idle_folder_with_clashes_and_secrets_to_move_names_both() {
    let idle = TeamStatus::new(TeamState::Idle, 0, vec![]);
    assert_eq!(
        problem_lines(&idle, 1, 2),
        ["1 clash to settle, 2 secrets to move."]
    );
}

fn refused(findings: Vec<Finding>, more: usize) -> Block {
    Block::Refused { findings, more }
}

#[test]
fn a_detail_shows_after_the_rule_beside_any_code() {
    let main = "refs/heads/main";
    let same = Finding::new(
        "secret_value",
        main,
        "environments/prod.yml",
        "secret_value",
    )
    .with_detail("token");
    let differs = Finding::new("secret_pattern", main, "notes.txt", "aws").with_detail("key");
    assert_eq!(
        finding_line(&same),
        "environments/prod.yml: secret_value (token) on refs/\u{2060}heads/\u{2060}main"
    );
    assert_eq!(
        finding_line(&differs),
        "notes.txt: aws (secret_pattern: key) on refs/\u{2060}heads/\u{2060}main"
    );
}

#[test]
fn a_mixed_refusal_lists_secrets_and_other_refusals_under_their_own_headings() {
    let main = "refs/heads/main";
    let block = refused(
        vec![
            Finding::new(
                "format_unsupported",
                main,
                "opencollection.yml",
                "workspace_format",
            )
            .with_detail("format missing"),
            Finding::new(
                "secret_value",
                main,
                "environments/prod.yml",
                "secret_value",
            )
            .with_detail("token"),
            Finding::new("future_code", main, "a.yml", "new_rule"),
        ],
        2,
    );
    let status = TeamStatus::new(TeamState::Blocked(block.clone()), 0, vec![]);
    let at = "on refs/\u{2060}heads/\u{2060}main";
    assert_eq!(
        problem_lines(&status, 0, 0),
        [
            "The server refused the push because it found secrets:".to_owned(),
            format!("environments/prod.yml: secret_value (token) {at}"),
            "The server refused the push:".to_owned(),
            format!(
                "opencollection.yml: workspace_format (format_unsupported: format missing) {at}"
            ),
            format!("a.yml: new_rule (future_code) {at}"),
            "and 2 more".to_owned(),
            "Fix them, save, then Retry.".to_owned(),
        ]
    );
    assert_eq!(
        sync_status(&status, false, 0, 0),
        (SyncKind::Problem, "problem: secrets found".into())
    );
    assert_eq!(
        problem_toast("api", &block),
        "Sync of api stopped: the server found secrets."
    );
}

#[test]
fn a_refusal_without_secrets_never_says_secrets_were_found() {
    let found = Finding::new("safety_relaxation", "refs/heads/main", "a.yml", "scripts");
    let block = refused(vec![found], 0);
    let status = TeamStatus::new(TeamState::Blocked(block.clone()), 0, vec![]);
    assert_eq!(
        problem_lines(&status, 0, 0),
        [
            "The server refused the push:",
            "a.yml: scripts (safety_relaxation) on refs/\u{2060}heads/\u{2060}main",
            "Fix them, save, then Retry.",
        ]
    );
    assert_eq!(
        sync_status(&status, false, 0, 0),
        (SyncKind::Problem, "problem: push refused".into())
    );
    assert_eq!(
        problem_toast("api", &block),
        "Sync of api stopped: the server refused the push."
    );
}
