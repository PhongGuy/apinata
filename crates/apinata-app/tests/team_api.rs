//! The workspace's team API: the role, the running loop, connect signals and kept watches.

use crate::git_support::Sandbox;
use crate::support::team::*;
use crate::support::*;
use apinata_core::team::{EventsConfig, Identity, Role, TeamAuth, TeamConfig, TeamState};
use gpui_kit::TestAppContext;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const FAST: Duration = Duration::from_millis(50);

fn app(cx: &mut TestAppContext, sb: &Sandbox, col: &tempfile::TempDir) -> Started {
    http_ready(cx);
    team_started(cx, sb, col, |_, _| {}).0
}

#[gpui_kit::test]
fn a_set_role_reaches_the_loop(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let s = app(cx, &sb, &col);
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    let root = s.root.clone();
    assert_eq!(
        s.ws.read_with(cx, |w, _| w.team_role(&root)),
        Some(Role::Write)
    );
    assert!(s.ws.read_with(cx, |w, _| w.team_sync(&root).is_some()));
    s.ws.update(cx, |w, cx| w.set_team_role(&root, Role::Read, cx));
    assert_eq!(
        s.ws.read_with(cx, |w, _| w.team_role(&root)),
        Some(Role::Read)
    );
    wait_for(cx, "read only", |cx| {
        s.ws.read_with(cx, |w, _| {
            matches!(
                w.team_status(&root).map(|t| t.state),
                Some(TeamState::ReadOnly { .. })
            )
        })
    });
}

#[gpui_kit::test]
fn a_collection_without_a_loop_has_no_team_api(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let s = app(cx, &sb, &col);
    let root = s.root.clone();
    s.ws.read_with(cx, |w, _| {
        assert_eq!(w.team_role(&root), None);
        assert!(w.team_sync(&root).is_none());
        assert!(w.team_connects(&root).is_none());
    });
}

#[gpui_kit::test]
fn each_events_connect_is_signalled(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let s = app(cx, &sb, &col);
    let server = potluck_server(cx);
    serve_workspace(cx, &server, "write");
    let auth = TeamAuth::new(server.uri(), "plk_test");
    let config = TeamConfig::new(
        s.root.clone(),
        copies_of(&s),
        auth.clone(),
        Identity::new("Ada", "ada@example.com"),
        Role::Write,
    )
    .base_options(sb.options())
    .debounce_unclamped(FAST);
    let events = EventsConfig::new(
        format!("{}/api/v1/workspaces/core/api/events", server.uri()),
        auth,
        "main",
    );
    let root = s.root.clone();
    s.ws.update(cx, |w, cx| w.start_team(&root, config, Some(events), cx));
    let connects =
        s.ws.read_with(cx, |w, _| w.team_connects(&root))
            .expect("a loop");
    wait_for(cx, "a connect", |_| connects.try_recv().is_ok());
}

#[gpui_kit::test]
fn a_kept_watch_ends_when_the_loop_is_replaced(cx: &mut TestAppContext) {
    struct Flag(Arc<AtomicBool>);
    impl Drop for Flag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let s = app(cx, &sb, &col);
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    let dropped = Arc::new(AtomicBool::new(false));
    let flag = Flag(dropped.clone());
    let root = s.root.clone();
    s.ws.update(cx, |w, cx| {
        let task = cx.spawn(async move |_, _| {
            let _flag = flag;
            std::future::pending::<()>().await;
        });
        w.keep_team_watch(&root, task);
    });
    assert!(!dropped.load(Ordering::SeqCst));
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    cx.run_until_parked();
    assert!(dropped.load(Ordering::SeqCst));
}
