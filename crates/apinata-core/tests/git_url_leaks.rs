//! No credential in a git URL reaches an error, a progress line or a redacted display string.

use crate::git_branch::cloned;
use crate::git_support;

use std::fs;
use std::path::{Path, PathBuf};

use apinata_core::{
    CancelFlag, CloneError, CloneRequest, FetchError, GitOptions, RemoteBranch, SwitchError,
    clone_repo, fetch, redact_url, switch_to_remote, validate_clone_url,
};
use git_support::{Sandbox, path_with, real_git};

const PW: &str = "s3cr3t";

/// The userinfo rule: userinfo is what precedes the last '@' of the whitespace-free token after
/// "://", provided no '/', '?' or '#' precedes its first ':' and it does not start with '['. All of it
/// is dropped for every scheme. The scp form `user:pw@host:path` loses its userinfo too; a bare
/// `user@host:path` holds no secret and stays.
#[test]
fn redact_url_table() {
    for (url, want) in [
        ("https://user:s3cr3t@host/a/b.git", "https://host/a/b.git"),
        ("https://user:s3cr3t@host/a.git/", "https://host/a.git/"),
        ("https://user:s3cr3t@host:8443/a", "https://host:8443/a"),
        ("https://user:s3/cr3t@host/a.git", "https://host/a.git"),
        ("https://user:s3?cr3t@host/a", "https://host/a"),
        ("https://user:s3#cr3t@host/a", "https://host/a"),
        ("https://user:s3/c?r#3t@host/a", "https://host/a"),
        ("https://user:p@ss:w/rd@host/a", "https://host/a"),
        ("https://user:s%2Fcr3t@host/a", "https://host/a"),
        ("https://token@host/a", "https://host/a"),
        ("https://u:123/x@host/y", "https://host/y"),
        ("https://u:123?x@host/y", "https://host/y"),
        // Fail closed: a port then a path holding '@' reads as userinfo.
        ("https://h:8080/a?e=a@b", "https://b"),
        ("ssh://git:s3cr3t@host/x", "ssh://host/x"),
        ("ssh://git:s3/cr3t@host/x", "ssh://host/x"),
        (
            "file://user:s3cr3t@127.0.0.1:1/x.git",
            "file://127.0.0.1:1/x.git",
        ),
        ("HTTPS://user:s3cr3t@host/a", "HTTPS://host/a"),
        ("user:s3cr3t@host:repo.git", "host:repo.git"),
        ("user:s3cr3t@host:team/repo.git/", "host:team/repo.git/"),
        ("user:s3/cr3t@host:repo.git", "host:repo.git"),
        // Nothing secret here, so nothing changes.
        (
            "https://host/a?email=me@x.com",
            "https://host/a?email=me@x.com",
        ),
        ("https://host/a#frag@x", "https://host/a#frag@x"),
        ("https://host/a@b:c", "https://host/a@b:c"),
        ("https://[::1]:80/a@b", "https://[::1]:80/a@b"),
        (
            "https://host/a?next=https://x/y",
            "https://host/a?next=https://x/y",
        ),
        ("git@host:repo", "git@host:repo"),
        ("git@host:team/repo.git", "git@host:team/repo.git"),
        ("/tmp/x/origin.git", "/tmp/x/origin.git"),
        ("https://host/a", "https://host/a"),
    ] {
        assert_eq!(redact_url(url), want, "{url}");
    }
}

/// Git prints the URL inside its sentences, quoted or not, and sometimes in a variant.
#[test]
fn redact_url_scrubs_urls_inside_git_text() {
    for (text, want) in [
        (
            "fatal: unable to access 'https://user:s3cr3t@host/r.git/': nope\n",
            "fatal: unable to access 'https://host/r.git/': nope\n",
        ),
        (
            "fatal: repository \"https://user:s3/cr3t@host/r\" not found",
            "fatal: repository \"https://host/r\" not found",
        ),
        (
            "fatal: could not read from user:s3cr3t@host:repo.git (x)",
            "fatal: could not read from host:repo.git (x)",
        ),
        (
            "see https://user:s3cr3t@a/x and ssh://u:s3cr3t@b/y",
            "see https://a/x and ssh://b/y",
        ),
    ] {
        assert_eq!(redact_url(text), want);
    }
}

#[test]
fn invalid_url_reasons_never_echo_the_url() {
    for url in [
        format!("ftp://user:{PW}@host/x"),
        format!("ext::sh -c {PW}"),
        format!("user:{PW}@:path"),
        format!("-u:{PW}@host/x"),
        format!("https://user:{PW}@/x"),
        format!("user:{PW}"),
        format!("https://user:{PW}@-host/x"),
        format!("user:{PW}@-host:x"),
    ] {
        let err = validate_clone_url(&url).unwrap_err();
        assert!(matches!(err, CloneError::InvalidUrl { .. }), "{url}");
        assert!(!err.to_string().contains(PW), "{url}: {err}");
    }
}

