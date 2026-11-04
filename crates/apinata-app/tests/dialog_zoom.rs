//! The zoom chords zoom the app from inside every dialog: the kit draws dialogs beside the
//! Shell, so each one must carry the zoom bindings itself. The dialog stays open, and focus
//! and the focused field's text stay where they were.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::shell::OpenRuntimeVariables;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, FocusHandle, Focusable as _, TestAppContext};
use support::*;

fn dialog_open(cx: &mut TestAppContext, win: AnyWindowHandle) -> bool {
    cx.update_window(win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap()
}

fn app_zoom(cx: &mut TestAppContext) -> f32 {
    cx.update(|cx| cx.global::<AppState>().settings.read(cx).get().zoom)
}

// Where focus is, as a fresh frame draws it: the focused handle, whether the Shell holds it,
// and the text of `field` when the dialog has one.
type Focus = (Option<FocusHandle>, bool, Option<String>);

fn focus(cx: &mut TestAppContext, s: &Started, field: Option<&'static str>) -> Focus {
    let shell = s.shell.clone();
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let in_shell = shell.read(cx).focus_handle(cx).contains_focused(w, cx);
        let text = field.and_then(|id| w.find(id).value().map(str::to_owned));
        (w.focused(cx), in_shell, text)
    })
    .unwrap()
}

/// Each zoom chord, pressed with focus inside the open dialog `name` (on `field` when given),
/// steps the app zoom and leaves the dialog open with focus and the field's text unchanged.
pub fn zoom_keys_reach(
    cx: &mut TestAppContext,
    s: &Started,
    name: &str,
    field: Option<&'static str>,
) {
    let win = s.win;
    frame(cx, win);
    assert!(dialog_open(cx, win), "{name} is open");
    if let Some(id) = field {
        assert!(holds_focus(cx, win, id), "{name}: {id} holds focus");
    }
    let before = focus(cx, s, field);
    assert!(
        before.0.is_some() && !before.1,
        "{name}: focus is inside the dialog"
    );
    for (k, want) in [("=", 1.1), ("+", 1.2), ("-", 1.1), ("0", 1.0)] {
        cx.simulate_keystrokes(win, &secondary(k));
        frame(cx, win);
        assert_eq!(app_zoom(cx), want, "{name}: {} zooms the app", secondary(k));
        assert!(dialog_open(cx, win), "{name} stays open");
        assert_eq!(
            focus(cx, s, field),
            before,
            "{name} keeps its focus and text"
        );
    }
}

#[gpui_kit::test]
fn the_zoom_keys_reach_preferences(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap();
    replace_input(cx, s.win, None, "prefs-search", "zoo");
    zoom_keys_reach(cx, &s, "Preferences", Some("prefs-search"));
}

#[gpui_kit::test]
fn the_zoom_keys_reach_the_cookie_jar(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| {
        apinata_app::cookie_jar::open_cookie_jar(w, cx);
    })
    .unwrap();
    zoom_keys_reach(cx, &s, "Cookie jar", None);
}

#[gpui_kit::test]
fn the_zoom_keys_reach_the_runtime_variables(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    gpui_kit::VisualTestContext::from_window(s.win, cx).dispatch_action(OpenRuntimeVariables);
    zoom_keys_reach(cx, &s, "Runtime variables", None);
}

#[gpui_kit::test]
fn the_zoom_keys_reach_the_global_environments(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let ws = s.ws.clone();
    cx.update_window(s.win, |_, w, cx| {
        apinata_app::global_env_editor::open_global_env_editor(ws, w, cx);
    })
    .unwrap();
    zoom_keys_reach(cx, &s, "Environments", None);
}

#[gpui_kit::test]
fn the_zoom_keys_reach_the_variables_overview(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| {
        apinata_app::variables_overview::open_failed(w, cx);
    })
    .unwrap();
    zoom_keys_reach(cx, &s, "Variables overview", None);
}

#[gpui_kit::test]
fn the_zoom_keys_reach_the_search_dialog(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.simulate_keystrokes(s.win, &key("shift-f"));
    frame(cx, s.win);
    zoom_keys_reach(cx, &s, "Search", Some("search-input"));
}

#[gpui_kit::test]
fn the_zoom_keys_reach_a_confirm_dialog(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let _answer = cx
        .update_window(s.win, |_, w, cx| {
            apinata_app::confirm::ask(apinata_app::confirm::delete_ask("x", None), w, cx)
        })
        .unwrap();
    zoom_keys_reach(cx, &s, "Confirm", None);
}

#[gpui_kit::test]
fn the_zoom_keys_reach_import_from_link(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let shell = s.shell.downgrade();
    cx.update_window(s.win, |_, w, cx| {
        apinata_app::import_link::open(shell, w, cx);
    })
    .unwrap();
    frame(cx, s.win);
    replace_input(cx, s.win, None, "import-link", "https://h/c.json");
    zoom_keys_reach(cx, &s, "Import from link", Some("import-link"));
}

