//! Closing a dialog that holds unsaved edits asks first: Global environments and the cookie
//! jar's form offer Save, Don't save or Cancel, whichever way the close comes. The dialogs that
//! save as you go keep what was typed when they close.

use crate::support;

use std::sync::Arc;

use apinata_app::app::AppState;
use apinata_app::cookie_jar::open_cookie_jar;
use apinata_app::executor::SharedExecutor;
use apinata_app::global_env_editor::{GlobalEnvEditor, open_global_env_editor};
use apinata_app::workspace::Workspace;
use apinata_core::{CookieInfo, ExecOptions, Executor, RuntimeVars};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, TestAppContext, VisualTestContext, WindowOptions,
    point, px,
};
use support::*;

struct Blank;
impl gpui_kit::Render for Blank {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        gpui_kit::div()
    }
}

// The ways a dialog is closed without an answer of its own.
#[derive(Clone, Copy, Debug)]
enum Close {
    XClick,
    XEnter,
    Escape,
    Backdrop,
}

fn close(cx: &mut TestAppContext, win: AnyWindowHandle, how: Close) {
    match how {
        Close::XClick => cx
            .update_window(win, |_, w, cx| {
                w.render_frame(cx);
                w.click("close", cx)
            })
            .unwrap(),
        Close::XEnter => enter_on(cx, win, "close"),
        Close::Escape => cx
            .update_window(win, |_, w, cx| w.press("escape", cx))
            .unwrap(),
        // The bottom left corner of the dialog's full-window layer is backdrop, well below
        // the title bar strip where a press never dismisses.
        Close::Backdrop => cx
            .update_window(win, |_, w, cx| {
                w.render_frame(cx);
                let layer = w.find("dialog").bounds();
                w.click_at("dialog", point(px(4.), layer.size.height - px(4.)), cx);
            })
            .unwrap(),
    }
    cx.run_until_parked();
}

// One drawn frame: whether the prompt shows, and whether `body` is still drawn under it.
fn drawn(cx: &mut TestAppContext, win: AnyWindowHandle, body: &'static str) -> (bool, bool) {
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        (
            w.try_find("confirm-dialog").is_some(),
            w.try_find(body).is_some(),
        )
    })
    .unwrap()
}

fn dialog_open(cx: &mut TestAppContext, win: AnyWindowHandle) -> bool {
    cx.update_window(win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap()
}

const EDITOR: &str = "global-env-editor";
const HOST: &str = "name: g\nvariables:\n  - name: host\n    value: a\n";

// A workspace over `files` as its global directory, with the editor open on `g`.
struct Globals {
    _col: tempfile::TempDir,
    dir: tempfile::TempDir,
    _state: tempfile::TempDir,
    ws: Entity<Workspace>,
    win: AnyWindowHandle,
    editor: Entity<GlobalEnvEditor>,
}

fn globals(cx: &mut TestAppContext, files: &[(&str, &str)]) -> Globals {
    support::init(cx);
    // Enter on a focused button presses it only under the app's keymap.
    cx.update(apinata_app::keymap::init);
    let col = collection();
    let dir = write_tree(files);
    let (state, ws, _root) = open_dir(cx, &col);
    let path = dir.path().to_path_buf();
    ws.update(cx, |w, cx| w.set_global_dir(path, cx));
    cx.run_until_parked();
    let (win, _) = cx
        .update(|cx| gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Blank)))
        .unwrap();
    let shared = ws.clone();
    let editor = cx
        .update_window(win, |_, w, cx| open_global_env_editor(shared, w, cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        editor.update(cx, |e, cx| e.select("g", w, cx))
    })
    .unwrap();
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    frame(cx, win);
    Globals {
        _col: col,
        dir,
        _state: state,
        ws,
        win,
        editor,
    }
}

fn file(g: &Globals, name: &str) -> String {
    std::fs::read_to_string(g.dir.path().join(name)).unwrap()
}

fn value(cx: &mut TestAppContext, g: &Globals, ix: usize) -> String {
    let table = g.editor.read_with(cx, |e, _| e.table().clone());
    table.read_with(cx, |t, cx| {
        t.value_input(ix).unwrap().read(cx).value().to_string()
    })
}

fn stored(cx: &mut TestAppContext, ws: &Entity<Workspace>, env: &str, var: &str) -> Option<String> {
    let (store, globals) =
        ws.read_with(cx, |w, _| (w.store().clone(), w.globals().expect("loaded")));
    store.get(&globals.secret_key(env, var)).unwrap()
}

