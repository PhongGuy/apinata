use apinata_core::{
    CloneError, CloneUrlKind, default_folder_name, redact_url, validate_clone_url,
    validate_folder_name,
};

#[test]
fn each_documented_form_is_accepted_with_its_kind() {
    use CloneUrlKind::*;
    for (url, kind) in [
        ("https://example.com/a/b.git", Https),
        ("HTTPS://example.com/a/b.git", Https),
        ("http://example.com/a/b", Http),
        ("ssh://git@example.com/a/b.git", Ssh),
        ("git@example.com:a/b.git", Ssh),
        ("file:///tmp/x/origin.git", File),
        ("/tmp/x/origin.git", Path),
    ] {
        assert_eq!(validate_clone_url(url).unwrap(), kind, "{url}");
    }
}

#[test]
fn everything_else_is_rejected_before_git_runs() {
    for url in [
        "",
        "   ",
        "-oProxyCommand=x",
        "--upload-pack=x",
        "ext::sh -c id",
        "fd::3",
        "ftp://h/x",
        "git://h/x",
        "ssh://-oProxyCommand=x/y",
        "ssh://git@-host/y",
        "git@-host:x",
        "relative/path",
        "https://h/x y",
        "https://h/x\n",
        "https://",
        "host:path",
        "https:///nohost",
    ] {
        assert!(
            matches!(validate_clone_url(url), Err(CloneError::InvalidUrl { .. })),
            "{url:?}"
        );
    }
}

#[test]
fn a_folder_name_is_one_plain_path_component() {
    for ok in ["repo", "my repo", "a.b", ".hidden"] {
        validate_folder_name(ok).unwrap();
    }
    for bad in ["", ".", "..", "a/b", "a\\b", "a\0b"] {
        assert!(
            matches!(
                validate_folder_name(bad),
                Err(CloneError::InvalidName { .. })
            ),
            "{bad:?}"
        );
    }
}

#[test]
fn the_default_folder_name_is_the_last_segment_without_dot_git() {
    for (url, want) in [
        ("https://github.com/a/b.git", Some("b")),
        ("https://h/a/b/", Some("b")),
        ("git@host:team/repo.git", Some("repo")),
        ("git@host:repo", Some("repo")),
        ("/tmp/x/origin.git", Some("origin")),
        ("file:///tmp/x/origin.git", Some("origin")),
        ("https://h/", None),
        ("https://h/a/.git", None),
    ] {
        assert_eq!(default_folder_name(url).as_deref(), want, "{url}");
    }
}

#[test]
fn redaction_drops_the_whole_userinfo_of_every_scheme() {
    for (url, want) in [
        (
            "https://user:secret@example.com/a/b.git",
            "https://example.com/a/b.git",
        ),
        ("https://token@example.com/a", "https://example.com/a"),
        ("ssh://git:pw@example.com/x", "ssh://example.com/x"),
        ("ssh://git@example.com/x", "ssh://example.com/x"),
        ("git@example.com:a/b.git", "git@example.com:a/b.git"),
        ("/tmp/x/origin.git", "/tmp/x/origin.git"),
    ] {
        assert_eq!(redact_url(url), want);
    }
}