#[gpui_kit::test]
fn the_zoom_keys_reach_import_curl(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let shell = s.shell.clone();
    cx.update_window(s.win, |_, w, cx| {
        shell.read(cx).focus_handle(cx).focus(w, cx);
        w.dispatch_action(Box::new(apinata_app::shell::ImportCurl), cx);
    })
    .unwrap();
    frame(cx, s.win);
    replace_code(cx, s.win, None, "import-curl-input", "curl http://h/");
    zoom_keys_reach(cx, &s, "Import curl", None);
}

#[gpui_kit::test]
fn the_zoom_keys_reach_the_clone_dialog(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let ws = s.ws.clone();
    cx.update_window(s.win, |_, w, cx| {
        apinata_app::clone_dialog::open(&ws, w, cx)
    })
    .unwrap();
    frame(cx, s.win);
    replace_input(cx, s.win, None, "clone-url", "https://h/r.git");
    zoom_keys_reach(cx, &s, "Clone", Some("clone-url"));
}

#[gpui_kit::test]
fn the_zoom_keys_reach_save_draft(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    press_in_shell(cx, &s, &key("n"));
    press_in_shell(cx, &s, &key("s"));
    frame(cx, s.win);
    replace_input(cx, s.win, None, "save-draft-name", "kept");
    zoom_keys_reach(cx, &s, "Save draft", Some("save-draft-name"));
}

#[gpui_kit::test]
fn the_zoom_keys_reach_set_variable(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let (ws, root) = (s.ws.clone(), s.root.clone());
    cx.update_window(s.win, |_, w, cx| {
        apinata_app::set_variable::open(&ws, &root, "name", "ann", w, cx);
    })
    .unwrap();
    frame(cx, s.win);
    zoom_keys_reach(cx, &s, "Set variable", Some("set-variable-name"));
}

#[gpui_kit::test]
fn the_zoom_keys_reach_the_variable_prompt(cx: &mut TestAppContext) {
    use apinata_app::prompt_vars::{PromptForm, PromptMemory, ask};
    let col = two_requests();
    let s = started(cx, &col);
    let labels = vec!["code".to_owned()];
    let _answers = cx
        .update_window(s.win, |_, w, cx| {
            let form = cx.new(|cx| PromptForm::new(&labels, &PromptMemory::default(), w, cx));
            ask(form, w, cx)
        })
        .unwrap();
    frame(cx, s.win);
    replace_input(cx, s.win, None, "prompt-var-0", "A1");
    zoom_keys_reach(cx, &s, "Variable prompt", Some("prompt-var-0"));
}

#[gpui_kit::test]
fn the_zoom_keys_reach_the_approval_prompt(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let store = apinata_core::TrustStore::empty(s.files.path().join("trusted-hosts.json"));
    apinata_core::ApprovalInbox::new(s.files.path().join("approvals"))
        .request(&store, &s.root, "api.example.com", "a.yml")
        .unwrap();
    cx.update(|cx| cx.global::<AppState>().approvals.clone())
        .update(cx, |a, cx| a.refresh(cx));
    cx.run_until_parked();
    zoom_keys_reach(cx, &s, "Host approval", None);
}

#[gpui_kit::test]
fn the_zoom_keys_reach_generate_code(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    press_in_shell(cx, &s, &key("shift-p"));
    cx.update_window(s.win, |_, w, cx| w.input("generate code", cx))
        .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "enter");
    cx.run_until_parked();
    assert!(cx.update(|cx| apinata_app::generate_code::current(cx).is_some()));
    zoom_keys_reach(cx, &s, "Generate code", None);
}

#[gpui_kit::test]
fn the_zoom_keys_reach_the_changes_dialog(cx: &mut TestAppContext) {
    let sb = crate::git_support::Sandbox::new();
    let col = crate::h::changed(&sb);
    let s = crate::h::git_started(cx, &sb, &col);
    let root = s.root.clone();
    crate::h::open_on(cx, &s, &root);
    zoom_keys_reach(cx, &s, "Changes", None);
}

#[gpui_kit::test]
fn the_zoom_keys_reach_the_import_report(cx: &mut TestAppContext) {
    use crate::import_collection::{answer, click_import, input, shop_with_oauth};
    let col = two_requests();
    let s = started(cx, &col);
    let into = tempfile::tempdir().unwrap();
    click_import(cx, s.win);
    answer(
        cx,
        input(s.files.path(), &shop_with_oauth()),
        into.path().to_path_buf(),
    );
    assert!(present(cx, s.win, "import-summary"));
    zoom_keys_reach(cx, &s, "Import report", None);
}