fn fake_git_printing(sb: &Sandbox, real: &Path, words: &str, on: &str) -> PathBuf {
    let body = format!(
        "#!/bin/sh\ncase \"$*\" in *\" config \"*) exit 1 ;; esac\ncase \"$*\" in *{on}*) ;; *) exec {real} \"$@\" ;; esac\nfor last; do :; done\nmkdir -p \"$last\" 2>/dev/null\nprintf '%s\\n' \"{words}\" >&2\nprintf '%s\\n' \"{words}\"\nexit 128\n",
        real = real.display(),
    );
    sb.fake_git("fake", &body)
}

#[cfg(unix)]
#[test]
fn a_failed_clone_never_carries_the_password_in_any_url_form() {
    let sb = Sandbox::new();
    let parent = sb.path().join("dest");
    fs::create_dir_all(&parent).unwrap();
    for url in [
        format!("https://user:{PW}@127.0.0.1:1/x.git/"),
        format!("https://user:{PW}/a?b#c@127.0.0.1:1/x.git"),
        format!("ssh://user:{PW}@127.0.0.1:1/x.git"),
        format!("file://user:{PW}@127.0.0.1:1/x.git"),
    ] {
        // The fake git echoes the URL in the variants git prints, plus a decoded one.
        let echoed = format!(
            "fatal: unable to access '{url}/': x; again {url}.git and {}",
            url.replace("127.0.0.1:1/x.git", "127.0.0.1:1/x.git/")
        );
        let bin = fake_git_printing(&sb, &real_git(), &echoed, "clone");
        let opts = sb.options().env("PATH", path_with(&bin));
        let mut lines = Vec::new();
        let r = clone_repo(
            &CloneRequest::new(&url, &parent, "col"),
            &CancelFlag::new(),
            &opts,
            &mut |p| lines.push(p.phase),
        );
        let err = r.unwrap_err();
        let CloneError::Failed { stderr } = err else {
            panic!("{url}: {err:?}");
        };
        assert!(!stderr.contains(PW), "{url}: {stderr}");
        assert!(lines.iter().all(|l| !l.contains(PW)), "{url}: {lines:?}");
        assert!(!lines.is_empty(), "{url}");
    }
}

#[test]
fn a_real_clone_of_a_dead_url_never_shows_the_password() {
    let sb = Sandbox::new();
    let parent = sb.path().join("dest");
    fs::create_dir_all(&parent).unwrap();
    for url in [
        format!("https://user:{PW}@127.0.0.1:1/x.git/"),
        format!("file://user:{PW}@127.0.0.1:1/x.git"),
        format!("user:{PW}@127.0.0.1:x.git"),
    ] {
        let mut lines = Vec::new();
        let r = clone_repo(
            &CloneRequest::new(&url, &parent, "col"),
            &CancelFlag::new(),
            &sb.options(),
            &mut |p| lines.push(p.phase),
        );
        let text = format!("{r:?} {lines:?}");
        assert!(!text.contains(PW), "{url}: {text}");
    }
}

#[cfg(unix)]
#[test]
fn a_failed_fetch_never_carries_the_password() {
    let sb = Sandbox::new();
    let words = format!(
        "fatal: unable to access 'https://user:{PW}@h/r.git/': x user:{PW}@h:r ssh://u:a/{PW}@h/y"
    );
    let bin = fake_git_printing(&sb, &real_git(), &words, "fetch");
    let opts = sb.options().env("PATH", path_with(&bin));
    let r = fetch(sb.path(), "origin", &CancelFlag::new(), &opts);
    let Err(FetchError::Failed { stderr }) = r else {
        panic!("{r:?}");
    };
    assert!(stderr.contains("fatal"), "{stderr}");
    assert!(!stderr.contains(PW), "{stderr}");
}

#[cfg(unix)]
#[test]
fn a_failed_switch_never_carries_the_password() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    let words = format!("fatal: bad 'https://user:{PW}@h/r.git/' and user:{PW}@h:r");
    let bin = fake_git_printing(&sb, &real_git(), &words, "switch");
    let opts: GitOptions = sb.options().env("PATH", path_with(&bin));
    let branch = RemoteBranch::new("origin", "feature", None);
    let r = switch_to_remote(&root, &branch, &opts);
    let Err(SwitchError::Failed { stderr }) = r else {
        panic!("{r:?}");
    };
    assert!(stderr.contains("fatal"), "{stderr}");
    assert!(!stderr.contains(PW), "{stderr}");
}
