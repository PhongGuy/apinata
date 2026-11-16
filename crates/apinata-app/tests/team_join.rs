//! Joining a team server: the password goes to the server once, the token to the keychain.

use crate::support::potluck::{error, info, session, tok};
use crate::support::team::*;
use crate::support::*;
use apinata_app::commands::registry;
use apinata_app::team_join::{self, JoinPhase, JoinView};
use apinata_core::SecretStore;
use apinata_core::team::{TeamStore, token_key};
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

fn open_join(cx: &mut TestAppContext, s: &Started) -> Entity<JoinView> {
    let ws = s.ws.clone();
    let view = cx
        .update_window(s.win, |_, w, cx| team_join::open(&ws, w, cx))
        .unwrap();
    frame(cx, s.win);
    view
}

fn fill(
    cx: &mut TestAppContext,
    s: &Started,
    view: &Entity<JoinView>,
    server: &str,
    user: &str,
    password: &str,
) {
    let view = view.clone();
    cx.update_window(s.win, |_, w, cx| {
        view.update(cx, |v, cx| {
            v.set_server(server, w, cx);
            v.set_username(user, w, cx);
            v.set_password(password, w, cx);
        })
    })
    .unwrap();
}

fn submit(cx: &mut TestAppContext, s: &Started) {
    use gpui_kit::test::TestWindowExt as _;
    cx.update_window(s.win, |_, w, cx| w.click("team-join-submit", cx))
        .unwrap();
    frame(cx, s.win);
}

fn answer(
    cx: &mut TestAppContext,
    server: &wiremock::MockServer,
    verb: &str,
    at: &str,
    response: ResponseTemplate,
) {
    mount(
        cx,
        server,
        Mock::given(method(verb))
            .and(path(at))
            .respond_with(response),
    );
}

#[gpui_kit::test]
fn joining_keeps_the_token_in_the_keychain_and_the_server_in_the_team_file(
    cx: &mut TestAppContext,
) {
    http_ready(cx);
    let server = potluck_server(cx);
    answer(cx, &server, "GET", "/api/v1/server", info(true));
    answer(cx, &server, "POST", "/api/v1/session", session(7, false));
    let (_col, s, secrets) = team_app(cx, |_, _| {});
    let view = open_join(cx, &s);
    fill(cx, &s, &view, &server.uri(), "ada", "hunter2");
    submit(cx, &s);
    let url = server_url(&server);
    let file = team_file(&s);
    wait_for(cx, "the account", |_| {
        TeamStore::load(&file).unwrap().account(&url).is_some()
    });
    assert_eq!(secrets.get(&token_key(&url, "ada")).unwrap(), Some(tok()));
    assert_eq!(hits_on(cx, &server, "POST", "/api/v1/session"), 1);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(!text.contains("plk_") && !text.contains("hunter2"));
}

#[gpui_kit::test]
fn a_wrong_password_stays_on_the_form_and_stores_nothing(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = potluck_server(cx);
    answer(cx, &server, "GET", "/api/v1/server", info(true));
    let denied =
        ResponseTemplate::new(401).set_body_json(error("unauthenticated", "Wrong password.", None));
    answer(cx, &server, "POST", "/api/v1/session", denied);
    let (_col, s, secrets) = team_app(cx, |_, _| {});
    let view = open_join(cx, &s);
    fill(cx, &s, &view, &server.uri(), "ada", "nope");
    submit(cx, &s);
    wait_for(cx, "the error", |cx| {
        view.read_with(cx, |v, _| v.error().is_some())
    });
    let (phase, err) = view.read_with(cx, |v, _| (v.phase(), v.error().map(str::to_owned)));
    assert_eq!(phase, JoinPhase::Form);
    assert_eq!(
        err.as_deref(),
        Some("Sign-in failed. Check the username and password.")
    );
    assert_eq!(
        secrets
            .get(&token_key(&server_url(&server), "ada"))
            .unwrap(),
        None
    );
    assert!(!team_file(&s).exists());
}

#[gpui_kit::test]
fn empty_fields_are_named_before_anything_is_sent(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = potluck_server(cx);
    let (_col, s, _secrets) = team_app(cx, |_, _| {});
    let view = open_join(cx, &s);
    let err = |cx: &mut TestAppContext| view.read_with(cx, |v, _| v.error().map(str::to_owned));
    fill(cx, &s, &view, "", "ada", "pw");
    submit(cx, &s);
    assert_eq!(err(cx).as_deref(), Some("Enter the server address."));
    fill(cx, &s, &view, &server.uri(), "", "pw");
    submit(cx, &s);
    assert_eq!(err(cx).as_deref(), Some("Enter your username."));
    fill(cx, &s, &view, &server.uri(), "ada", "");
    submit(cx, &s);
    assert_eq!(err(cx).as_deref(), Some("Enter your password."));
    assert_eq!(hits_on(cx, &server, "GET", "/api/v1/server"), 0);
}

