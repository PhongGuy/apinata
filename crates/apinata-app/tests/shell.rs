use crate::support;

use apinata_app::command::{Field, NoticeKind, Target};
use apinata_app::session::Session;
use apinata_app::shell::{Shell, collection_row, notice_message};
use gpui_kit::component::WindowExt;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Context, Entity, IntoElement, ParentElement, Render,
    TestAppContext, VisualTestContext, Window, WindowOptions, div, px, size,
};
use support::*;

fn shell(
    cx: &mut TestAppContext,
    start: Session,
) -> (tempfile::TempDir, AnyWindowHandle, Entity<Shell>) {
    support::init(cx);
    let files = tempfile::tempdir().unwrap();
    let session = session_at(cx, &files, start);
    let ws = workspace(cx, &session);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Shell::new(ws, window, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    (files, win, view)
}

fn session_of(cx: &mut TestAppContext, view: &Entity<Shell>) -> Session {
    let s = view.read_with(cx, |v, cx| v.workspace().read(cx).session().clone());
    session_value(cx, &s)
}

fn opened(cx: &mut TestAppContext, view: &Entity<Shell>) -> usize {
    view.read_with(cx, |v, cx| v.workspace().read(cx).collections().len())
}

#[gpui_kit::test]
fn the_window_has_a_title_bar_a_sidebar_and_a_tab_area(cx: &mut TestAppContext) {
    let (_files, win, view) = shell(cx, Session::default());
    let found = cx
        .update_window(win, |_, w, _| {
            [
                w.within("title-bar")
                    .try_find("collection-switcher")
                    .is_some(),
                w.try_find("sidebar").is_some(),
                w.try_find("tab-area").is_some(),
            ]
        })
        .unwrap();
    assert_eq!(found, [true; 3]);
    let sizes = view.read_with(cx, |v, cx| v.split().read(cx).sizes().clone());
    assert_eq!(sizes[0], px(240.));
}

#[gpui_kit::test]
fn the_sidebar_width_is_restored_and_saved(cx: &mut TestAppContext) {
    let start = Session {
        sidebar_width: Some(300.0),
        ..Session::default()
    };
    let (_files, win, view) = shell(cx, start);
    let split = view.read_with(cx, |v, _| v.split().clone());
    assert_eq!(split.read_with(cx, |s, _| s.sizes()[0]), px(300.));
    cx.update_window(win, |_, w, cx| {
        split.update(cx, |s, cx| s.resize_panel(0, px(320.), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(session_of(cx, &view).sidebar_width, Some(320.0));
}

#[gpui_kit::test]
fn window_bounds_are_saved_on_resize(cx: &mut TestAppContext) {
    let (_files, win, view) = shell(cx, Session::default());
    let vcx = VisualTestContext::from_window(win, cx);
    vcx.simulate_resize(size(px(1000.), px(600.)));
    vcx.run_until_parked();
    let saved = session_of(cx, &view).window.expect("bounds saved");
    assert_eq!((saved.width, saved.height), (1000.0, 600.0));
}

#[gpui_kit::test]
fn open_collection_prompts_for_a_directory_and_opens_it(cx: &mut TestAppContext) {
    let col = collection();
    let root = col.path().to_path_buf();
    let (_files, win, view) = shell(cx, Session::default());
    switcher_click(cx, win, "switcher-open-collection");
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(move |o| {
        assert!(o.directories && !o.files && !o.multiple);
        Some(vec![root])
    });
    cx.run_until_parked();
    assert_eq!(opened(cx, &view), 1);
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    assert!(
        cx.update_window(win, |_, w, _| w.try_find(("collection", 0usize)).is_some())
            .unwrap()
    );
}

#[gpui_kit::test]
fn a_cancelled_prompt_opens_nothing(cx: &mut TestAppContext) {
    let (_files, win, view) = shell(cx, Session::default());
    switcher_click(cx, win, "switcher-open-collection");
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    assert_eq!(opened(cx, &view), 0);
}

#[gpui_kit::test]
fn a_failed_open_shows_a_dialog(cx: &mut TestAppContext) {
    let (files, win, _view) = shell(cx, Session::default());
    let missing = files.path().join("gone");
    switcher_click(cx, win, "switcher-open-collection");
    cx.simulate_path_prompt_response(move |_| Some(vec![missing]));
    cx.run_until_parked();
    assert!(
        cx.update_window(win, |_, w, cx| w.has_active_dialog(cx))
            .unwrap()
    );
}

#[gpui_kit::test]
fn a_recent_row_opens_its_collection(cx: &mut TestAppContext) {
    let col = collection();
    let start = Session {
        recent: vec![col.path().to_path_buf()],
        ..Session::default()
    };
    let (_files, win, view) = shell(cx, start);
    cx.update_window(win, |_, w, cx| w.click(("recent", 0usize), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(opened(cx, &view), 1);
}

#[gpui_kit::test]
fn error_notices_become_notifications(cx: &mut TestAppContext) {
    let col = collection();
    let root = col.path().to_path_buf();
    let (_files, win, view) = shell(cx, Session::default());
    let ws = view.read_with(cx, |v, _| v.workspace().clone());
    ws.update(cx, |w, cx| w.open(root.clone(), cx));
    cx.run_until_parked();
    ws.update(cx, |w, cx| {
        w.send(&root, edit_url("nope.yml", "http://x/1"), cx)
    });
    cx.run_until_parked();
    assert_eq!(
        cx.update_window(win, |_, w, cx| w.notifications(cx).len())
            .unwrap(),
        1
    );
}

struct Rows;

impl Render for Rows {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .child(collection_row(0, "a".into(), true))
            .child(collection_row(1, "b".into(), false))
    }
}

#[gpui_kit::test]
fn the_busy_marker_shows_only_on_busy_rows(cx: &mut TestAppContext) {
    support::init(cx);
    let (win, _) = cx
        .update(|cx| gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Rows)))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    let found = cx
        .update_window(win, |_, w, _| {
            (
                w.try_find(("busy", 0usize)).is_some(),
                w.try_find(("busy", 1usize)).is_some(),
            )
        })
        .unwrap();
    assert_eq!(found, (true, false));
}

#[test]
fn notice_messages_cover_errors_warnings_and_confirmations() {
    let t = Target::Request(ip("a.yml"));
    assert!(
        notice_message(&NoticeKind::NotFound(t.clone()))
            .unwrap()
            .contains("a.yml")
    );
    assert!(
        notice_message(&NoticeKind::Unsupported {
            target: t.clone(),
            field: Field::Body
        })
        .is_some()
    );
    let failed = NoticeKind::Failed {
        target: None,
        message: "disk full".into(),
        row: None,
    };
    assert!(notice_message(&failed).unwrap().contains("disk full"));
    assert!(notice_message(&NoticeKind::Warnings(vec!["w".into()])).is_some());
    let added = NoticeKind::ExampleAdded {
        item: ip("a.yml"),
        name: "first".into(),
    };
    assert!(notice_message(&added).unwrap().contains("first"));
    assert_eq!(notice_message(&NoticeKind::Stale(t)), None);
    assert_eq!(notice_message(&NoticeKind::Created(ip("b.yml"))), None);
    assert_eq!(
        notice_message(&NoticeKind::EnvironmentCreated("dev".into())),
        None
    );
    assert_eq!(
        notice_message(&NoticeKind::Moved {
            from: ip("a.yml"),
            to: ip("b.yml")
        }),
        None
    );
}

#[test]
fn notice_messages_use_plain_quotes() {
    let failed = NoticeKind::Failed {
        target: None,
        message: "variable `x` is not defined".into(),
        row: None,
    };
    assert_eq!(
        notice_message(&failed).as_deref(),
        Some("variable \"x\" is not defined")
    );
}

#[test]
fn an_added_example_names_itself_and_its_request() {
    let added = NoticeKind::ExampleAdded {
        item: ip("users/get.yml"),
        name: "created".into(),
    };
    assert_eq!(
        notice_message(&added).as_deref(),
        Some("Saved example \"created\" to users/get.yml")
    );
}
