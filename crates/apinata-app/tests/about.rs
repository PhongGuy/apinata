//! The About dialog: reached from the app menu and the palette, it shows the running version
//! and licence and opens the project's pages in the browser.

use crate::support;

use apinata_app::about::{ISSUES_URL, OpenAbout, REPOSITORY_URL};
use apinata_app::app_menu::app_menu_index;
use apinata_app::palette::palette_items;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Action as _, AppContext as _, TestAppContext};
use support::*;

fn dialog_open(cx: &mut TestAppContext, s: &Started) -> bool {
    cx.update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap()
}

fn label(cx: &mut TestAppContext, s: &Started, id: &'static str) -> Option<String> {
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

fn open_about(cx: &mut TestAppContext, s: &Started) {
    cx.dispatch_action(s.win, OpenAbout);
    frame(cx, s.win);
}

fn click(cx: &mut TestAppContext, s: &Started, id: &'static str) {
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.click(id, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_menu_entry_opens_the_about_dialog(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let ix = app_menu_index(&OpenAbout).expect("About is in the app menu");
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("app-menu", cx)
    })
    .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.within("popup-menu").click(ix, cx))
        .unwrap();
    cx.run_until_parked();
    assert!(dialog_open(cx, &s));
    assert!(present(cx, s.win, "about-dialog"));
}

#[gpui_kit::test]
fn the_palette_command_opens_the_about_dialog(cx: &mut TestAppContext) {
    let entry = palette_items()
        .into_iter()
        .find(|e| e.action.name() == OpenAbout.name())
        .expect("About is in the palette");
    assert_eq!(entry.label, "About Apinata");
    let col = two_requests();
    let s = started(cx, &col);
    open_about(cx, &s);
    assert!(present(cx, s.win, "about-dialog"));
}

#[gpui_kit::test]
fn the_dialog_shows_the_running_version_and_licence(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_about(cx, &s);
    assert_eq!(
        label(cx, &s, "about-version").as_deref(),
        Some(format!("Version {}", env!("CARGO_PKG_VERSION")).as_str())
    );
    assert_eq!(
        label(cx, &s, "about-licence").as_deref(),
        Some("Licence: MIT OR Apache-2.0")
    );
}

#[gpui_kit::test]
fn the_repository_link_opens_its_page(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_about(cx, &s);
    click(cx, &s, "about-repository");
    assert_eq!(REPOSITORY_URL, "https://github.com/PhongGuy/apinata");
    assert_eq!(cx.opened_url().as_deref(), Some(REPOSITORY_URL));
}

#[gpui_kit::test]
fn the_issue_link_opens_the_tracker(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_about(cx, &s);
    click(cx, &s, "about-issues");
    assert_eq!(ISSUES_URL, "https://github.com/PhongGuy/apinata/issues");
    assert_eq!(cx.opened_url().as_deref(), Some(ISSUES_URL));
}

#[gpui_kit::test]
fn close_and_escape_each_close_the_dialog(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_about(cx, &s);
    click(cx, &s, "about-close");
    assert!(!dialog_open(cx, &s), "Close closes the dialog");
    open_about(cx, &s);
    assert!(dialog_open(cx, &s));
    press_key(cx, s.win, "escape");
    assert!(!dialog_open(cx, &s), "Escape closes the dialog");
}
