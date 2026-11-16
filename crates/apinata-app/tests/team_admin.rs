//! Creating a workspace as a server admin, and signing out.

use crate::support::potluck::{error, me, workspace};
use crate::support::team::*;
use crate::support::*;
use apinata_app::team_workspaces::{self, SectionState, WorkspacesView};
use apinata_core::SecretStore;
use apinata_core::team::{TeamStore, token_key};
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use serde_json::json;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn answer(
    cx: &mut TestAppContext,
    server: &MockServer,
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

fn ready(cx: &mut TestAppContext, s: &Started) -> Entity<WorkspacesView> {
    let ws = s.ws.clone();
    let view = cx
        .update_window(s.win, |_, w, cx| team_workspaces::open(&ws, w, cx))
        .unwrap()
        .expect("signed in");
    wait_for(cx, "the list", |cx| {
        view.read_with(cx, |v, _| *v.sections()[0].state() == SectionState::Ready)
    });
    view
}

fn section_error(cx: &mut TestAppContext, view: &Entity<WorkspacesView>) -> Option<String> {
    view.read_with(cx, |v, _| {
        v.sections()
            .first()
            .and_then(|s| s.error().map(str::to_owned))
    })
}

fn create(
    cx: &mut TestAppContext,
    s: &Started,
    view: &Entity<WorkspacesView>,
    team: &str,
    slug: &str,
    name: &str,
) {
    let v = view.clone();
    cx.update_window(s.win, |_, w, cx| {
        v.update(cx, |v, cx| {
            v.set_new_workspace(0, team, slug, name, w, cx);
            v.create_workspace(0, w, cx);
        })
    })
    .unwrap();
    frame(cx, s.win);
}

fn teams() -> ResponseTemplate {
    ResponseTemplate::new(200)
        .set_body_json(json!([{"slug": "core", "name": "Core", "members": ["ada"]}]))
}

#[gpui_kit::test]
fn only_a_server_admin_may_create(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = potluck_server(cx);
    answer(cx, &server, "GET", "/api/v1/me", me(false, vec![]));
    let (_col, s, _secrets) = team_app(cx, |paths, secrets| {
        signed_in(paths, secrets, &server, false)
    });
    let view = ready(cx, &s);
    assert!(!view.read_with(cx, |v, _| v.sections()[0].can_create()));
}

#[gpui_kit::test]
fn an_admin_creates_a_seeded_apinata_workspace_and_sees_it_listed(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = potluck_server(cx);
    answer(cx, &server, "GET", "/api/v1/me", me(true, vec![]));
    answer(cx, &server, "GET", "/api/v1/teams", teams());
    mount(
        cx,
        &server,
        Mock::given(method("POST"))
            .and(path("/api/v1/teams/core/workspaces"))
            .and(body_json(json!({"slug": "api-tests", "name": "API tests", "kind": "apinata", "default_branch": "main", "seed": true})))
            .respond_with(ResponseTemplate::new(201).set_body_json(workspace(&server.uri(), "core", "api-tests", "apinata", "admin"))),
    );
    let (_col, s, _secrets) = team_app(cx, |paths, secrets| {
        signed_in(paths, secrets, &server, true)
    });
    let view = ready(cx, &s);
    assert!(view.read_with(cx, |v, _| v.sections()[0].can_create()));
    create(cx, &s, &view, "core", "api-tests", "API tests");
    wait_for(cx, "the new row", |cx| {
        view.read_with(cx, |v, _| {
            v.sections()[0].rows().iter().any(|r| r.slug == "api-tests")
        })
    });
    assert_eq!(
        hits_on(cx, &server, "POST", "/api/v1/teams/core/workspaces"),
        1
    );
}

#[gpui_kit::test]
fn a_taken_name_or_a_bad_slug_shows_why(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = potluck_server(cx);
    answer(cx, &server, "GET", "/api/v1/me", me(true, vec![]));
    answer(cx, &server, "GET", "/api/v1/teams", teams());
    answer(
        cx,
        &server,
        "POST",
        "/api/v1/teams/core/workspaces",
        ResponseTemplate::new(409).set_body_json(error("conflict", "Taken.", None)),
    );
    let (_col, s, _secrets) = team_app(cx, |paths, secrets| {
        signed_in(paths, secrets, &server, true)
    });
    let view = ready(cx, &s);
    create(cx, &s, &view, "core", "Bad_Slug", "API");
    assert_eq!(
        section_error(cx, &view).as_deref(),
        Some("slug: Use lowercase letters, digits and dashes, starting with a letter or digit.")
    );
    assert_eq!(
        hits_on(cx, &server, "POST", "/api/v1/teams/core/workspaces"),
        0
    );
    create(cx, &s, &view, "core", "api", "API");
    wait_for(cx, "the conflict", |cx| {
        section_error(cx, &view).is_some_and(|e| e.contains("already exists"))
    });
    assert_eq!(
        section_error(cx, &view).as_deref(),
        Some("A workspace with that name already exists in this team.")
    );
}

#[gpui_kit::test]
fn signing_out_revokes_the_token_and_forgets_the_server(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = potluck_server(cx);
    answer(cx, &server, "GET", "/api/v1/me", me(false, vec![]));
    answer(
        cx,
        &server,
        "DELETE",
        "/api/v1/tokens/7",
        ResponseTemplate::new(204),
    );
    let (_col, s, secrets) = team_app(cx, |paths, secrets| {
        signed_in(paths, secrets, &server, false)
    });
    let view = ready(cx, &s);
    frame(cx, s.win);
    use gpui_kit::test::TestWindowExt as _;
    cx.update_window(s.win, |_, w, cx| w.click(("team-sign-out", 0usize), cx))
        .unwrap();
    let file = team_file(&s);
    wait_for(cx, "the account to go", |_| {
        TeamStore::load(&file).unwrap().accounts().is_empty()
    });
    assert_eq!(hits_on(cx, &server, "DELETE", "/api/v1/tokens/7"), 1);
    assert_eq!(
        secrets
            .get(&token_key(&server_url(&server), "ada"))
            .unwrap(),
        None
    );
    wait_for(cx, "the section to go", |cx| {
        view.read_with(cx, |v, _| v.sections().is_empty())
    });
}

#[gpui_kit::test]
fn a_failed_sign_out_keeps_the_token_and_says_why(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = potluck_server(cx);
    answer(cx, &server, "GET", "/api/v1/me", me(false, vec![]));
    answer(
        cx,
        &server,
        "DELETE",
        "/api/v1/tokens/7",
        ResponseTemplate::new(503).set_body_json(error("unavailable", "Busy.", None)),
    );
    let (_col, s, secrets) = team_app(cx, |paths, secrets| {
        signed_in(paths, secrets, &server, false)
    });
    let view = ready(cx, &s);
    let v = view.clone();
    cx.update_window(s.win, |_, w, cx| v.update(cx, |v, cx| v.sign_out(0, w, cx)))
        .unwrap();
    wait_for(cx, "the error", |cx| section_error(cx, &view).is_some());
    assert_eq!(
        section_error(cx, &view).as_deref(),
        Some("The server is unavailable right now. Try again later.")
    );
    assert!(
        secrets
            .get(&token_key(&server_url(&server), "ada"))
            .unwrap()
            .is_some()
    );
}

/// Settings whose clone parent is `dest`, so no test clones under the real home.
fn parent_at(paths: &apinata_app::persist::Paths, dest: &std::path::Path) {
    let mut settings = test_settings();
    settings.clone_parent = Some(dest.to_path_buf());
    apinata_app::persist::save_json(&paths.settings, &settings).unwrap();
}

fn section_of(
    cx: &mut TestAppContext,
    view: &Entity<WorkspacesView>,
    server: &MockServer,
) -> usize {
    let uri = server.uri();
    view.read_with(cx, |v, _| {
        v.sections()
            .iter()
            .position(|s| s.server() == uri)
            .expect("a section")
    })
}

#[gpui_kit::test]
fn a_failed_open_reports_on_its_own_server_after_another_signs_out(cx: &mut TestAppContext) {
    use apinata_app::clone_job::CloneJob;
    use apinata_core::CloneRequest;
    let sb = crate::git_support::Sandbox::new();
    let dest = sb.path().join("dest");
    http_ready(cx);
    // The section signed out sorts first, so removing it shifts the one that stays.
    let (mut one, mut two) = (potluck_server(cx), potluck_server(cx));
    if two.uri() < one.uri() {
        std::mem::swap(&mut one, &mut two);
    }
    for server in [&one, &two] {
        let listed = vec![workspace(&server.uri(), "core", "api", "apinata", "write")];
        answer(cx, server, "GET", "/api/v1/me", me(false, listed));
        answer(
            cx,
            server,
            "DELETE",
            "/api/v1/tokens/7",
            ResponseTemplate::new(204),
        );
    }
    // The clone of the server that stays fails only once the other sign out is done.
    let slow = ResponseTemplate::new(404).set_delay(std::time::Duration::from_millis(1500));
    answer(cx, &two, "GET", "/slow/repo.git/info/refs", slow);
    let (_col, s, _secrets) = team_app(cx, |paths, secrets| {
        signed_in(paths, secrets, &one, false);
        signed_in(paths, secrets, &two, false);
        parent_at(paths, &dest);
    });
    s.ws.update(cx, |w, cx| w.set_git(Some(sb.options()), cx));
    let view = ready(cx, &s);
    wait_for(cx, "both lists", |cx| {
        view.read_with(cx, |v, _| {
            v.sections()
                .iter()
                .all(|s| *s.state() == SectionState::Ready)
        })
    });
    let (gone, kept) = (section_of(cx, &view, &one), section_of(cx, &view, &two));
    assert_eq!((gone, kept), (0, 1));
    let url = format!("{}/slow/repo.git", two.uri());
    let v = view.clone();
    cx.update_window(s.win, |_, w, cx| {
        v.update(cx, |v, cx| {
            v.sign_out(gone, w, cx);
            v.open_row_with(
                kept,
                0,
                move |req, opts, cx| {
                    CloneJob::start(CloneRequest::new(url, req.parent, req.name), opts, cx)
                },
                w,
                cx,
            );
        })
    })
    .unwrap();
    wait_for(cx, "the sign out", |cx| {
        view.read_with(cx, |v, _| v.sections().len() == 1)
    });
    wait_for(cx, "the clone error on the kept server", |cx| {
        section_error(cx, &view).is_some()
    });
    assert_eq!(
        view.read_with(cx, |v, _| v.sections()[0].server().to_owned()),
        two.uri()
    );
}

#[gpui_kit::test]
fn a_section_that_is_signing_out_neither_opens_nor_creates(cx: &mut TestAppContext) {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    http_ready(cx);
    let server = potluck_server(cx);
    let listed = vec![workspace(&server.uri(), "core", "api", "apinata", "write")];
    answer(cx, &server, "GET", "/api/v1/me", me(true, listed));
    answer(cx, &server, "GET", "/api/v1/teams", teams());
    answer(
        cx,
        &server,
        "DELETE",
        "/api/v1/tokens/7",
        ResponseTemplate::new(204),
    );
    let dest = tempfile::tempdir().unwrap();
    let (_col, s, _secrets) = team_app(cx, |paths, secrets| {
        signed_in(paths, secrets, &server, true);
        parent_at(paths, dest.path());
    });
    let view = ready(cx, &s);
    let cloned = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cloned);
    let v = view.clone();
    cx.update_window(s.win, |_, w, cx| {
        v.update(cx, |v, cx| {
            v.sign_out(0, w, cx);
            v.open_row_with(
                0,
                0,
                move |req, opts, cx| {
                    flag.store(true, Ordering::SeqCst);
                    apinata_app::clone_job::CloneJob::start(req, opts, cx)
                },
                w,
                cx,
            );
            v.set_new_workspace(0, "core", "api-tests", "API tests", w, cx);
            v.create_workspace(0, w, cx);
        })
    })
    .unwrap();
    wait_for(cx, "the sign out", |cx| {
        view.read_with(cx, |v, _| v.sections().is_empty())
    });
    assert!(
        !cloned.load(Ordering::SeqCst),
        "an Open started while signing out"
    );
    assert_eq!(
        hits_on(cx, &server, "POST", "/api/v1/teams/core/workspaces"),
        0
    );
}

