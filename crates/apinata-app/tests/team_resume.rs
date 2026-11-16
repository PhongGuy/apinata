//! Bound folders restart team mode at launch with the role the server gives; each connect and
//! a periodic check re-read it.

use crate::git_support::Sandbox;
use crate::support::potluck::{error, me, tok, workspace};
use crate::support::team::*;
use crate::support::*;
use apinata_core::SecretStore;
use apinata_core::team::{
    Account, Block, Identity, Role, ServerUrl, TeamState, TeamStore, token_key,
};
use gpui_kit::TestAppContext;
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

fn state(cx: &mut TestAppContext, s: &Started) -> Option<TeamState> {
    s.ws.read_with(cx, |w, _| w.team_status(&s.root).map(|t| t.state))
}

fn role(cx: &mut TestAppContext, s: &Started) -> Option<Role> {
    s.ws.read_with(cx, |w, _| w.team_role(&s.root))
}

#[gpui_kit::test]
fn a_bound_folder_restarts_at_launch_with_the_role_the_server_gives(cx: &mut TestAppContext) {
    http_ready(cx);
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let server = potluck_server(cx);
    serve_workspace(cx, &server, "read");
    let root = col.path().to_path_buf();
    let (s, _secrets) = team_started(cx, &sb, &col, |paths, secrets| {
        signed_in(paths, secrets, &server, false);
        bind(paths, &root, &server.uri(), Role::Write);
    });
    wait_for(cx, "the read-only loop", |cx| {
        state(cx, &s) == Some(TeamState::ReadOnly { paused: false })
    });
    assert_eq!(role(cx, &s), Some(Role::Read));
    let store = TeamStore::load(&team_file(&s)).unwrap();
    assert_eq!(store.binding_at(&root).unwrap().role, Role::Read);
}

#[gpui_kit::test]
fn a_rejected_token_at_launch_resumes_signed_out(cx: &mut TestAppContext) {
    http_ready(cx);
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let server = potluck_server(cx);
    let refused =
        ResponseTemplate::new(401).set_body_json(error("unauthenticated", "Revoked.", None));
    mount(
        cx,
        &server,
        Mock::given(method("GET"))
            .and(path("/api/v1/me"))
            .respond_with(refused),
    );
    let root = col.path().to_path_buf();
    let (s, _secrets) = team_started(cx, &sb, &col, |paths, secrets| {
        signed_in(paths, secrets, &server, false);
        bind(paths, &root, &server.uri(), Role::Write);
    });
    wait_for(cx, "signed out", |cx| {
        state(cx, &s) == Some(TeamState::Blocked(Block::SignedOut))
    });
}

#[gpui_kit::test]
fn an_unreachable_server_still_commits_under_the_cached_identity(cx: &mut TestAppContext) {
    http_ready(cx);
    let (sb, col) = (Sandbox::new(), collection());
    let bare = shared_origin(&sb, col.path());
    let dead = format!("http://127.0.0.1:{}", dead_port());
    let root = col.path().to_path_buf();
    let (s, _secrets) = team_started(cx, &sb, &col, |paths, secrets| {
        let url = ServerUrl::parse(&dead).unwrap();
        let mut account = Account::new(&url, "ada", "Ada Lovelace", "7", false);
        account.identity = Some(Identity::new("Ada Cached", "ada@cached.example"));
        TeamStore::update(&paths.team, |t| t.put_account(account)).unwrap();
        secrets.set(&token_key(&url, "ada"), &tok()).unwrap();
        bind(paths, &root, &dead, Role::Write);
    });
    wait_for(cx, "the loop", |cx| state(cx, &s) == Some(TeamState::Idle));
    std::fs::write(col.path().join("b.yml"), request("b", "http://x/b")).unwrap();
    wait_for(cx, "the push", |_| origin_has(&sb, &bare, "b.yml", "x/b"));
    let out = std::process::Command::new("git")
        .arg("--git-dir")
        .arg(&bare)
        .args(["log", "-1", "--format=%an <%ae>", "main"])
        .envs(sb.env())
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "Ada Cached <ada@cached.example>"
    );
}

