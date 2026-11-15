//! A signed-out folder keeps committing, and signing in again from its dialog resumes and pushes.

use crate::git_support::Sandbox;
use crate::support::potluck::{error, info, session, token_meta, user, workspace};
use crate::support::team::*;
use crate::support::*;
use apinata_core::SecretStore;
use apinata_core::team::{Block, Role, TeamState, TeamStore, token_key};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

fn state(cx: &mut TestAppContext, s: &Started) -> Option<TeamState> {
    s.ws.read_with(cx, |w, _| w.team_status(&s.root).map(|t| t.state))
}

#[gpui_kit::test]
fn a_signed_out_folder_commits_locally_and_pushes_after_signing_in_again(cx: &mut TestAppContext) {
    http_ready(cx);
    let (sb, col) = (Sandbox::new(), collection());
    let bare = shared_origin(&sb, col.path());
    let server = potluck_server(cx);
    let refused =
        ResponseTemplate::new(401).set_body_json(error("unauthenticated", "Revoked.", None));
    mount(
        cx,
        &server,
        Mock::given(method("GET"))
            .and(path("/api/v1/me"))
            .respond_with(refused)
            .up_to_n_times(1),
    );
    serve_workspace(cx, &server, "write");
    mount(
        cx,
        &server,
        Mock::given(method("GET"))
            .and(path("/api/v1/server"))
            .respond_with(info(true)),
    );
    mount(
        cx,
        &server,
        Mock::given(method("POST"))
            .and(path("/api/v1/session"))
            .respond_with(session("tok-8", false)),
    );
    let root = col.path().to_path_buf();
    let (s, secrets) = team_started(cx, &sb, &col, |paths, secrets| {
        signed_in(paths, secrets, &server, false);
        bind(paths, &root, &server.uri(), Role::Write);
    });
    wait_for(cx, "signed out", |cx| {
        state(cx, &s) == Some(TeamState::Blocked(Block::SignedOut))
    });
    std::fs::write(col.path().join("b.yml"), request("b", "http://x/b")).unwrap();
    wait_for(cx, "a local commit", |cx| {
        s.ws.read_with(cx, |w, _| {
            w.team_status(&s.root).is_some_and(|t| t.unpushed == 1)
        })
    });
    assert_eq!(state(cx, &s), Some(TeamState::Blocked(Block::SignedOut)));
    let (ws, at) = (s.ws.clone(), s.root.clone());
    cx.update_window(s.win, |_, w, cx| {
        apinata_app::team_sync_dialog::open(&ws, at, w, cx)
    })
    .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("team-sign-in-again", cx))
        .unwrap();
    frame(cx, s.win);
    let join = cx
        .update(|cx| apinata_app::team_join::current(cx))
        .expect("join dialog");
    let (url, user) = join.read_with(cx, |j, cx| (j.server(cx), j.username(cx)));
    assert_eq!((url, user.as_str()), (server.uri(), "ada"));
    cx.update_window(s.win, |_, w, cx| {
        join.update(cx, |j, cx| {
            j.set_password("hunter2", w, cx);
            j.submit(w, cx);
        })
    })
    .unwrap();
    wait_for(cx, "the push", |_| origin_has(&sb, &bare, "b.yml", "x/b"));
    wait_for(cx, "in sync", |cx| state(cx, &s) == Some(TeamState::Idle));
    let key = token_key(&server_url(&server), "ada");
    assert!(secrets.get(&key).unwrap().is_some());
}