#[gpui_kit::test]
fn signing_out_of_a_signed_out_server_sends_nothing(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = potluck_server(cx);
    answer(
        cx,
        &server,
        "GET",
        "/api/v1/me",
        ResponseTemplate::new(401).set_body_json(error("unauthenticated", "No.", None)),
    );
    let (_col, s, _secrets) = team_app(cx, |paths, secrets| {
        signed_in(paths, secrets, &server, false)
    });
    let ws = s.ws.clone();
    let view = cx
        .update_window(s.win, |_, w, cx| team_workspaces::open(&ws, w, cx))
        .unwrap()
        .expect("signed in");
    wait_for(cx, "signed out", |cx| {
        view.read_with(cx, |v, _| {
            *v.sections()[0].state() == SectionState::SignedOut
        })
    });
    let v = view.clone();
    cx.update_window(s.win, |_, w, cx| v.update(cx, |v, cx| v.sign_out(0, w, cx)))
        .unwrap();
    wait_for(cx, "the section to go", |cx| {
        view.read_with(cx, |v, _| v.sections().is_empty())
    });
    assert!(
        TeamStore::load(&team_file(&s))
            .unwrap()
            .accounts()
            .is_empty()
    );
    assert_eq!(hits_on(cx, &server, "DELETE", "/api/v1/tokens/7"), 0);
}