// A callback off loopback asks for the pasted address, which gives the dialog a field to keep.
#[gpui_kit::test]
fn the_zoom_keys_reach_the_oauth2_sign_in(cx: &mut TestAppContext) {
    use crate::oauth2_signin::{collection, opener, provider, showing, start_send};
    http_ready(cx);
    let server = provider(cx);
    let col = collection(
        &server.uri(),
        "    callbackUrl: https://app.example.test/cb\n",
    );
    let s = started(cx, &col);
    let _opened = opener(cx, false);
    let model = start_send(cx, &s);
    wait_real(cx, |cx| showing(cx).is_some());
    frame(cx, s.win);
    replace_input(
        cx,
        s.win,
        None,
        "oauth2-signin-paste",
        "https://app.example.test/cb",
    );
    zoom_keys_reach(cx, &s, "OAuth2 sign-in", Some("oauth2-signin-paste"));
    cx.update_window(s.win, |_, w, cx| w.click("oauth2-signin-cancel", cx))
        .unwrap();
    finished(cx, &model);
}

#[gpui_kit::test]
fn the_zoom_keys_reach_the_script_prompt(cx: &mut TestAppContext) {
    use apinata_core::{ApprovalInbox, ScriptTrust, script_hash};
    const CODE: &str = "const t = 1";
    let a = format!(
        "{}runtime:\n  scripts:\n    - type: before-request\n      code: \"{CODE}\"\n",
        request("a", "http://127.0.0.1:9/ok")
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let s = started_with(cx, &col, |paths| {
        let trust = ScriptTrust::empty(&paths.script_trust);
        ApprovalInbox::new(&paths.approvals)
            .request_scripts(&trust, col.path(), "a.yml", &[script_hash(CODE)])
            .unwrap();
    });
    cx.run_until_parked();
    frame(cx, s.win);
    assert!(present(cx, s.win, "script-approval-text"));
    zoom_keys_reach(cx, &s, "Script prompt", None);
}

#[gpui_kit::test]
fn the_zoom_keys_reach_save_example(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    crate::save_example::ask_on(cx, &s, "a.yml");
    replace_input(cx, s.win, None, "save-example-name", "kept");
    zoom_keys_reach(cx, &s, "Save example", Some("save-example-name"));
}

#[gpui_kit::test]
fn the_zoom_keys_reach_the_branch_dialog(cx: &mut TestAppContext) {
    let sb = crate::git_support::Sandbox::new();
    let (_dir, s) = crate::branch_shell::app(cx, &sb, |_| {});
    crate::branch_shell::open_picker(cx, &s);
    zoom_keys_reach(cx, &s, "Branch", None);
}

// The banner's Compare shows once the draft holds an edit and the file changes under it.
#[gpui_kit::test]
fn the_zoom_keys_reach_the_compare_dialog(cx: &mut TestAppContext) {
    use apinata_app::command::Command;
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let root = s.root.clone();
    s.ws.update(cx, |w, cx| {
        w.send(&root, edit_url("a.yml", "http://x/mine"), cx)
    });
    cx.run_until_parked();
    let path = col.path().join("a.yml");
    std::fs::write(&path, request("a", "http://x/disk")).unwrap();
    s.ws.update(cx, |w, cx| w.send(&root, Command::Reload(vec![path]), cx));
    cx.run_until_parked();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("banner-compare", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(present(cx, s.win, "compare-dialog"));
    zoom_keys_reach(cx, &s, "Compare", None);
}

// Escape over an unsaved edit in Global environments raises the guard's Save prompt.
#[gpui_kit::test]
fn the_zoom_keys_reach_the_unsaved_edits_prompt(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let globals = write_tree(&[(
        "g.yml",
        "name: g\nvariables:\n  - name: host\n    value: a\n",
    )]);
    let dir = globals.path().to_path_buf();
    s.ws.update(cx, |w, cx| w.set_global_dir(dir, cx));
    cx.run_until_parked();
    let ws = s.ws.clone();
    let editor = cx
        .update_window(s.win, |_, w, cx| {
            apinata_app::global_env_editor::open_global_env_editor(ws, w, cx)
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(s.win, |_, w, cx| {
        editor.update(cx, |e, cx| e.select("g", w, cx))
    })
    .unwrap();
    frame(cx, s.win);
    replace_input(cx, s.win, None, ("env-value", 0usize), "b");
    cx.simulate_keystrokes(s.win, "escape");
    cx.run_until_parked();
    assert!(present(cx, s.win, "confirm-dialog"), "Escape asks first");
    zoom_keys_reach(cx, &s, "Unsaved edits prompt", None);
}

// The zoom bindings sit on the kit's dialog key context, which a dialog drops when it turns its
// keyboard handling off; no dialog in the app may, or zoom would stop there unnoticed.
#[test]
fn no_dialog_turns_its_keyboard_handling_off() {
    let mut bad = Vec::new();
    let mut dirs = vec![std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                if text.contains(".keyboard(false)") {
                    bad.push(path.display().to_string());
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "dialogs without the dialog key context: {bad:?}"
    );
}
