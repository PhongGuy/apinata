use crate::git_branch::cloned;
use crate::git_support;

use std::fs;
use std::path::Path;

use apinata_core::{
    CancelFlag, CloneError, CloneRequest, FetchError, RemoteBranch, SwitchError, clone_repo, fetch,
    remote_branches, switch_to_remote, validate_folder_name,
};
use git_support::{Sandbox, path_with};

#[test]
fn a_folder_name_with_a_leading_dash_is_refused() {
    for name in ["-x", "--upload-pack=x"] {
        assert!(
            matches!(
                validate_folder_name(name),
                Err(CloneError::InvalidName { .. })
            ),
            "{name}"
        );
    }
    assert!(validate_folder_name("a-b").is_ok());
}

#[test]
fn a_local_branch_with_a_leading_dash_never_starts_git() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    let bad = RemoteBranch::new("origin", "main", Some("--orphan".to_owned()));
    let r = switch_to_remote(&root, &bad, &sb.options());
    assert!(
        matches!(&r, Err(SwitchError::Failed { stderr }) if stderr.contains("branch name")),
        "{r:?}"
    );
}

#[test]
fn fetch_refuses_anything_but_a_configured_remote_name() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    let bare = sb.path().join("origin.git");
    let marker = sb.path().join("ran");
    for target in [
        bare.to_str().unwrap().to_owned(),
        format!("ext::touch {}", marker.display()),
        "nope".to_owned(),
    ] {
        let r = fetch(&root, &target, &CancelFlag::new(), &sb.options());
        assert!(
            matches!(&r, Err(FetchError::Failed { stderr }) if stderr.contains("remote")),
            "{target}: {r:?}"
        );
    }
    assert!(!marker.exists());
    fetch(&root, "origin", &CancelFlag::new(), &sb.options()).expect("a configured name works");
}

/// Every call carries `protocol.ext.allow=never`, even when the user's config allows ext.
#[cfg(unix)]
#[test]
fn a_configured_ext_remote_cannot_run_a_command() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    let marker = sb.path().join("ran");
    sb.git(
        &root,
        &[
            "remote",
            "add",
            "evil",
            &format!("ext::touch {}", marker.display()),
        ],
    );
    let allow = sb.path().join("allow.gitconfig");
    fs::write(&allow, "[protocol \"ext\"]\n\tallow = always\n").unwrap();
    let opts = sb.options().env("GIT_CONFIG_GLOBAL", &allow);
    let r = fetch(&root, "evil", &CancelFlag::new(), &opts);
    assert!(r.is_err(), "{r:?}");
    assert!(!marker.exists(), "the ext command ran");
}

/// A recorder in place of git that logs each argv on one line and succeeds.
#[cfg(unix)]
const RECORD: &str = "#!/bin/sh\ndir=${0%/*}\necho \"$*\" >> \"$dir/argv\"\n";

#[cfg(unix)]
#[test]
fn every_call_disables_the_ext_transport() {
    let sb = Sandbox::new();
    let bin = sb.fake_git("rec", RECORD);
    let root = sb.path().join("col");
    fs::create_dir_all(&root).unwrap();
    let opts = sb.options().env("PATH", path_with(&bin));
    let _ = apinata_core::remotes(&root, &opts);
    let parent = sb.path().join("dest");
    fs::create_dir_all(&parent).unwrap();
    let _ = clone_repo(
        &CloneRequest::new("/srv/x.git", &parent, "col"),
        &CancelFlag::new(),
        &opts,
        &mut |_| {},
    );
    let argv = fs::read_to_string(bin.join("argv")).unwrap();
    assert!(argv.lines().count() >= 2, "{argv}");
    for line in argv.lines() {
        assert!(line.contains("-c protocol.ext.allow=never"), "{line}");
    }
}

#[test]
fn a_remote_name_with_a_slash_splits_at_the_remote_not_the_first_slash() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    let bare = sb.path().join("origin.git");
    sb.git(
        &root,
        &["remote", "add", "team/fork", bare.to_str().unwrap()],
    );
    sb.git(&root, &["fetch", "-q", "team/fork"]);
    let list = remote_branches(&root, &sb.options()).unwrap();
    let forks: Vec<(&str, &str)> = list
        .iter()
        .filter(|b| b.remote.contains('/'))
        .map(|b| (b.remote.as_str(), b.name.as_str()))
        .collect();
    assert_eq!(forks, [("team/fork", "feature"), ("team/fork", "main")]);
    assert!(
        list.iter()
            .all(|b| b.remote == "origin" || b.remote == "team/fork")
    );
}

#[cfg(unix)]
#[test]
fn an_unreadable_destination_is_an_io_error_not_exists() {
    use std::os::unix::fs::PermissionsExt;
    let sb = Sandbox::new();
    let parent = sb.path().join("dest");
    let dest = parent.join("col");
    fs::create_dir_all(&dest).unwrap();
    let _g = crate::common::ModeGuard::set(&dest, 0o000);
    // Root reads anything, so the case cannot be set up there.
    if fs::read_dir(&dest).is_ok() {
        fs::set_permissions(&dest, fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }
    let r = clone_repo(
        &CloneRequest::new("/srv/x.git", &parent, "col"),
        &CancelFlag::new(),
        &sb.options(),
        &mut |_| {},
    );
    fs::set_permissions(&dest, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        matches!(&r, Err(CloneError::Io(e)) if e.kind() == std::io::ErrorKind::PermissionDenied),
        "{r:?}"
    );
    let _: &Path = &dest;
}