// Types `b` over the host value, closes `how`, and checks the prompt over the editor; Cancel
// keeps both, a second close asks again, and Don't save closes with the file untouched.
fn cancel_then_dont_save(cx: &mut TestAppContext, how: Close) {
    let g = globals(cx, &[("g.yml", HOST)]);
    replace_input(cx, g.win, None, ("env-value", 0usize), "b");
    close(cx, g.win, how);
    assert_eq!(drawn(cx, g.win, EDITOR), (true, true), "{how:?} asks first");
    answer_confirm(cx, "confirm-cancel");
    assert_eq!(drawn(cx, g.win, EDITOR), (false, true), "Cancel keeps it");
    assert_eq!(value(cx, &g, 0), "b", "Cancel keeps the edit");
    close(cx, g.win, how);
    assert_eq!(drawn(cx, g.win, EDITOR), (true, true), "{how:?} asks again");
    answer_confirm(cx, "confirm-discard");
    assert_eq!(
        drawn(cx, g.win, EDITOR),
        (false, false),
        "Don't save closes"
    );
    assert_eq!(file(&g, "g.yml"), HOST);
}

// As above, answered Save: the dialog closes and the file has the edit.
fn save(cx: &mut TestAppContext, how: Close) {
    let g = globals(cx, &[("g.yml", HOST)]);
    replace_input(cx, g.win, None, ("env-value", 0usize), "b");
    close(cx, g.win, how);
    assert_eq!(drawn(cx, g.win, EDITOR), (true, true), "{how:?} asks first");
    answer_confirm(cx, "confirm-save");
    assert_eq!(drawn(cx, g.win, EDITOR), (false, false), "Save closes");
    assert!(
        file(&g, "g.yml").contains("value: b"),
        "{}",
        file(&g, "g.yml")
    );
}

#[gpui_kit::test]
fn the_x_click_asks_cancel_keeps_and_dont_save_drops(cx: &mut TestAppContext) {
    cancel_then_dont_save(cx, Close::XClick);
}

#[gpui_kit::test]
fn the_x_by_enter_asks_cancel_keeps_and_dont_save_drops(cx: &mut TestAppContext) {
    cancel_then_dont_save(cx, Close::XEnter);
}

#[gpui_kit::test]
fn escape_asks_cancel_keeps_and_dont_save_drops(cx: &mut TestAppContext) {
    cancel_then_dont_save(cx, Close::Escape);
}

#[gpui_kit::test]
fn a_backdrop_press_asks_cancel_keeps_and_dont_save_drops(cx: &mut TestAppContext) {
    cancel_then_dont_save(cx, Close::Backdrop);
}

#[gpui_kit::test]
fn the_x_click_then_save_writes_the_edit(cx: &mut TestAppContext) {
    save(cx, Close::XClick);
}

#[gpui_kit::test]
fn the_x_by_enter_then_save_writes_the_edit(cx: &mut TestAppContext) {
    save(cx, Close::XEnter);
}

#[gpui_kit::test]
fn escape_then_save_writes_the_edit(cx: &mut TestAppContext) {
    save(cx, Close::Escape);
}

#[gpui_kit::test]
fn a_backdrop_press_then_save_writes_the_edit(cx: &mut TestAppContext) {
    save(cx, Close::Backdrop);
}

// A second Escape while the prompt is up answers the prompt, never a second prompt.
#[gpui_kit::test]
fn escape_on_the_prompt_cancels_it_and_keeps_the_editor(cx: &mut TestAppContext) {
    let g = globals(cx, &[("g.yml", HOST)]);
    replace_input(cx, g.win, None, ("env-value", 0usize), "b");
    close(cx, g.win, Close::Escape);
    assert_eq!(drawn(cx, g.win, EDITOR), (true, true));
    close(cx, g.win, Close::Escape);
    assert_eq!(drawn(cx, g.win, EDITOR), (false, true));
    assert_eq!(value(cx, &g, 0), "b");
}

const SECRET: &str = "name: g\nvariables:\n  - name: tok\n    secret: true\n";

// Typing in a secret row that was never sent still counts as an edit.
fn type_secret(cx: &mut TestAppContext, g: &Globals) {
    let table = g.editor.read_with(cx, |e, _| e.table().clone());
    cx.update_window(g.win, |_, w, cx| {
        let input = table.read(cx).value_input(0).unwrap().clone();
        input.update(cx, |s, cx| s.set_value("s3", w, cx));
    })
    .unwrap();
}