#[gpui_kit::test]
fn a_server_without_password_sign_in_says_so(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = potluck_server(cx);
    answer(cx, &server, "GET", "/api/v1/server", info(false));
    let (_col, s, _secrets) = team_app(cx, |_, _| {});
    let view = open_join(cx, &s);
    fill(cx, &s, &view, &server.uri(), "ada", "hunter2");
    submit(cx, &s);
    wait_for(cx, "the error", |cx| {
        view.read_with(cx, |v, _| v.error().is_some())
    });
    let err = view.read_with(cx, |v, _| v.error().map(str::to_owned));
    assert_eq!(
        err.as_deref(),
        Some("This server does not offer password sign-in.")
    );
    assert_eq!(hits_on(cx, &server, "POST", "/api/v1/session"), 0);
}

#[gpui_kit::test]
fn an_unencrypted_address_shows_a_warning(cx: &mut TestAppContext) {
    http_ready(cx);
    let (_col, s, _secrets) = team_app(cx, |_, _| {});
    let view = open_join(cx, &s);
    fill(cx, &s, &view, "http://team.example.com", "ada", "pw");
    assert!(view.read_with(cx, |v, cx| v.warning(cx)).is_some());
    fill(cx, &s, &view, "https://team.example.com", "ada", "pw");
    assert!(view.read_with(cx, |v, cx| v.warning(cx)).is_none());
}

#[gpui_kit::test]
fn the_welcome_screen_offers_joining_after_its_other_actions(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let col = collection();
    let s = started(cx, &col);
    let root = s.root.clone();
    s.ws.update(cx, |w, cx| w.close(&root, cx));
    frame(cx, s.win);
    let label = cx
        .update_window(s.win, |_, w, _| {
            w.within("no-collection-actions")
                .find(("empty-action", 5usize))
                .label()
                .map(str::to_owned)
        })
        .unwrap();
    assert_eq!(label.as_deref(), Some("Join team server…"));
    cx.update_window(s.win, |_, w, cx| {
        w.within("no-collection-actions")
            .click(("empty-action", 5usize), cx)
    })
    .unwrap();
    frame(cx, s.win);
    assert!(cx.update(|cx| team_join::current(cx)).is_some());
}

#[test]
fn the_palette_offers_joining_a_team_server() {
    let cmds = registry();
    let join = cmds
        .iter()
        .find(|c| c.id == "collection.join_team_server")
        .expect("collection.join_team_server");
    assert_eq!(join.label, "Join team server…");
}

#[gpui_kit::test]
fn cancelling_while_signing_in_revokes_the_new_token_and_stores_nothing(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    http_ready(cx);
    let server = potluck_server(cx);
    // The server check answers late, so a cancel sent too early would never reach the sign-in.
    let late = info(true).set_delay(std::time::Duration::from_millis(300));
    answer(cx, &server, "GET", "/api/v1/server", late);
    let slow = session(7, false).set_delay(std::time::Duration::from_millis(500));
    answer(cx, &server, "POST", "/api/v1/session", slow);
    answer(
        cx,
        &server,
        "DELETE",
        "/api/v1/tokens/7",
        ResponseTemplate::new(204),
    );
    let (_col, s, secrets) = team_app(cx, |_, _| {});
    let view = open_join(cx, &s);
    fill(cx, &s, &view, &server.uri(), "ada", "hunter2");
    submit(cx, &s);
    assert_eq!(view.read_with(cx, |v, _| v.phase()), JoinPhase::Signing);
    // Cancel while the server holds the sign-in answer, so a token exists to give back.
    wait_for(cx, "the sign-in request", |cx| {
        hits_on(cx, &server, "POST", "/api/v1/session") > 0
    });
    cx.update_window(s.win, |_, w, cx| w.click("team-join-cancel", cx))
        .unwrap();
    frame(cx, s.win);
    let file = team_file(&s);
    wait_for(cx, "the sign-in to settle", |cx| {
        file.exists() || hits_on(cx, &server, "DELETE", "/api/v1/tokens/7") > 0
    });
    assert_eq!(hits_on(cx, &server, "DELETE", "/api/v1/tokens/7"), 1);
    assert_eq!(
        secrets
            .get(&token_key(&server_url(&server), "ada"))
            .unwrap(),
        None
    );
    assert!(!file.exists());
}
