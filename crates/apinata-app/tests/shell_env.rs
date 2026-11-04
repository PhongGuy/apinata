use crate::support;

use apinata_app::env_selector::{EnvSelector, MANAGE_ENVIRONMENTS, NO_ENVIRONMENT};
use apinata_app::session::Session;
use apinata_app::shell::Shell;
use apinata_app::tabs::TabKind;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, SharedString, TestAppContext, WindowOptions, px,
};
use support::*;

fn shell_with(
    cx: &mut TestAppContext,
    col: &tempfile::TempDir,
) -> (
    tempfile::TempDir,
    AnyWindowHandle,
    Entity<apinata_app::workspace::Workspace>,
) {
    support::init(cx);
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, Session::default());
    let ws = workspace(cx, &session);
    ws.update(cx, |w, cx| w.open(col.path().to_path_buf(), cx));
    let shared = ws.clone();
    let (win, _shell) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| Shell::new(shared, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    (state, win, ws)
}

fn envs() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", "name: dev\n"),
    ])
}

#[gpui_kit::test]
fn the_selector_lists_and_chooses_the_current_collections_environments(cx: &mut TestAppContext) {
    let col = envs();
    let (_state, win, ws) = shell_with(cx, &col);
    let root = col.path().to_path_buf();
    let sel = cx
        .update_window(win, |_, w, cx| {
            cx.new(|cx| EnvSelector::new(ws.clone(), w, cx))
        })
        .unwrap();
    cx.run_until_parked();
    let items = sel.read_with(cx, |s, cx| s.items(cx));
    assert_eq!(
        items,
        [
            SharedString::from(NO_ENVIRONMENT),
            SharedString::from("dev")
        ]
    );
    cx.update_window(win, |_, w, cx| {
        sel.update(cx, |s, cx| s.choose("dev", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        ws.read_with(cx, |w, cx| w.selected_environment(&root, cx))
            .as_deref(),
        Some("dev")
    );
    assert_eq!(
        sel.read_with(cx, |s, _| s.shown()),
        Some(SharedString::from("dev"))
    );
    cx.update_window(win, |_, w, cx| {
        sel.update(cx, |s, cx| s.choose(NO_ENVIRONMENT, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        ws.read_with(cx, |w, cx| w.selected_environment(&root, cx)),
        None
    );
}

#[gpui_kit::test]
fn the_title_bar_and_its_menu_open_the_environments_tab_and_the_cookie_jar(
    cx: &mut TestAppContext,
) {
    let col = envs();
    let (_state, win, ws) = shell_with(cx, &col);
    assert!(present(cx, win, "env-select"));
    cx.update_window(win, |_, w, cx| {
        w.within("title-bar").click("environments", cx)
    })
    .unwrap();
    frame(cx, win);
    assert_eq!(
        ws.read_with(cx, |w, _| w.tabs().active().map(|t| t.kind.clone())),
        Some(TabKind::Environments)
    );
    assert!(present(cx, win, "env-editor"));
    let ix = apinata_app::app_menu::app_menu_index(&apinata_app::shell::OpenCookies)
        .expect("Cookies is in the menu");
    cx.update_window(win, |_, w, cx| w.within("title-bar").click("app-menu", cx))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.within("popup-menu").click(ix, cx))
        .unwrap();
    cx.run_until_parked();
    assert!(
        present(cx, win, "cookie-jar"),
        "the cookie jar dialog is open"
    );
    assert!(
        cx.update_window(win, |_, w, cx| w.has_active_dialog(cx))
            .unwrap()
    );
}

#[gpui_kit::test]
fn manage_environments_opens_the_editor_and_keeps_the_selection(cx: &mut TestAppContext) {
    let col = envs();
    let (_state, win, ws) = shell_with(cx, &col);
    let root = col.path().to_path_buf();
    ws.update(cx, |w, cx| {
        w.select_environment(&root, Some("dev".into()), cx)
    });
    frame(cx, win);
    let sel = cx
        .update_window(win, |_, w, cx| {
            cx.new(|cx| EnvSelector::new(ws.clone(), w, cx))
        })
        .unwrap();
    cx.run_until_parked();
    let second_last = sel.read_with(cx, |s, _| s.listed().iter().rev().nth(1).cloned());
    assert_eq!(second_last.as_deref(), Some(MANAGE_ENVIRONMENTS));
    cx.update_window(win, |_, w, cx| {
        sel.update(cx, |s, cx| s.choose(MANAGE_ENVIRONMENTS, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        ws.read_with(cx, |w, _| w.tabs().active().map(|t| t.kind.clone())),
        Some(TabKind::Environments)
    );
    assert_eq!(
        ws.read_with(cx, |w, cx| w.selected_environment(&root, cx))
            .as_deref(),
        Some("dev")
    );
    assert_eq!(sel.read_with(cx, |s, _| s.shown()).as_deref(), Some("dev"));
}

#[gpui_kit::test]
fn the_manage_row_fits_the_picker(cx: &mut TestAppContext) {
    let col = envs();
    let (_state, win, _ws) = shell_with(cx, &col);
    cx.update_window(win, |_, w, cx| {
        w.within("title-bar").click("env-select", cx)
    })
    .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let picker = w.find("env-control").bounds();
        let row = w.find("env-manage").bounds();
        let style = w.text_style();
        let size = gpui_kit::rems(0.875).to_pixels(w.rem_size());
        let run = style.to_run(MANAGE_ENVIRONMENTS.len());
        let label = w
            .text_system()
            .shape_line(MANAGE_ENVIRONMENTS.into(), size, &[run], None)
            .width;
        assert!(
            row.right() <= picker.right() + px(0.5),
            "row {row:?} picker {picker:?}"
        );
        assert!(label <= row.size.width, "label {label:?} row {row:?}");
    })
    .unwrap();
}