#[gpui_kit::test]
fn signing_out_waits_for_a_running_create(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = potluck_server(cx);
    answer(cx, &server, "GET", "/api/v1/me", me(true, vec![]));
    answer(cx, &server, "GET", "/api/v1/teams", teams());
    answer(
        cx,
        &server,
        "DELETE",
        "/api/v1/tokens/7",
        ResponseTemplate::new(204),
    );
    let created = workspace(&server.uri(), "core", "api-tests", "apinata", "admin");
    let slow = ResponseTemplate::new(201)
        .set_body_json(created)
        .set_delay(std::time::Duration::from_millis(800));
    answer(cx, &server, "POST", "/api/v1/teams/core/workspaces", slow);
    let (_col, s, secrets) = team_app(cx, |paths, secrets| {
        signed_in(paths, secrets, &server, true)
    });
    let view = ready(cx, &s);
    create(cx, &s, &view, "core", "api-tests", "API tests");
    let v = view.clone();
    cx.update_window(s.win, |_, w, cx| v.update(cx, |v, cx| v.sign_out(0, w, cx)))
        .unwrap();
    wait_for(cx, "the new row", |cx| {
        view.read_with(cx, |v, _| {
            (v.sections().first()).is_some_and(|s| s.rows().iter().any(|r| r.slug == "api-tests"))
        })
    });
    assert_eq!(hits_on(cx, &server, "DELETE", "/api/v1/tokens/7"), 0);
    assert!(
        secrets
            .get(&token_key(&server_url(&server), "ada"))
            .unwrap()
            .is_some()
    );
}