#[gpui_kit::test]
fn unsent_secret_typing_asks_and_dont_save_stores_nothing(cx: &mut TestAppContext) {
    let g = globals(cx, &[("g.yml", SECRET)]);
    type_secret(cx, &g);
    close(cx, g.win, Close::Escape);
    assert_eq!(drawn(cx, g.win, EDITOR), (true, true));
    answer_confirm(cx, "confirm-discard");
    assert_eq!(drawn(cx, g.win, EDITOR), (false, false));
    assert_eq!(stored(cx, &g.ws, "g", "tok"), None);
}

#[gpui_kit::test]
fn unsent_secret_typing_asks_and_save_stores_it(cx: &mut TestAppContext) {
    let g = globals(cx, &[("g.yml", SECRET)]);
    type_secret(cx, &g);
    close(cx, g.win, Close::XClick);
    assert_eq!(drawn(cx, g.win, EDITOR), (true, true));
    answer_confirm(cx, "confirm-save");
    assert_eq!(drawn(cx, g.win, EDITOR), (false, false));
    assert_eq!(stored(cx, &g.ws, "g", "tok").as_deref(), Some("s3"));
}

#[gpui_kit::test]
fn a_clean_editor_closes_at_once_by_the_x(cx: &mut TestAppContext) {
    let g = globals(cx, &[("g.yml", HOST)]);
    close(cx, g.win, Close::XClick);
    assert_eq!(drawn(cx, g.win, EDITOR), (false, false));
}

#[gpui_kit::test]
fn a_clean_editor_closes_at_once_by_escape(cx: &mut TestAppContext) {
    let g = globals(cx, &[("g.yml", HOST)]);
    close(cx, g.win, Close::Escape);
    assert_eq!(drawn(cx, g.win, EDITOR), (false, false));
}

const OTHER: &str = "name: h\n";

fn selected(cx: &mut TestAppContext, g: &Globals) -> Option<String> {
    g.editor
        .read_with(cx, |e, _| e.selected().map(str::to_owned))
}

