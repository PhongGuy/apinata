//! The team server update line: shown to server admins only, refreshed by Check now.

use crate::support::potluck::{error, me};
use crate::support::team::*;
use crate::support::*;
use apinata_app::team_workspaces::{self, SectionState, WorkspacesView};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const AT: Option<&str> = Some("2026-11-17T06:00:00Z");
const NOTES: &str = "https://example.test/releases/0.4.0";

fn status(check: &str, latest: Option<&str>, checked_at: Option<&str>) -> ResponseTemplate {
    let latest = latest.map(|v| {
        json!({"version": v, "digest": "sha256:00", "published": "2026-11-16T09:00:00Z",
               "notes_url": NOTES})
    });
    ResponseTemplate::new(200).set_body_json(
        json!({"current": "0.3.0", "latest": latest, "checked_at": checked_at, "check": check}),
    )
}

fn answer(cx: &mut TestAppContext, server: &MockServer, verb: &str, at: &str, r: ResponseTemplate) {
    mount(
        cx,
        server,
        Mock::given(method(verb)).and(path(at)).respond_with(r),
    );
}

// The app signed in as ada, admin or not, with the workspace list open and loaded.
fn listed(
    cx: &mut TestAppContext,
    server: &MockServer,
    admin: bool,
) -> (tempfile::TempDir, Started, Entity<WorkspacesView>) {
    answer(cx, server, "GET", "/api/v1/me", me(admin, vec![]));
    answer(
        cx,
        server,
        "GET",
        "/api/v1/teams",
        ResponseTemplate::new(200).set_body_json(json!([])),
    );
    let (col, s, _secrets) = team_app(cx, |paths, secrets| {
        signed_in(paths, secrets, server, admin)
    });
    let ws = s.ws.clone();
    let view = cx
        .update_window(s.win, |_, w, cx| team_workspaces::open(&ws, w, cx))
        .unwrap()
        .expect("signed in");
    wait_for(cx, "the list", |cx| {
        view.read_with(cx, |v, _| *v.sections()[0].state() == SectionState::Ready)
    });
    frame(cx, s.win);
    (col, s, view)
}

fn line(cx: &mut TestAppContext, view: &Entity<WorkspacesView>) -> Option<String> {
    view.read_with(cx, |v, _| v.sections()[0].update_line().map(str::to_owned))
}

fn line_for(cx: &mut TestAppContext, body: ResponseTemplate) -> Option<String> {
    http_ready(cx);
    let server = potluck_server(cx);
    answer(cx, &server, "GET", "/api/v1/update", body);
    let (_col, _s, view) = listed(cx, &server, true);
    line(cx, &view)
}

#[gpui_kit::test]
fn an_admin_sees_a_newer_release(cx: &mut TestAppContext) {
    assert_eq!(
        line_for(cx, status("daily", Some("0.4.0"), AT)).as_deref(),
        Some("Potluck 0.4.0 is available. This server runs 0.3.0.")
    );
}

#[gpui_kit::test]
fn an_admin_sees_the_server_is_up_to_date(cx: &mut TestAppContext) {
    assert_eq!(
        line_for(cx, status("daily", None, AT)).as_deref(),
        Some("Potluck 0.3.0 is up to date.")
    );
}

#[gpui_kit::test]
fn an_admin_sees_nothing_was_checked_yet(cx: &mut TestAppContext) {
    assert_eq!(
        line_for(cx, status("daily", None, None)).as_deref(),
        Some("The server has not checked for updates yet.")
    );
}

#[gpui_kit::test]
fn an_admin_sees_the_checks_are_off(cx: &mut TestAppContext) {
    assert_eq!(
        line_for(cx, status("off", None, None)).as_deref(),
        Some("Update checks are off on this server.")
    );
}

#[gpui_kit::test]
fn the_notes_link_opens_in_the_browser(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = potluck_server(cx);
    answer(
        cx,
        &server,
        "GET",
        "/api/v1/update",
        status("daily", Some("0.4.0"), AT),
    );
    let (_col, s, _view) = listed(cx, &server, true);
    let right = |cx: &mut TestAppContext, id: &'static str| {
        cx.update_window(s.win, |_, w, _| {
            let b = w.find((id, 0usize)).bounds();
            b.origin.x + b.size.width
        })
        .unwrap()
    };
    // Both buttons sit inside the section, so a click lands on them, not outside the dialog.
    let edge = right(cx, "team-section");
    assert!(right(cx, "team-update-notes") <= edge && right(cx, "team-update-check") <= edge);
    cx.update_window(s.win, |_, w, cx| w.click(("team-update-notes", 0usize), cx))
        .unwrap();
    frame(cx, s.win);
    assert_eq!(cx.opened_url().as_deref(), Some(NOTES));
}

#[gpui_kit::test]
fn check_now_refreshes_the_line(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = potluck_server(cx);
    answer(
        cx,
        &server,
        "GET",
        "/api/v1/update",
        status("daily", None, None),
    );
    answer(
        cx,
        &server,
        "POST",
        "/api/v1/update/check",
        status("daily", Some("0.4.0"), AT),
    );
    let (_col, s, view) = listed(cx, &server, true);
    cx.update_window(s.win, |_, w, cx| w.click(("team-update-check", 0usize), cx))
        .unwrap();
    wait_for(cx, "the fresh line", |cx| {
        line(cx, &view).is_some_and(|l| l.contains("0.4.0"))
    });
    assert_eq!(hits_on(cx, &server, "POST", "/api/v1/update/check"), 1);
}

#[gpui_kit::test]
fn an_unavailable_check_shows_the_server_message(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = potluck_server(cx);
    answer(
        cx,
        &server,
        "GET",
        "/api/v1/update",
        status("off", None, None),
    );
    let body = error("unavailable", "no update source is configured", None);
    answer(
        cx,
        &server,
        "POST",
        "/api/v1/update/check",
        ResponseTemplate::new(503).set_body_json(body),
    );
    let (_col, s, view) = listed(cx, &server, true);
    cx.update_window(s.win, |_, w, cx| w.click(("team-update-check", 0usize), cx))
        .unwrap();
    wait_for(cx, "the message", |cx| {
        line(cx, &view).is_some_and(|l| l.contains("no update source"))
    });
    assert_eq!(
        line(cx, &view).as_deref(),
        Some("Update check unavailable: no update source is configured")
    );
}

#[gpui_kit::test]
fn a_user_who_is_not_an_admin_never_asks(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = potluck_server(cx);
    for (verb, at) in [("GET", "/api/v1/update"), ("POST", "/api/v1/update/check")] {
        mount(
            cx,
            &server,
            Mock::given(method(verb))
                .and(path(at))
                .respond_with(status("daily", None, AT))
                .expect(0),
        );
    }
    let (_col, s, view) = listed(cx, &server, false);
    assert_eq!(line(cx, &view), None);
    let v = view.clone();
    cx.update_window(s.win, |_, w, cx| {
        v.update(cx, |v, cx| v.check_update(0, w, cx))
    })
    .unwrap();
    frame(cx, s.win);
    assert_eq!(hits_on(cx, &server, "GET", "/api/v1/update"), 0);
    assert_eq!(hits_on(cx, &server, "POST", "/api/v1/update/check"), 0);
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    handle.block_on(server.verify());
}
