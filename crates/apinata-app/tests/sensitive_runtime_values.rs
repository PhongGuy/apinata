//! A runtime variable marked sensitive (or holding a tainted value) is masked wherever the
//! app shows runtime values: the Variables overview, placeholder tooltips and autocomplete.

use crate::support;

use apinata_app::command::Command;
use apinata_app::env_table::SECRET_HIDDEN;
use apinata_app::shell::OpenRuntimeVariables;
use apinata_app::var_complete::choices;
use apinata_app::var_tokens::{SECRET_DOTS, tokens};
use apinata_core::{Collection, ContextOptions, RuntimeVars, VarContext, VariableSet};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;
use wiremock::ResponseTemplate;

const ROOT: &str = "opencollection: 1.0.0\ninfo:\n  name: c\n";
const DEV: &str = "name: dev\nvariables:\n  - secret: true\n    name: key\n";

fn collection() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("r.yml", &request("r", "http://127.0.0.1:9/")),
    ])
}

// The overview's value label at row `ix`.
fn value(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle, ix: usize) -> Option<String> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(("var-overview-value", ix))
            .and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

fn click(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle, id: (&'static str, usize)) {
    cx.update_window(win, |_, w, cx| w.click(id, cx)).unwrap();
    frame(cx, win);
}

fn open_overview(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle) {
    cx.update_window(win, |_, w, cx| w.click("variables-button", cx))
        .unwrap();
    frame(cx, win);
    cx.run_until_parked();
    frame(cx, win);
}

// A value typed into a masked row of the dialog stays masked in the overview too, with an
// eye to reveal it; an unmarked row shows plainly there.
#[gpui_kit::test]
fn a_value_typed_into_a_masked_row_is_masked_in_the_overview(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    let select = Command::SelectEnvironment(Some("dev".into()));
    s.ws.update(cx, |w, cx| w.send(&s.root, select, cx));
    s.ws.update(cx, |w, cx| w.open_item(&s.root, ip("r.yml"), false, cx));
    cx.run_until_parked();
    let vars =
        s.ws.read_with(cx, |w, cx| w.runtime_vars(&s.root, cx))
            .expect("open");
    vars.set_text("key", "old-secret");
    vars.set_text("plain", "shown");
    gpui_kit::VisualTestContext::from_window(s.win, cx).dispatch_action(OpenRuntimeVariables);
    frame(cx, s.win);
    replace_input(
        cx,
        s.win,
        None,
        ("runtime-var-value", 0usize),
        "typed-value",
    );
    cx.update_window(s.win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    frame(cx, s.win);
    assert!(vars.is_sensitive("key"));
    cx.update_window(s.win, |_, w, cx| w.press("escape", cx))
        .unwrap();
    frame(cx, s.win);
    open_overview(cx, s.win);
    // Runtime rows by name: key, then plain.
    assert_eq!(value(cx, s.win, 0).as_deref(), Some(SECRET_HIDDEN));
    assert!(present(cx, s.win, ("var-overview-reveal", 0usize)));
    assert_eq!(value(cx, s.win, 1).as_deref(), Some("shown"));
    assert!(!present(cx, s.win, ("var-overview-reveal", 1usize)));
    click(cx, s.win, ("var-overview-reveal", 0));
    assert_eq!(value(cx, s.win, 0).as_deref(), Some("typed-value"));
    click(cx, s.win, ("var-overview-hide", 0));
    assert_eq!(value(cx, s.win, 0).as_deref(), Some(SECRET_HIDDEN));
}

// A value a script copied from a sensitive source is masked in the overview like in the dialog.
#[gpui_kit::test]
fn a_tainted_runtime_value_is_masked_in_the_overview(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let a = format!(
        "{}runtime:\n  scripts:\n    - type: before-request\n      code: \"bru.setVar('auth', 'Bearer ' + bru.getProcessEnv('TOKEN')); bru.setVar('plain', 'hello')\"\n",
        request("a", &format!("{}/ok", server.uri()))
    );
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &a),
        (".env", "TOKEN=s3cr3t-value\n"),
    ]);
    let root = col.path().to_path_buf();
    let s = started_with(cx, &col, |p| allow_scripts(p, &root));
    s.ws.update(cx, |w, cx| w.open_item(&s.root, ip("a.yml"), false, cx));
    cx.run_until_parked();
    let model = sender(cx, &s.ws);
    assert!(model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx)));
    let result = finished(cx, &model);
    assert!(result.error.is_none());
    open_overview(cx, s.win);
    // Runtime rows by name: auth (holds the .env value), then plain.
    assert_eq!(value(cx, s.win, 0).as_deref(), Some(SECRET_HIDDEN));
    assert!(present(cx, s.win, ("var-overview-reveal", 0usize)));
    assert_eq!(value(cx, s.win, 1).as_deref(), Some("hello"));
    click(cx, s.win, ("var-overview-reveal", 0));
    assert_eq!(value(cx, s.win, 0).as_deref(), Some("Bearer s3cr3t-value"));
}

// The variables `r.yml` sees with `vars` as the runtime store, `dev` selected.
fn set_over(vars: &RuntimeVars) -> (tempfile::TempDir, VariableSet) {
    let dir = collection();
    let c = Collection::open(dir.path()).unwrap();
    let options = ContextOptions::new()
        .env("dev")
        .runtime_vars(vars.clone())
        .process_env(Vec::<(String, String)>::new());
    let ctx = VarContext::new(&c, options).unwrap();
    let set = ctx.variables(c.request(&ip("r.yml")).unwrap());
    (dir, set)
}

// The placeholder tooltip shows dots, not the value, for a runtime variable marked sensitive.
#[test]
fn the_placeholder_tooltip_masks_a_sensitive_runtime_value() {
    let vars = RuntimeVars::new();
    vars.set_text("echoed", "typed-value");
    vars.set_text("plain", "shown");
    vars.mark_sensitive("echoed");
    let (_dir, set) = set_over(&vars);
    let tips: Vec<_> = tokens("{{echoed}} {{plain}}", &set)
        .into_iter()
        .map(|t| t.tip)
        .collect();
    assert_eq!(
        tips,
        [
            format!("Runtime · {SECRET_DOTS}"),
            "Runtime · shown".to_owned()
        ]
    );
}

// Autocomplete previews dots, not the value, for a runtime variable marked sensitive.
#[test]
fn autocomplete_masks_a_sensitive_runtime_value() {
    let vars = RuntimeVars::new();
    vars.set_text("echoed", "typed-value");
    vars.set_text("plain", "shown");
    vars.mark_sensitive("echoed");
    let (_dir, set) = set_over(&vars);
    let all = choices(&set);
    let preview = |n: &str| all.iter().find(|c| c.name == n).map(|c| c.preview.clone());
    assert_eq!(preview("echoed").as_deref(), Some(SECRET_DOTS));
    assert_eq!(preview("plain").as_deref(), Some("shown"));
}