fn pick(cx: &mut TestAppContext, g: &Globals, ix: usize) {
    cx.update_window(g.win, |_, w, cx| {
        w.render_frame(cx);
        w.click(("global-env-item", ix), cx)
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn switching_environment_with_an_edit_asks_and_cancel_keeps_it(cx: &mut TestAppContext) {
    let g = globals(cx, &[("g.yml", HOST), ("h.yml", OTHER)]);
    replace_input(cx, g.win, None, ("env-value", 0usize), "b");
    pick(cx, &g, 1);
    assert_eq!(drawn(cx, g.win, EDITOR), (true, true), "the switch asks");
    assert_eq!(selected(cx, &g).as_deref(), Some("g"));
    answer_confirm(cx, "confirm-cancel");
    assert_eq!(selected(cx, &g).as_deref(), Some("g"));
    assert_eq!(value(cx, &g, 0), "b");
    pick(cx, &g, 1);
    answer_confirm(cx, "confirm-save");
    assert_eq!(drawn(cx, g.win, EDITOR), (false, true), "the editor stays");
    assert_eq!(selected(cx, &g).as_deref(), Some("h"));
    assert!(
        file(&g, "g.yml").contains("value: b"),
        "{}",
        file(&g, "g.yml")
    );
}

#[gpui_kit::test]
fn switching_environment_then_dont_save_drops_the_edit(cx: &mut TestAppContext) {
    let g = globals(cx, &[("g.yml", HOST), ("h.yml", OTHER)]);
    replace_input(cx, g.win, None, ("env-value", 0usize), "b");
    pick(cx, &g, 1);
    answer_confirm(cx, "confirm-discard");
    assert_eq!(selected(cx, &g).as_deref(), Some("h"));
    assert_eq!(file(&g, "g.yml"), HOST);
}

const NAME: &str = "global-env-name-input";

// One drawn frame: whether the prompt shows, and whether the name field shows, holds focus and
// is empty.
fn naming(cx: &mut TestAppContext, win: AnyWindowHandle) -> (bool, Option<(bool, String)>) {
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let field = w.try_find(NAME).map(|e| {
            (
                e.focused() == Some(true),
                e.value().unwrap_or("").to_owned(),
            )
        });
        (w.try_find("confirm-dialog").is_some(), field)
    })
    .unwrap()
}

fn press_new(cx: &mut TestAppContext, g: &Globals) {
    cx.update_window(g.win, |_, w, cx| {
        w.render_frame(cx);
        w.click("global-env-create", cx)
    })
    .unwrap();
}

// New with an edit pending asks before the name field opens: Cancel keeps the edit with no
// field, Don't save opens the field focused and empty.
#[gpui_kit::test]
fn new_with_an_edit_asks_before_the_name_field(cx: &mut TestAppContext) {
    let g = globals(cx, &[("g.yml", HOST)]);
    replace_input(cx, g.win, None, ("env-value", 0usize), "b");
    press_new(cx, &g);
    assert_eq!(naming(cx, g.win), (true, None), "New asks first");
    answer_confirm(cx, "confirm-cancel");
    assert_eq!(naming(cx, g.win), (false, None), "Cancel opens no field");
    assert_eq!(value(cx, &g, 0), "b", "Cancel keeps the edit");
    press_new(cx, &g);
    assert_eq!(naming(cx, g.win), (true, None), "New asks again");
    answer_confirm(cx, "confirm-discard");
    assert_eq!(
        naming(cx, g.win),
        (false, Some((true, String::new()))),
        "Don't save opens the field, focused and empty"
    );
    assert_eq!(file(&g, "g.yml"), HOST);
}

#[gpui_kit::test]
fn a_failed_save_keeps_the_editor_and_shows_why(cx: &mut TestAppContext) {
    let g = globals(cx, &[("g.yml", HOST)]);
    replace_input(cx, g.win, None, ("env-value", 0usize), "b");
    std::fs::remove_file(g.dir.path().join("g.yml")).unwrap();
    close(cx, g.win, Close::Escape);
    assert_eq!(drawn(cx, g.win, EDITOR), (true, true));
    answer_confirm(cx, "confirm-save");
    assert_eq!(
        drawn(cx, g.win, EDITOR),
        (false, true),
        "the failed save keeps it"
    );
    assert!(present(cx, g.win, "global-env-error"));
    assert_eq!(value(cx, &g, 0), "b");
}

// The started app with its window active and the jar open over one cookie.
fn jar(cx: &mut TestAppContext) -> (Started, tempfile::TempDir, Arc<Executor>) {
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, _| w.activate_window())
        .unwrap();
    let exec = Arc::new(Executor::new(ExecOptions::new()).unwrap());
    exec.cookies()
        .insert(CookieInfo::new("a.test", "k", "v"))
        .unwrap();
    cx.update(|cx| cx.set_global(SharedExecutor(exec.clone())));
    cx.update_window(s.win, |_, w, cx| open_cookie_jar(w, cx))
        .unwrap();
    frame(cx, s.win);
    (s, col, exec)
}

const JAR: &str = "cookie-jar-scroll";

fn add_cookie(cx: &mut TestAppContext, s: &Started, value: &str) {
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.click("jar-add", cx)
    })
    .unwrap();
    frame(cx, s.win);
    replace_input(cx, s.win, None, "jar-form-domain", "b.test");
    replace_input(cx, s.win, None, "jar-form-name", "n");
    if !value.is_empty() {
        replace_input(cx, s.win, None, "jar-form-value", value);
    }
}

fn names(exec: &Executor) -> Vec<String> {
    exec.cookies().list().into_iter().map(|c| c.name).collect()
}

#[gpui_kit::test]
fn a_typed_cookie_form_asks_cancel_keeps_and_dont_save_drops(cx: &mut TestAppContext) {
    let (s, _col, exec) = jar(cx);
    add_cookie(cx, &s, "v");
    close(cx, s.win, Close::XClick);
    assert_eq!(drawn(cx, s.win, JAR), (true, true), "the x asks");
    answer_confirm(cx, "confirm-cancel");
    assert_eq!(drawn(cx, s.win, JAR), (false, true));
    assert!(present(cx, s.win, "jar-form"), "Cancel keeps the form");
    close(cx, s.win, Close::Escape);
    assert_eq!(drawn(cx, s.win, JAR), (true, true), "Escape asks");
    answer_confirm(cx, "confirm-discard");
    assert_eq!(drawn(cx, s.win, JAR), (false, false));
    assert_eq!(names(&exec), ["k"]);
}

