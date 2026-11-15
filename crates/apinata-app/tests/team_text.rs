//! What the team dialogs say for each failure.

use apinata_app::team_text::{error_text, server_warning, sign_in_error_text};
use apinata_core::team::{ServerUrl, TeamError};

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
