//! The clone dialog's rules: where a clone goes, what the form accepts, what the user is told.

use std::fs;
use std::path::{Path, PathBuf};

use apinata_app::clone_form::{
    FormError, Opened, auth_hint, check, classify, clone_error_text, default_parent, ensure_parent,
    is_unencrypted, no_collection_text, shown_url, suggested_name,
};
use apinata_app::persist::{load_json, save_json};
use apinata_app::settings::Settings;
use apinata_core::CloneError;

#[test]
fn the_saved_parent_wins_over_the_home_folder() {
    let home = Path::new("/home/me");
    assert_eq!(
        default_parent(Some(Path::new("/work")), Some(home)),
        Some(PathBuf::from("/work"))
    );
    assert_eq!(
        default_parent(None, Some(home)),
        Some(PathBuf::from("/home/me/Apinata"))
    );
    assert_eq!(default_parent(None, None), None);
}

#[test]
fn ensure_parent_creates_the_first_use_folder_and_accepts_an_existing_one() {
    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path().join("Apinata");
    ensure_parent(&parent).unwrap();
    assert!(parent.is_dir());
    ensure_parent(&parent).unwrap();
}

#[test]
fn the_clone_parent_setting_defaults_to_none_and_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    fs::write(&path, r#"{"insecure": true}"#).unwrap();
    assert_eq!(load_json::<Settings>(&path).value.clone_parent, None);
    let saved = Settings {
        clone_parent: Some(PathBuf::from("/work")),
        ..Settings::default()
    };
    save_json(&path, &saved).unwrap();
    assert_eq!(
        load_json::<Settings>(&path).value.clone_parent,
        Some(PathBuf::from("/work"))
    );
}

#[test]
fn the_folder_name_follows_the_url() {
    assert_eq!(suggested_name("https://github.com/acme/api.git"), "api");
    assert_eq!(suggested_name("git@github.com:acme/api"), "api");
    assert_eq!(suggested_name("/srv/git/api.git/"), "api");
    assert_eq!(suggested_name(""), "");
    assert_eq!(suggested_name("https://example.com"), "");
}

#[test]
fn a_valid_form_becomes_a_request() {
    let dir = tempfile::tempdir().unwrap();
    let req = check(" https://example.com/a/b.git ", Some(dir.path()), "b").unwrap();
    assert_eq!(req.url, "https://example.com/a/b.git");
    assert_eq!(req.parent, dir.path());
    assert_eq!(req.name, "b");
}

#[test]
fn the_form_refuses_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let p = Some(dir.path());
    assert_eq!(check("  ", p, "x").unwrap_err(), FormError::EmptyUrl);
    assert!(matches!(
        check("-oops", p, "x").unwrap_err(),
        FormError::BadUrl(_)
    ));
    assert!(matches!(
        check("ext::sh -c id", p, "x").unwrap_err(),
        FormError::BadUrl(_)
    ));
    assert_eq!(
        check("https://example.com/a.git", None, "a").unwrap_err(),
        FormError::NoParent
    );
    for name in ["", ".", "..", "a/b"] {
        let e = check("https://example.com/a.git", p, name).unwrap_err();
        assert!(matches!(e, FormError::BadName(_)), "{name:?}: {e:?}");
    }
}

#[test]
fn only_a_non_empty_or_non_directory_destination_exists() {
    let dir = tempfile::tempdir().unwrap();
    let p = Some(dir.path());
    let url = "https://example.com/a.git";
    fs::create_dir(dir.path().join("empty")).unwrap();
    assert!(check(url, p, "empty").is_ok());
    assert!(check(url, p, "missing").is_ok());
    fs::create_dir(dir.path().join("full")).unwrap();
    fs::write(dir.path().join("full/keep.txt"), "mine").unwrap();
    assert_eq!(check(url, p, "full").unwrap_err(), FormError::Exists);
    fs::write(dir.path().join("file"), "x").unwrap();
    assert_eq!(check(url, p, "file").unwrap_err(), FormError::Exists);
    assert!(check(url, Some(&dir.path().join("not-yet")), "a").is_ok());
}

#[test]
fn urls_are_shown_without_credentials_and_http_is_flagged() {
    assert_eq!(
        shown_url("https://me:s3cret@example.com/a.git"),
        "https://example.com/a.git"
    );
    assert!(is_unencrypted("HTTP://example.com/a.git"));
    assert!(!is_unencrypted("https://example.com/a.git"));
    assert!(!is_unencrypted("ssh://git@example.com/a.git"));
}

#[test]
fn failures_read_as_git_said_plus_one_hint() {
    let failed = |s: &str| CloneError::Failed {
        stderr: s.to_owned(),
    };
    let auth = clone_error_text(&failed("fatal: Authentication failed for 'x'")).unwrap();
    assert!(auth.starts_with("fatal: Authentication failed"), "{auth}");
    assert!(auth.contains("credential helper or ssh-agent"), "{auth}");
    let host = clone_error_text(&failed("Host key verification failed.")).unwrap();
    assert!(host.contains("accept the host key"), "{host}");
    assert_eq!(
        clone_error_text(&failed("fatal: repository not found")).unwrap(),
        "fatal: repository not found"
    );
    assert_eq!(auth_hint("all fine"), None);
    assert_eq!(clone_error_text(&CloneError::Cancelled), None);
    assert!(
        clone_error_text(&CloneError::GitMissing)
            .unwrap()
            .contains("Git was not found")
    );
}

#[test]
fn the_result_opens_only_when_the_root_holds_a_collection() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        classify(dir.path()),
        Opened::NoCollection(dir.path().to_path_buf())
    );
    fs::write(dir.path().join("opencollection.yml"), "x").unwrap();
    assert_eq!(
        classify(dir.path()),
        Opened::Collection(dir.path().to_path_buf())
    );
    let text = no_collection_text(dir.path());
    assert!(text.contains(&dir.path().display().to_string()) && text.contains("No collection"));
    for t in [
        text,
        FormError::Exists.message(),
        FormError::EmptyUrl.message(),
    ] {
        assert!(!t.to_lowercase().contains("bruno"));
    }
}

#[test]
fn a_failure_with_no_output_still_says_something() {
    let text = clone_error_text(&CloneError::Failed {
        stderr: "  \n".to_owned(),
    })
    .unwrap();
    assert!(!text.trim().is_empty());
}