#[gpui_kit::test]
fn a_typed_cookie_form_then_save_inserts_it(cx: &mut TestAppContext) {
    let (s, _col, exec) = jar(cx);
    add_cookie(cx, &s, "v");
    close(cx, s.win, Close::Escape);
    answer_confirm(cx, "confirm-save");
    assert_eq!(drawn(cx, s.win, JAR), (false, false));
    let mut got = names(&exec);
    got.sort();
    assert_eq!(got, ["k", "n"]);
}

#[gpui_kit::test]
fn a_refused_cookie_form_keeps_the_jar_with_its_errors(cx: &mut TestAppContext) {
    let (s, _col, exec) = jar(cx);
    add_cookie(cx, &s, "a;b");
    close(cx, s.win, Close::Escape);
    answer_confirm(cx, "confirm-save");
    assert_eq!(drawn(cx, s.win, JAR), (false, true), "the refusal keeps it");
    assert!(present(cx, s.win, "jar-form-value-error"));
    assert_eq!(names(&exec), ["k"]);
}

#[gpui_kit::test]
fn an_untouched_cookie_form_closes_at_once(cx: &mut TestAppContext) {
    let (s, _col, _exec) = jar(cx);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.click("jar-add", cx)
    })
    .unwrap();
    frame(cx, s.win);
    close(cx, s.win, Close::Escape);
    assert_eq!(drawn(cx, s.win, JAR), (false, false));
}

// The started app with a global directory holding `g`, the editor open on it and edited.
fn started_editing(cx: &mut TestAppContext) -> (Started, tempfile::TempDir, tempfile::TempDir) {
    let col = two_requests();
    let s = started(cx, &col);
    let dir = write_tree(&[("g.yml", HOST)]);
    let path = dir.path().to_path_buf();
    s.ws.update(cx, |w, cx| w.set_global_dir(path, cx));
    cx.run_until_parked();
    let ws = s.ws.clone();
    let editor = cx
        .update_window(s.win, |_, w, cx| open_global_env_editor(ws, w, cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(s.win, |_, w, cx| {
        editor.update(cx, |e, cx| e.select("g", w, cx))
    })
    .unwrap();
    frame(cx, s.win);
    replace_input(cx, s.win, None, ("env-value", 0usize), "b");
    (s, col, dir)
}

fn close_window(cx: &mut TestAppContext, s: &Started) -> bool {
    VisualTestContext::from_window(s.win, cx).simulate_close()
}

fn windows(cx: &mut TestAppContext) -> usize {
    cx.update(|cx| cx.windows().len())
}

#[gpui_kit::test]
fn a_window_close_asks_for_the_editor_and_cancel_keeps_both(cx: &mut TestAppContext) {
    let (s, _col, dir) = started_editing(cx);
    assert!(!close_window(cx, &s), "the close waits for the answer");
    assert_eq!(drawn(cx, s.win, EDITOR), (true, true));
    assert!(!close_window(cx, &s), "a second close while asking");
    answer_confirm(cx, "confirm-cancel");
    assert_eq!(windows(cx), 1);
    assert_eq!(drawn(cx, s.win, EDITOR), (false, true));
    assert!(!close_window(cx, &s));
    answer_confirm(cx, "confirm-discard");
    assert_eq!(windows(cx), 0, "Don't save lets the close through");
    let text = std::fs::read_to_string(dir.path().join("g.yml")).unwrap();
    assert_eq!(text, HOST);
}

#[gpui_kit::test]
fn a_window_close_then_save_writes_the_edit_and_closes(cx: &mut TestAppContext) {
    let (s, _col, dir) = started_editing(cx);
    assert!(!close_window(cx, &s));
    answer_confirm(cx, "confirm-save");
    wait_for(cx, "the window to close", |cx| windows(cx) == 0);
    let text = std::fs::read_to_string(dir.path().join("g.yml")).unwrap();
    assert!(text.contains("value: b"), "{text}");
}

// Runtime variables over a blank window, holding `id = 42`.
fn runtime_vars(cx: &mut TestAppContext) -> (AnyWindowHandle, RuntimeVars) {
    support::init(cx);
    let vars = RuntimeVars::new();
    vars.set_text("id", "42");
    let (win, _) = cx
        .update(|cx| gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Blank)))
        .unwrap();
    let v = vars.clone();
    cx.update_window(win, |_, w, cx| {
        apinata_app::runtime_vars::open_runtime_vars(
            v,
            apinata_core::EnvOverlay::new(),
            Vec::new(),
            Vec::new(),
            w,
            cx,
        );
    })
    .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    frame(cx, win);
    (win, vars)
}

