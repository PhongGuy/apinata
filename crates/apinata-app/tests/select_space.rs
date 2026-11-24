//! Space opens the app's kit selects, as Enter does.

use crate::support;

use apinata_app::env_picker::global_option_id;
use apinata_app::env_picker::option_id;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

fn two_envs() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", "name: dev\n"),
        ("environments/prod.yml", "name: prod\n"),
    ])
}

#[gpui_kit::test]
fn space_opens_the_collection_environment_select(cx: &mut TestAppContext) {
    let col = two_envs();
    let s = started(cx, &col);
    let sel = s.shell.read_with(cx, |v, _| v.env_picker().clone());
    let focus = sel.read_with(cx, |v, _| v.focus_handle().clone());
    cx.update_window(s.win, |_, w, cx| focus.focus(w, cx))
        .unwrap();
    frame(cx, s.win);
    assert!(
        !present(cx, s.win, option_id("dev")),
        "closed to begin with"
    );
    press_key(cx, s.win, "space");
    assert!(
        present(cx, s.win, option_id("dev")),
        "Space opened the list"
    );
}

#[gpui_kit::test]
fn space_opens_the_global_environment_select(cx: &mut TestAppContext) {
    let col = two_envs();
    let globals = write_tree(&[("g.yml", "name: g\n")]);
    let s = started(cx, &col);
    let dir = globals.path().to_path_buf();
    s.ws.update(cx, |w, cx| w.set_global_dir(dir, cx));
    cx.run_until_parked();
    let sel = s.shell.read_with(cx, |v, _| v.env_picker().clone());
    let focus = sel.read_with(cx, |v, _| v.focus_handle().clone());
    cx.update_window(s.win, |_, w, cx| focus.focus(w, cx))
        .unwrap();
    frame(cx, s.win);
    assert!(
        !present(cx, s.win, global_option_id("g")),
        "closed to begin with"
    );
    press_key(cx, s.win, "space");
    assert!(
        present(cx, s.win, global_option_id("g")),
        "Space opened the list"
    );
}
