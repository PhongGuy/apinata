//! A signed-out folder keeps committing, and signing in again from its dialog resumes and pushes.

use crate::git_support::Sandbox;
use crate::support::potluck::{error, info, session};
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