fn runtime_id(vars: &RuntimeVars) -> Option<String> {
    (vars.entries().iter())
        .find(|v| v.name() == "id")
        .and_then(|v| v.value().map(str::to_owned))
}

// Save-as-you-go dialogs keep what was typed when they close, by Escape or by the x.
#[gpui_kit::test]
fn a_runtime_variable_typed_then_escape_is_kept(cx: &mut TestAppContext) {
    let (win, vars) = runtime_vars(cx);
    replace_input(cx, win, None, ("runtime-var-value", 0usize), "43");
    close(cx, win, Close::Escape);
    assert!(!dialog_open(cx, win));
    assert_eq!(runtime_id(&vars).as_deref(), Some("43"));
}

#[gpui_kit::test]
fn a_runtime_variable_typed_then_the_x_is_kept(cx: &mut TestAppContext) {
    let (win, vars) = runtime_vars(cx);
    replace_input(cx, win, None, ("runtime-var-value", 0usize), "43");
    close(cx, win, Close::XClick);
    assert!(!dialog_open(cx, win));
    assert_eq!(runtime_id(&vars).as_deref(), Some("43"));
}

fn zoom(cx: &mut TestAppContext) -> f32 {
    cx.update(|cx| cx.global::<AppState>().settings.read(cx).get().zoom)
}

fn prefs_typed(cx: &mut TestAppContext, how: Close) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap();
    frame(cx, s.win);
    replace_input(cx, s.win, None, "zoom", "150");
    close(cx, s.win, how);
    frame(cx, s.win);
    assert!(!dialog_open(cx, s.win), "{how:?} closes Preferences");
    assert_eq!(zoom(cx), 1.5, "{how:?} keeps the typed zoom");
}

#[gpui_kit::test]
fn a_preferences_field_typed_then_escape_is_kept(cx: &mut TestAppContext) {
    prefs_typed(cx, Close::Escape);
}

#[gpui_kit::test]
fn a_preferences_field_typed_then_the_x_is_kept(cx: &mut TestAppContext) {
    prefs_typed(cx, Close::XClick);
}

// A window close while another dialog sits over the edited editor leaves that dialog alone:
// no prompt opens over it, and the editor asks only once it is on top again.
#[gpui_kit::test]
fn a_window_close_under_another_prompt_does_not_ask_for_the_editor(cx: &mut TestAppContext) {
    let (s, _col, _dir) = started_editing(cx);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.click("global-env-delete", cx)
    })
    .unwrap();
    frame(cx, s.win);
    assert!(present(cx, s.win, "confirm-delete"));
    assert!(!close_window(cx, &s));
    frame(cx, s.win);
    assert!(
        !present(cx, s.win, "confirm-save"),
        "no prompt over the confirm"
    );
    answer_confirm(cx, "confirm-cancel");
    assert_eq!(drawn(cx, s.win, EDITOR), (false, true));
    assert!(!close_window(cx, &s));
    assert!(present(cx, s.win, "confirm-save"), "now the editor asks");
    answer_confirm(cx, "confirm-cancel");
    assert_eq!(windows(cx), 1);
    assert_eq!(drawn(cx, s.win, EDITOR), (false, true));
}

// A window close while the Escape prompt is up waits for its answer.
#[gpui_kit::test]
fn a_window_close_during_the_escape_prompt_follows_dont_save(cx: &mut TestAppContext) {
    let (s, _col, dir) = started_editing(cx);
    close(cx, s.win, Close::Escape);
    assert_eq!(drawn(cx, s.win, EDITOR), (true, true));
    assert!(!close_window(cx, &s));
    answer_confirm(cx, "confirm-discard");
    assert_eq!(windows(cx), 0, "the queued close ran");
    let text = std::fs::read_to_string(dir.path().join("g.yml")).unwrap();
    assert_eq!(text, HOST);
}

#[gpui_kit::test]
fn a_window_close_during_the_escape_prompt_is_dropped_by_cancel(cx: &mut TestAppContext) {
    let (s, _col, _dir) = started_editing(cx);
    close(cx, s.win, Close::Escape);
    assert!(!close_window(cx, &s));
    answer_confirm(cx, "confirm-cancel");
    assert_eq!(windows(cx), 1);
    assert_eq!(drawn(cx, s.win, EDITOR), (false, true));
}