#[gpui_kit::test]
fn a_sign_in_during_a_resume_resumes_again_once_it_lands(cx: &mut TestAppContext) {
    http_ready(cx);
    let (sb, col) = (Sandbox::new(), collection());
    let bare = shared_origin(&sb, col.path());
    let server = potluck_server(cx);
    // The launch resume's `/me` answers signed out only after the sign-in below has finished.
    let refused = ResponseTemplate::new(401)
        .set_body_json(error("unauthenticated", "Revoked.", None))
        .set_delay(std::time::Duration::from_secs(2));
    mount(
        cx,
        &server,
        Mock::given(method("GET"))
            .and(path("/api/v1/me"))
            .respond_with(refused)
            .up_to_n_times(1),
    );
    serve_workspace(cx, &server, "write");
    mount(
        cx,
        &server,
        Mock::given(method("GET"))
            .and(path("/api/v1/server"))
            .respond_with(info(true)),
    );
    mount(
        cx,
        &server,
        Mock::given(method("POST"))
            .and(path("/api/v1/session"))
            .respond_with(session("tok-8", false)),
    );
    let root = col.path().to_path_buf();
    let s = team_started(cx, &sb, &col, |paths, secrets| {
        signed_in(paths, secrets, &server, false);
        bind(paths, &root, &server.uri(), Role::Write);
    })
    .0;
    wait_for(cx, "the resume's /me", |cx| {
        hits_on(cx, &server, "GET", "/api/v1/me") >= 1
    });
    assert_eq!(state(cx, &s), None, "the resume is still in flight");
    let ws = s.ws.clone();
    let join = cx
        .update_window(s.win, |_, w, cx| {
            apinata_app::team_join::open_for(&ws, &server.uri(), "ada", w, cx)
        })
        .unwrap();
    cx.update_window(s.win, |_, w, cx| {
        join.update(cx, |j, cx| {
            j.set_password("hunter2", w, cx);
            j.submit(w, cx);
        })
    })
    .unwrap();
    let (file, url) = (team_file(&s), server_url(&server));
    wait_for(cx, "the new session", |_| {
        TeamStore::load(&file)
            .unwrap()
            .account(&url)
            .is_some_and(|a| a.token_id == "tok-8")
    });
    assert_eq!(state(cx, &s), None, "the sign-in landed before the resume");
    wait_for(cx, "in sync", |cx| state(cx, &s) == Some(TeamState::Idle));
    std::fs::write(col.path().join("b.yml"), request("b", "http://x/b")).unwrap();
    wait_for(cx, "the push", |_| origin_has(&sb, &bare, "b.yml", "x/b"));
}

#[gpui_kit::test]
fn a_sign_in_leaves_a_healthy_loop_and_its_unsettled_clash_alone(cx: &mut TestAppContext) {
    http_ready(cx);
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let server = potluck_server(cx);
    let fresh = format!("plk_{}", "B".repeat(43));
    serve_workspace(cx, &server, "write");
    mount(
        cx,
        &server,
        Mock::given(method("GET"))
            .and(path("/api/v1/server"))
            .respond_with(info(true)),
    );
    mount(
        cx,
        &server,
        Mock::given(method("POST"))
            .and(path("/api/v1/session"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "token": fresh, "token_meta": token_meta("tok-8"), "user": user("ada", false)
            }))),
    );
    let root = col.path().to_path_buf();
    let s = team_started(cx, &sb, &col, |paths, secrets| {
        signed_in(paths, secrets, &server, false);
        bind(paths, &root, &server.uri(), Role::Write);
    })
    .0;
    wait_for(cx, "in sync", |cx| state(cx, &s) == Some(TeamState::Idle));
    let mate = sb.path().join("mate");
    crate::git_support::write(&mate, "a.yml", &request("a", "http://x/theirs"));
    sb.commit_all(&mate, "theirs");
    sb.git(&mate, &["push", "-q"]);
    std::fs::remove_file(col.path().join("a.yml")).unwrap();
    let clashes = |cx: &mut TestAppContext| s.ws.read_with(cx, |w, _| w.team_clashes(&s.root));
    wait_for(cx, "the clash", |cx| clashes(cx).len() == 1);
    wait_for(cx, "in sync", |cx| state(cx, &s) == Some(TeamState::Idle));
    let before = s.ws.read_with(cx, |w, _| w.team_sync(&s.root)).unwrap();
    let ws = s.ws.clone();
    let join = cx
        .update_window(s.win, |_, w, cx| {
            apinata_app::team_join::open_for(&ws, &server.uri(), "ada", w, cx)
        })
        .unwrap();
    cx.update_window(s.win, |_, w, cx| {
        join.update(cx, |j, cx| {
            j.set_password("hunter2", w, cx);
            j.submit(w, cx);
        })
    })
    .unwrap();
    let (file, url) = (team_file(&s), server_url(&server));
    wait_for(cx, "the new session", |_| {
        TeamStore::load(&file)
            .unwrap()
            .account(&url)
            .is_some_and(|a| a.token_id == "tok-8")
    });
    let bearer = format!("Bearer {fresh}");
    wait_for(cx, "the events stream on the new token", |cx| {
        let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
        let seen = handle.block_on(server.received_requests()).unwrap();
        seen.iter().any(|r| {
            r.url.path() == "/api/v1/workspaces/core/api/events"
                && r.headers
                    .get("authorization")
                    .is_some_and(|v| v == bearer.as_str())
        })
    });
    settle(cx);
    let after = s.ws.read_with(cx, |w, _| w.team_sync(&s.root)).unwrap();
    assert!(
        std::sync::Arc::ptr_eq(&before, &after),
        "the loop kept running"
    );
    assert_eq!(clashes(cx).len(), 1, "the clash is still unsettled");
}

