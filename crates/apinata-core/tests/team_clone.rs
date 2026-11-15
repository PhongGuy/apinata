//! Team clones carry the token only in git's environment, and only to the server's own address.

use crate::common::potluck::*;
use crate::git_support::Sandbox;
#[cfg(unix)]
use crate::git_support::path_with;
use apinata_core::team::{self, ServerUrl, TeamApi, TeamError};
use apinata_core::{CancelFlag, CloneRequest, Collection, clone_repo};
use serde_json::json;
use std::fs;
use std::path::Path;
use wiremock::MockServer;

#[cfg(unix)]
#[test]
fn git_gets_the_token_only_through_its_environment() {
    let sb = Sandbox::new();
    let rec = sb.path().join("rec");
    fs::create_dir(&rec).unwrap();
    let body = "#!/bin/sh\n\
        env | grep -E '^GIT_(CONFIG|TRACE_REDACT)' | sort > \"$REC/env\"\n\
        printf '%s\\n' \"$@\" >> \"$REC/args\"\n\
        echo \"fatal: Authentication failed for 'http://127.0.0.1:9/git/core/api.git/'\" >&2\n\
        exit 128\n";
    let bin = sb.fake_git("bin", body);
    let server = ServerUrl::parse("http://127.0.0.1:9").unwrap();
    let base = sb.options().env("PATH", path_with(&bin)).env("REC", &rec);
    let opts = team_opts(base, &server);
    let dest = sb.path().join("dest");
    fs::create_dir(&dest).unwrap();
    let req = CloneRequest::new("http://127.0.0.1:9/git/core/api.git", &dest, "api");
    let err = clone_repo(&req, &CancelFlag::new(), &opts, &mut |_| {}).unwrap_err();
    let env = fs::read_to_string(rec.join("env")).unwrap();
    let header = format!("GIT_CONFIG_VALUE_0=Authorization: Bearer {}", tok());
    for line in [
        "GIT_CONFIG_COUNT=3",
        "GIT_CONFIG_KEY_0=http.http://127.0.0.1:9/.extraHeader",
        header.as_str(),
        "GIT_CONFIG_KEY_1=credential.helper",
        "GIT_CONFIG_VALUE_1=",
        "GIT_CONFIG_KEY_2=http.followRedirects",
        "GIT_CONFIG_VALUE_2=false",
        "GIT_TRACE_REDACT=1",
    ] {
        assert!(env.lines().any(|l| l == line), "{line} in {env}");
    }
    let args = fs::read_to_string(rec.join("args")).unwrap();
    assert!(!args.contains("plk_"), "{args}");
    assert!(!format!("{err} {err:?}").contains("plk_"));
}

#[test]
fn a_team_clone_leaves_no_token_in_the_repository_config() {
    let sb = Sandbox::new();
    let bare = sb.origin();
    let server = ServerUrl::parse("http://127.0.0.1:9").unwrap();
    let opts = team_opts(sb.options(), &server);
    let dest = sb.path().join("dest");
    fs::create_dir(&dest).unwrap();
    let req = CloneRequest::new(bare.to_str().unwrap(), &dest, "api");
    let root = clone_repo(&req, &CancelFlag::new(), &opts, &mut |_| {}).unwrap();
    let config = fs::read_to_string(root.join(".git/config")).unwrap();
    assert!(
        !config.contains("plk_") && !config.to_lowercase().contains("extraheader"),
        "{config}"
    );
    assert!(root.join("opencollection.yml").is_file());
}

#[tokio::test]
async fn the_clone_request_uses_the_workspace_git_address_and_slug() {
    let s = MockServer::start().await;
    answer(
        &s,
        "GET",
        "/api/v1/me",
        me(
            false,
            vec![workspace(&s.uri(), "core", "api", "apinata", "write")],
        ),
    )
    .await;
    let got = client(&s).me(&token()).await.unwrap();
    let parent = tempfile::tempdir().unwrap();
    let server = ServerUrl::parse(&s.uri()).unwrap();
    let req = team::clone_request(&server, &got.workspaces[0], parent.path()).unwrap();
    assert_eq!(req.url, format!("{}/git/core/api.git", s.uri()));
    assert_eq!(
        (req.name.as_str(), req.parent.as_path()),
        ("api", parent.path())
    );
}

#[tokio::test]
async fn a_git_address_on_another_host_is_refused() {
    let s = MockServer::start().await;
    let mut w = workspace(&s.uri(), "core", "api", "apinata", "write");
    w["git_url"] = json!("https://elsewhere.example/git/core/api.git");
    answer(&s, "GET", "/api/v1/me", me(false, vec![w])).await;
    let got = client(&s).me(&token()).await.unwrap();
    let server = ServerUrl::parse(&s.uri()).unwrap();
    let e = team::clone_request(&server, &got.workspaces[0], Path::new("/tmp")).unwrap_err();
    assert!(matches!(e, TeamError::Protocol(_)), "{e:?}");
}

#[test]
fn a_workspace_name_that_is_not_a_folder_name_is_refused() {
    let server = ServerUrl::parse("http://127.0.0.1:9").unwrap();
    for (t, slug) in [
        ("core", ".."),
        ("core", "a/b"),
        ("..", "api"),
        ("a/b", "api"),
        ("core", ""),
    ] {
        let w = workspace(server.as_str(), t, slug, "apinata", "write");
        let ws: team::Workspace = serde_json::from_value(w).unwrap();
        let e = team::clone_request(&server, &ws, Path::new("/tmp")).unwrap_err();
        assert!(matches!(e, TeamError::Protocol(_)), "{t}/{slug}: {e:?}");
    }
}

// Potluck's fixed seed for kind apinata, vendored from PhongGuy/potluck at the commit its NOTICE.md names.
const SEED: &str = "tests/fixtures/potluck/kinds/apinata/seed/opencollection.yml";

#[test]
#[ignore = "waits on Potluck's seed fixture commit: vendor kinds/apinata/seed/opencollection.yml with a NOTICE.md, then remove this ignore"]
fn the_writer_reproduces_the_potluck_seed_byte_for_byte() {
    let want = fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join(SEED)).expect("vendored seed");
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("ws");
    Collection::create(&root, "Workspace").unwrap();
    let got = fs::read(root.join("opencollection.yml")).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&got),
        String::from_utf8_lossy(&want)
    );
    assert_eq!(got, want);
}
