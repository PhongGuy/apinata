//! Every select's dropdown rows carry `{select}-option-{key}` ids, and a click on one picks it.

use crate::support;

use apinata_app::auth_section::{AuthMode, AuthSection};
use apinata_app::body_section::{BodyMode, BodySection};
use apinata_app::generate_code::current;
use apinata_app::save_draft::SaveDraftForm;
use apinata_core::{AuthSpec, BodySpec, ItemPath, Target};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Focusable as _, SharedString, TestAppContext};
use std::path::PathBuf;
use support::*;

// Opens the select `select`, checks each of `rows` is listed, then clicks `pick`.
fn open_and_pick(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    select: &str,
    rows: &[&str],
    pick: &str,
) {
    assert!(
        !present(cx, win, pick.to_owned()),
        "a closed {select} lists nothing"
    );
    cx.update_window(win, |_, w, cx| w.click(select.to_owned(), cx))
        .unwrap();
    frame(cx, win);
    for id in rows {
        assert!(present(cx, win, id.to_string()), "{id} is in the tree");
    }
    cx.update_window(win, |_, w, cx| w.click(pick.to_owned(), cx))
        .unwrap();
    frame(cx, win);
}

#[gpui_kit::test]
fn method_rows_have_ids_and_a_click_picks_one(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x.test/ok")),
    ]);
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let editor = editor_of(cx, &s, tab);
    open_and_pick(
        cx,
        s.win,
        "method",
        &["method-option-GET", "method-option-DELETE"],
        "method-option-POST",
    );
    let method = editor.read_with(cx, |e, cx| e.method(cx));
    assert_eq!(method.as_deref(), Some("POST"));
}

#[gpui_kit::test]
fn auth_mode_rows_have_ids_and_a_click_picks_one(cx: &mut TestAppContext) {
    let (win, host) = host_section(cx, AuthSection::new);
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&AuthSpec::None, w, cx))
    })
    .unwrap();
    frame(cx, win);
    open_and_pick(
        cx,
        win,
        "auth-mode",
        &["auth-mode-option-none", "auth-mode-option-oauth2"],
        "auth-mode-option-bearer",
    );
    assert_eq!(s.read_with(cx, |s, _| s.mode()), Some(AuthMode::Bearer));
}

// The auth type list is tall enough for every type when the window has room, so the last
// one shows without a scroll.
#[gpui_kit::test]
fn the_auth_mode_list_shows_every_type_without_scrolling(cx: &mut TestAppContext) {
    let (win, host) = host_section(cx, AuthSection::new);
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&AuthSpec::None, w, cx))
    })
    .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("auth-mode", cx))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, _| {
        for mode in AuthMode::ALL {
            let id = format!("auth-mode-option-{}", mode.key());
            assert!(w.find(SharedString::from(id.clone())).visible(), "{id}");
        }
    })
    .unwrap();
}

// With the last type picked, the opened list still starts at No Auth.
#[gpui_kit::test]
fn the_auth_mode_list_opened_on_the_last_type_still_shows_no_auth(cx: &mut TestAppContext) {
    let (win, host) = host_section(cx, AuthSection::new);
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.set_mode(AuthMode::OAuth2, w, cx))
    })
    .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("auth-mode", cx))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, _| {
        assert!(w.find("auth-mode-option-none").visible());
        assert!(w.find("auth-mode-option-oauth2").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn body_mode_rows_have_ids_and_a_click_picks_one(cx: &mut TestAppContext) {
    let (win, host) = host_section(cx, |w, cx| BodySection::new(PathBuf::from("/c"), w, cx));
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&BodySpec::None, w, cx))
    })
    .unwrap();
    frame(cx, win);
    open_and_pick(
        cx,
        win,
        "body-mode",
        &["body-mode-option-none", "body-mode-option-xml"],
        "body-mode-option-json",
    );
    assert_eq!(s.read_with(cx, |s, _| s.mode()), Some(BodyMode::Json));
}

// Common body types come first and rare ones last, while each keeps the key it saves as.
#[gpui_kit::test]
fn body_mode_rows_list_common_types_first(cx: &mut TestAppContext) {
    let (win, host) = host_section(cx, |w, cx| BodySection::new(PathBuf::from("/c"), w, cx));
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&BodySpec::None, w, cx))
    })
    .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("body-mode", cx))
        .unwrap();
    frame(cx, win);
    let keys = [
        "none",
        "json",
        "form-urlencoded",
        "multipart-form",
        "text",
        "xml",
        "file",
        "sparql",
    ];
    let tops: Vec<_> = cx
        .update_window(win, |_, w, _| {
            keys.map(|k| w.find(format!("body-mode-option-{k}")).bounds().top())
        })
        .unwrap()
        .to_vec();
    assert!(tops.is_sorted_by(|a, b| a < b), "{tops:?}");
    let saved: Vec<_> = BodyMode::ALL.iter().map(|m| m.key()).collect();
    assert_eq!(saved, keys);
}

#[gpui_kit::test]
fn codegen_target_rows_have_ids_and_a_click_picks_one(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x.test/ok")),
    ]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let shell = s.shell.clone();
    cx.update_window(s.win, |_, w, cx| {
        shell.read(cx).focus_handle(cx).focus(w, cx)
    })
    .unwrap();
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.input("generate code", cx))
        .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "enter");
    cx.run_until_parked();
    open_and_pick(
        cx,
        s.win,
        "generate-code-target",
        &[
            "generate-code-target-option-curl",
            "generate-code-target-option-go",
        ],
        "generate-code-target-option-httpie",
    );
    let target = cx.update(|cx| current(cx).map(|v| v.read(cx).target()));
    assert_eq!(target, Some(Target::Httpie));
}

#[gpui_kit::test]
fn env_extends_rows_have_ids_and_a_click_picks_one(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", "name: dev\n"),
        ("environments/prod.yml", "name: prod\n"),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = env_editor(cx, &ws, &root);
    open_and_pick(
        cx,
        win,
        "env-extends",
        &["env-extends-option-none"],
        "env-extends-option-prod",
    );
    let dev = env_view(cx, &ws, &root, "dev");
    assert_eq!(dev.extends.as_deref(), Some("prod"));
}

#[gpui_kit::test]
fn folders_that_differ_only_in_punctuation_get_distinct_rows(cx: &mut TestAppContext) {
    support::init(cx);
    let folders: Vec<(ItemPath, SharedString)> = vec![
        (ItemPath::root(), "Collection root".into()),
        (ip("a/b"), "a/b".into()),
        (ip("a-b"), "a-b".into()),
    ];
    let (win, form) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| SaveDraftForm::new(folders, ItemPath::root(), "r", w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    open_and_pick(
        cx,
        win,
        "save-draft-folder",
        &["save-draft-folder-option-a/b"],
        "save-draft-folder-option-a-b",
    );
    assert_eq!(form.read_with(cx, |f, cx| f.folder(cx)), ip("a-b"));
}