#[gpui_kit::test]
fn a_sign_in_starts_over_a_loop_the_server_had_dropped(cx: &mut TestAppContext) {
    http_ready(cx);
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let server = potluck_server(cx);
    serve_workspace(cx, &server, "write");
    mount(
        cx,
        &server,
        Mock::given(method("GET"))
            .and(path("/api/v1/server"))
            .respond_with(info(true)),
    );
    mount(
        cx,
        &server,
        Mock::given(method("POST"))
            .and(path("/api/v1/session"))
            .respond_with(session("tok-8", false)),
    );
    let root = col.path().to_path_buf();
    let s = team_started(cx, &sb, &col, |paths, secrets| {
        signed_in(paths, secrets, &server, false);
        bind(paths, &root, &server.uri(), Role::Write);
    })
    .0;
    wait_for(cx, "in sync", |cx| state(cx, &s) == Some(TeamState::Idle));
    let before = s.ws.read_with(cx, |w, _| w.team_sync(&s.root)).unwrap();
    before.gone();
    wait_for(cx, "gone", |cx| {
        state(cx, &s) == Some(TeamState::Blocked(Block::Gone))
    });
    let ws = s.ws.clone();
    let join = cx
        .update_window(s.win, |_, w, cx| {
            apinata_app::team_join::open_for(&ws, &server.uri(), "ada", w, cx)
        })
        .unwrap();
    cx.update_window(s.win, |_, w, cx| {
        join.update(cx, |j, cx| {
            j.set_password("hunter2", w, cx);
            j.submit(w, cx);
        })
    })
    .unwrap();
    wait_for(cx, "in sync again", |cx| {
        state(cx, &s) == Some(TeamState::Idle)
    });
    let after = s.ws.read_with(cx, |w, _| w.team_sync(&s.root)).unwrap();
    assert!(!std::sync::Arc::ptr_eq(&before, &after), "a new loop runs");
}

// Signs in to `server` as `username` through the join dialog.
fn sign_in_as(cx: &mut TestAppContext, s: &Started, server: &wiremock::MockServer, username: &str) {
    let ws = s.ws.clone();
    let join = cx
        .update_window(s.win, |_, w, cx| {
            apinata_app::team_join::open_for(&ws, &server.uri(), username, w, cx)
        })
        .unwrap();
    cx.update_window(s.win, |_, w, cx| {
        join.update(cx, |j, cx| {
            j.set_password("hunter2", w, cx);
            j.submit(w, cx);
        })
    })
    .unwrap();
}

// The author and committer of `rev` in the repository at `git_dir`, as `name <email>` each.
fn signature(sb: &Sandbox, git_dir: &std::path::Path, rev: &str) -> (String, String) {
    let out = std::process::Command::new("git")
        .arg("--git-dir")
        .arg(git_dir)
        .args(["log", "-1", "--format=%an <%ae>%n%cn <%ce>", rev])
        .envs(sb.env())
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    let mut lines = text.lines().map(str::to_owned);
    (lines.next().unwrap(), lines.next().unwrap())
}

// Whether a request to the events stream carried `token`.
fn events_on(cx: &mut TestAppContext, server: &wiremock::MockServer, token: &str) -> bool {
    let bearer = format!("Bearer {token}");
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let seen = handle.block_on(server.received_requests()).unwrap();
    seen.iter().any(|r| {
        r.url.path() == "/api/v1/workspaces/core/api/events"
            && r.headers
                .get("authorization")
                .is_some_and(|v| v == bearer.as_str())
    })
}