#[gpui_kit::test]
fn each_connect_re_reads_the_role(cx: &mut TestAppContext) {
    http_ready(cx);
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let server = potluck_server(cx);
    let base = server.uri();
    let first = me(
        false,
        vec![workspace(&base, "core", "api", "apinata", "write")],
    );
    mount(
        cx,
        &server,
        Mock::given(method("GET"))
            .and(path("/api/v1/me"))
            .respond_with(first)
            .up_to_n_times(1),
    );
    serve_workspace(cx, &server, "read");
    let root = col.path().to_path_buf();
    let (s, _secrets) = team_started(cx, &sb, &col, |paths, secrets| {
        signed_in(paths, secrets, &server, false);
        bind(paths, &root, &base, Role::Write);
    });
    wait_for(cx, "the role from the connect", |cx| {
        role(cx, &s) == Some(Role::Read)
    });
    wait_for(cx, "read only", |cx| {
        matches!(state(cx, &s), Some(TeamState::ReadOnly { .. }))
    });
}

#[gpui_kit::test]
fn a_bound_folder_is_resumed_once(cx: &mut TestAppContext) {
    http_ready(cx);
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let server = potluck_server(cx);
    let base = server.uri();
    let listed = me(
        false,
        vec![workspace(&base, "core", "api", "apinata", "write")],
    );
    mount(
        cx,
        &server,
        Mock::given(method("GET"))
            .and(path("/api/v1/me"))
            .respond_with(listed),
    );
    // A failing stream never connects, so only resumes ask `/me`.
    mount(
        cx,
        &server,
        Mock::given(method("GET"))
            .and(path("/api/v1/workspaces/core/api/events"))
            .respond_with(ResponseTemplate::new(500)),
    );
    let root = col.path().to_path_buf();
    let (s, _secrets) = team_started(cx, &sb, &col, |paths, secrets| {
        signed_in(paths, secrets, &server, false);
        bind(paths, &root, &base, Role::Write);
    });
    wait_for(cx, "the loop", |cx| state(cx, &s) == Some(TeamState::Idle));
    s.ws.update(cx, |w, cx| w.set_git(Some(sb.options()), cx));
    settle(cx);
    assert_eq!(hits_on(cx, &server, "GET", "/api/v1/me"), 1);
}

#[gpui_kit::test]
fn an_unbound_folder_starts_no_team_mode_and_asks_nothing(cx: &mut TestAppContext) {
    http_ready(cx);
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let server = potluck_server(cx);
    serve_workspace(cx, &server, "write");
    let (s, _secrets) = team_started(cx, &sb, &col, |paths, secrets| {
        signed_in(paths, secrets, &server, false)
    });
    settle(cx);
    assert_eq!(state(cx, &s), None);
    assert_eq!(hits_on(cx, &server, "GET", "/api/v1/me"), 0);
}

#[gpui_kit::test]
fn a_raised_role_applies_within_a_minute_while_the_stream_stays_quiet(cx: &mut TestAppContext) {
    http_ready(cx);
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let server = potluck_server(cx);
    let base = server.uri();
    let read = me(
        false,
        vec![workspace(&base, "core", "api", "apinata", "read")],
    );
    mount(
        cx,
        &server,
        Mock::given(method("GET"))
            .and(path("/api/v1/me"))
            .respond_with(read),
    );
    // One connect, then a server that never answers again, so no reconnect re-reads the role.
    let events = || Mock::given(method("GET")).and(path("/api/v1/workspaces/core/api/events"));
    let opened = ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(": ok\n\n");
    mount(cx, &server, events().respond_with(opened).up_to_n_times(1));
    let hung = ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(3600));
    mount(cx, &server, events().respond_with(hung));
    let root = col.path().to_path_buf();
    let (s, _secrets) = team_started(cx, &sb, &col, |paths, secrets| {
        signed_in(paths, secrets, &server, false);
        bind(paths, &root, &base, Role::Read);
    });
    wait_for(cx, "the read-only loop", |cx| {
        matches!(state(cx, &s), Some(TeamState::ReadOnly { .. }))
    });
    wait_for(cx, "the connect's re-read", |cx| {
        hits_on(cx, &server, "GET", "/api/v1/me") >= 2
    });

    let raised = me(
        false,
        vec![workspace(&base, "core", "api", "apinata", "write")],
    );
    mount(
        cx,
        &server,
        Mock::given(method("GET"))
            .and(path("/api/v1/me"))
            .respond_with(raised)
            .with_priority(1),
    );
    settle(cx);
    assert_eq!(
        role(cx, &s),
        Some(Role::Read),
        "nothing re-read the role yet"
    );
    cx.executor()
        .advance_clock(apinata_app::team_mode::ROLE_CHECK + std::time::Duration::from_secs(1));
    wait_for(cx, "the raised role", |cx| {
        role(cx, &s) == Some(Role::Write)
    });
    wait_for(cx, "a writable loop", |cx| {
        state(cx, &s) == Some(TeamState::Idle)
    });
}