// Creating an environment asks about the current one's edits, and none of its secrets
// travel to the new one.
#[gpui_kit::test]
fn creating_an_environment_asks_and_carries_no_secrets(cx: &mut TestAppContext) {
    let g = globals(cx, &[("g.yml", SECRET)]);
    let table = g.editor.read_with(cx, |e, _| e.table().clone());
    table.update(cx, |_, cx| {
        cx.emit(apinata_app::env_table::EnvTableEvent::SecretValue {
            name: "tok".into(),
            value: "s3".into(),
        })
    });
    press_new(cx, &g);
    assert_eq!(drawn(cx, g.win, EDITOR), (true, true), "New asks first");
    answer_confirm(cx, "confirm-discard");
    // Don't save dropped the edits, so the name goes through without a second prompt.
    cx.simulate_input(g.win, "n");
    cx.simulate_keystrokes(g.win, "enter");
    assert_eq!(drawn(cx, g.win, EDITOR), (false, true), "no second prompt");
    wait_for(cx, "n selected", |cx| {
        selected(cx, &g).as_deref() == Some("n")
    });
    let _ = g.editor.update(cx, |e, cx| e.save(cx));
    cx.run_until_parked();
    assert_eq!(stored(cx, &g.ws, "n", "tok"), None);
    assert_eq!(stored(cx, &g.ws, "g", "tok"), None);
}

#[gpui_kit::test]
fn duplicating_an_edited_environment_asks_and_save_copies_the_edit(cx: &mut TestAppContext) {
    let g = globals(cx, &[("g.yml", HOST)]);
    replace_input(cx, g.win, None, ("env-value", 0usize), "b");
    cx.update_window(g.win, |_, w, cx| {
        w.render_frame(cx);
        w.click("global-env-duplicate", cx)
    })
    .unwrap();
    assert_eq!(
        drawn(cx, g.win, EDITOR),
        (true, true),
        "Duplicate asks first"
    );
    answer_confirm(cx, "confirm-save");
    wait_for(cx, "copy selected", |cx| {
        selected(cx, &g).as_deref() == Some("g copy")
    });
    assert!(
        file(&g, "g.yml").contains("value: b"),
        "{}",
        file(&g, "g.yml")
    );
    let copy = file(&g, "g copy.yml");
    assert!(copy.contains("value: b"), "{copy}");
}

// A dirty guarded dialog over the started app whose guard never saw the dialog open.
fn unopened_guard(cx: &mut TestAppContext) -> (Started, tempfile::TempDir) {
    use apinata_app::dialog_guard::{Guard, guarded, settled};
    use gpui_kit::{InteractiveElement as _, ParentElement as _, TestSupportExt as _};
    let col = two_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| {
        let guard = Guard::new(
            |_| true,
            |_| "Save?".into(),
            |_, _| settled(Ok(())),
            |_, _| {},
        );
        let guard = guarded(guard, w, cx);
        w.open_dialog(cx, move |d, _, _| {
            apinata_app::ui::with_close(guard.apply(d), "Guarded").child(
                gpui_kit::div()
                    .id("guarded-body")
                    .test_support()
                    .child("body"),
            )
        });
    })
    .unwrap();
    frame(cx, s.win);
    (s, col)
}

// Without its own focus node known, the guard cannot tell it is on top, so it never closes.
#[gpui_kit::test]
fn a_guard_that_never_saw_its_dialog_open_fails_closed(cx: &mut TestAppContext) {
    let (s, _col) = unopened_guard(cx);
    close(cx, s.win, Close::Escape);
    assert_eq!(drawn(cx, s.win, "guarded-body"), (true, true));
    answer_confirm(cx, "confirm-discard");
    assert_eq!(drawn(cx, s.win, "guarded-body"), (false, true));
}

// A window close waiting on a guard whose dialog then goes away is released, not held.
#[gpui_kit::test]
fn a_window_close_waiting_on_a_vanished_guard_is_released(cx: &mut TestAppContext) {
    let (s, _col) = unopened_guard(cx);
    close(cx, s.win, Close::Escape);
    answer_confirm(cx, "confirm-discard");
    assert!(!close_window(cx, &s), "the close waits on the guard");
    cx.update_window(s.win, |_, w, cx| w.close_dialog(cx))
        .unwrap();
    frame(cx, s.win);
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(500));
    cx.run_until_parked();
    assert!(close_window(cx, &s), "a later close is not stuck");
}
