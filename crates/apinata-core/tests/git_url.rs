use apinata_core::{
    CloneError, CloneUrlKind, RemoteHost, default_folder_name, proxy_host, redact_url, remote_host,
    validate_clone_url, validate_folder_name,
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

#[test]
fn a_network_remote_names_the_host_git_would_contact() {
    for (url, host) in [
        ("https://example.invalid/r.git", "example.invalid"),
        (
            "HTTPS://User:pw@Example.INVALID:8443/r.git",
            "example.invalid",
        ),
        ("http://example.invalid/r", "example.invalid"),
        ("ssh://git@example.invalid:2222/r.git", "example.invalid"),
        ("git+ssh://example.invalid/r", "example.invalid"),
        ("git://example.invalid/r", "example.invalid"),
        ("git@example.invalid:r.git", "example.invalid"),
        ("example.invalid:team/r.git", "example.invalid"),
        ("git@[2001:db8::1]:r.git", "[2001:db8::1]"),
        ("ssh://[2001:db8::1]:22/r", "[2001:db8::1]"),
        ("[git@example.invalid:22]:r.git", "example.invalid"),
        ("ssh://a@localhost@evil.invalid/r", "evil.invalid"),
        ("a@localhost@evil.invalid:r", "evil.invalid"),
        ("https://127.0.0.1@evil.invalid/r", "evil.invalid"),
    ] {
        assert_eq!(remote_host(url), RemoteHost::Host(host.to_owned()), "{url}");
    }
}

#[test]
fn a_path_file_url_or_loopback_host_is_local() {
    for url in [
        "/srv/git/r.git",
        "../r.git",
        "r.git",
        "./a:b",
        "file:///srv/git/r.git",
        "file://localhost/srv/git/r.git",
        "FILE:///srv/r",
        "ssh://git@localhost/r.git",
        "git@127.0.0.1:r.git",
        "https://[::1]/r.git",
        "https://api.localhost/r.git",
    ] {
        assert_eq!(remote_host(url), RemoteHost::Local, "{url}");
    }
}

#[test]
fn a_transport_whose_destination_cannot_be_read_is_unknown() {
    for url in [
        "",
        "ext::ssh -o ProxyCommand=x host",
        "fd::3",
        "persistent-https::example.invalid/r",
        "gcrypt://example.invalid/r",
        "https:///nohost",
        "git@:r.git",
        // ssh reads a `?` or `#` as part of the host text, not as the end of it.
        "ssh://127.0.0.1?@evil.invalid/repo",
        "ssh://localhost#@evil2.invalid/repo",
        "git://localhost?x/repo",
        "localhost?x@evil.invalid:repo",
        "localhost#@evil.invalid:repo",
        // A network share or a file URL naming a host is never read as local.
        "\\\\evil.invalid\\share\\r.git",
        "//evil.invalid/share/r.git",
        "file://evil.invalid/share/r.git",
        "file:////evil.invalid/share/r.git",
        "file://///evil.invalid/share/r.git",
        "file:///\\\\evil.invalid\\share",
        "file://localhost//evil.invalid/share",
    ] {
        assert_eq!(remote_host(url), RemoteHost::Unknown, "{url:?}");
    }
}

#[test]
fn a_proxy_names_its_host_or_is_local() {
    let host = |h: &str| RemoteHost::Host(h.to_owned());
    for (proxy, want) in [
        ("http://proxy.invalid:3128", host("proxy.invalid")),
        ("proxy.invalid:3128", host("proxy.invalid")),
        ("http://u:pw@Proxy.Invalid/", host("proxy.invalid")),
        ("socks5h://u:p@127.0.0.1:1080", RemoteHost::Local),
        ("localhost:8080", RemoteHost::Local),
        ("", RemoteHost::Unknown),
        ("http://", RemoteHost::Unknown),
    ] {
        assert_eq!(proxy_host(proxy), want, "{proxy}");
    }
}