#[gpui_kit::test]
fn signing_in_again_commits_under_the_identity_the_server_gives_now(cx: &mut TestAppContext) {
    http_ready(cx);
    let (sb, col) = (Sandbox::new(), collection());
    let bare = shared_origin(&sb, col.path());
    let server = potluck_server(cx);
    let fresh = format!("plk_{}", "C".repeat(43));
    serve_workspace(cx, &server, "write");
    mount(
        cx,
        &server,
        Mock::given(method("GET"))
            .and(path("/api/v1/server"))
            .respond_with(info(true)),
    );
    mount(
        cx,
        &server,
        Mock::given(method("POST"))
            .and(path("/api/v1/session"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "token": fresh, "token_meta": token_meta("tok-8"), "user": user("ada", false)
            }))),
    );
    let root = col.path().to_path_buf();
    let s = team_started(cx, &sb, &col, |paths, secrets| {
        signed_in(paths, secrets, &server, false);
        bind(paths, &root, &server.uri(), Role::Write);
    })
    .0;
    wait_for(cx, "in sync", |cx| state(cx, &s) == Some(TeamState::Idle));
    // From now on the server names ada's commits differently.
    let listed = workspace(&server.uri(), "core", "api", "apinata", "write");
    let renamed = serde_json::json!({
        "user": user("ada", false),
        "identity": {"name": "Ada King", "email": "ada.king@users.noreply.127.0.0.1"},
        "workspaces": [listed],
    });
    mount(
        cx,
        &server,
        Mock::given(method("GET"))
            .and(path("/api/v1/me"))
            .respond_with(ResponseTemplate::new(200).set_body_json(renamed))
            .with_priority(1),
    );
    sign_in_as(cx, &s, &server, "ada");
    wait_for(cx, "the loop on the new token", |cx| {
        events_on(cx, &server, &fresh)
    });
    std::fs::write(col.path().join("b.yml"), request("b", "http://x/b")).unwrap();
    wait_for(cx, "the push", |_| origin_has(&sb, &bare, "b.yml", "x/b"));
    let want = "Ada King <ada.king@users.noreply.127.0.0.1>".to_owned();
    assert_eq!(signature(&sb, &bare, "main"), (want.clone(), want));
}

#[gpui_kit::test]
fn signing_in_as_someone_else_leaves_a_folder_bound_to_the_first_user_signed_out(
    cx: &mut TestAppContext,
) {
    http_ready(cx);
    let (sb, col) = (Sandbox::new(), collection());
    let bare = shared_origin(&sb, col.path());
    let server = potluck_server(cx);
    let theirs = format!("plk_{}", "D".repeat(43));
    serve_workspace(cx, &server, "write");
    mount(
        cx,
        &server,
        Mock::given(method("GET"))
            .and(path("/api/v1/server"))
            .respond_with(info(true)),
    );
    mount(
        cx,
        &server,
        Mock::given(method("POST"))
            .and(path("/api/v1/session"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "token": theirs, "token_meta": token_meta("tok-9"), "user": user("bob", false)
            }))),
    );
    let root = col.path().to_path_buf();
    let s = team_started(cx, &sb, &col, |paths, secrets| {
        signed_in(paths, secrets, &server, false);
        bind(paths, &root, &server.uri(), Role::Write);
    })
    .0;
    wait_for(cx, "in sync", |cx| state(cx, &s) == Some(TeamState::Idle));
    sign_in_as(cx, &s, &server, "bob");
    let (file, url) = (team_file(&s), server_url(&server));
    wait_for(cx, "bob's session", |_| {
        TeamStore::load(&file)
            .unwrap()
            .account(&url)
            .is_some_and(|a| a.username == "bob")
    });
    wait_for(cx, "signed out", |cx| {
        state(cx, &s) == Some(TeamState::Blocked(Block::SignedOut))
    });
    std::fs::write(col.path().join("b.yml"), request("b", "http://x/b")).unwrap();
    wait_for(cx, "a local commit", |cx| {
        s.ws.read_with(cx, |w, _| {
            w.team_status(&s.root).is_some_and(|t| t.unpushed == 1)
        })
    });
    settle(cx);
    assert_eq!(state(cx, &s), Some(TeamState::Blocked(Block::SignedOut)));
    assert!(!origin_has(&sb, &bare, "b.yml", "x/b"), "nothing pushed");
    assert!(!events_on(cx, &server, &theirs), "bob's token is not used");
    // The folder is still ada's, so its local work waits for her under her name.
    let ada = "Ada Lovelace <ada@users.noreply.127.0.0.1>".to_owned();
    let git_dir = col.path().join(".git");
    assert_eq!(signature(&sb, &git_dir, "HEAD"), (ada.clone(), ada));
    let binding = TeamStore::load(&file).unwrap().binding_at(&s.root).cloned();
    assert_eq!(binding.map(|b| b.username).as_deref(), Some("ada"));
}
