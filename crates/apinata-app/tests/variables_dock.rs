//! The dock's Variables tab: the active request's variables and its undefined names, with
//! runtime and environment values edited in place.

use crate::support;

use apinata_app::command::Command;
use apinata_app::console_dock::DockTab;
use apinata_app::env_table::SECRET_HIDDEN;
use apinata_app::keymap::ToggleConsole;
use apinata_app::session::Session;
use apinata_app::shell::{OpenVariablesOverview, ShowVariableAtCaret};
use gpui_kit::component::input::InputState;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext};
use support::*;

const BADGE: &str = "undefined-variables";
const DEV: &str =
    "name: dev\nvariables:\n  - name: shared\n    value: env\n  - secret: true\n    name: key\n";

fn collection(url: &str) -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        // A second environment, so the only one is not selected on open.
        ("environments/prod.yml", "name: prod\nvariables: []\n"),
        ("r.yml", &request("r", url)),
    ])
}

// The app over `col` with `r.yml` active and, when given, `env` selected.
fn app(cx: &mut TestAppContext, col: &tempfile::TempDir, env: Option<&str>) -> Started {
    let store = std::sync::Arc::new(apinata_core::MemoryStore::new());
    let key = apinata_core::Collection::open(col.path())
        .unwrap()
        .secret_key("dev", "key");
    apinata_core::SecretStore::set(store.as_ref(), &key, "hunter2-value").unwrap();
    let s = started_with_store(cx, col, store);
    if let Some(env) = env {
        let select = Command::SelectEnvironment(Some(env.into()));
        s.ws.update(cx, |w, cx| w.send(&s.root, select, cx));
    }
    s.ws.update(cx, |w, cx| w.open_item(&s.root, ip("r.yml"), false, cx));
    cx.run_until_parked();
    frame(cx, s.win);
    s
}

fn show_tab(cx: &mut TestAppContext, s: &Started) {
    VisualTestContext::from_window(s.win, cx).dispatch_action(OpenVariablesOverview);
    cx.run_until_parked();
    frame(cx, s.win);
}

fn label(cx: &mut TestAppContext, s: &Started, id: (&'static str, usize)) -> Option<String> {
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

// Labels of `(prefix, 0..)` for as long as such an element exists.
fn labels(cx: &mut TestAppContext, s: &Started, prefix: &'static str) -> Vec<String> {
    (0usize..)
        .map_while(|ix| label(cx, s, (prefix, ix)))
        .collect()
}

// The row index whose name label is `name`.
fn row_of(cx: &mut TestAppContext, s: &Started, name: &str) -> usize {
    let names = labels(cx, s, "var-overview-name");
    names
        .iter()
        .position(|n| n == name)
        .unwrap_or_else(|| panic!("no row {name} in {names:?}"))
}

fn runtime_text(vars: &apinata_core::RuntimeVars, name: &str) -> Option<String> {
    (vars.entries().into_iter())
        .find(|v| v.name() == name)
        .and_then(|v| v.value().map(str::to_owned))
}

fn url_input(cx: &mut TestAppContext, s: &Started) -> gpui_kit::Entity<InputState> {
    let tab = s.ws.read_with(cx, |w, _| w.tabs().active().unwrap().id);
    let editor = s.shell.read_with(cx, |sh, cx| {
        sh.tab_area().read(cx).editor(tab).unwrap().clone()
    });
    editor.read_with(cx, |e, _| e.url_input().clone())
}

fn focused_name(cx: &mut TestAppContext, s: &Started) -> Option<String> {
    let panel = s.shell.read_with(cx, |sh, cx| {
        sh.dock().read(cx).variables().cloned().unwrap()
    });
    panel.read_with(cx, |p, _| p.focused().map(str::to_owned))
}

fn dev_yml(col: &tempfile::TempDir) -> String {
    std::fs::read_to_string(col.path().join("environments/dev.yml")).unwrap()
}

fn dock_tab(cx: &mut TestAppContext, s: &Started) -> (bool, DockTab) {
    let dock = s.shell.read_with(cx, |sh, _| sh.dock().clone());
    let open = s.shell.read_with(cx, |sh, _| sh.dock_open());
    (open, dock.read_with(cx, |d, _| d.tab()))
}

fn type_and_enter(cx: &mut TestAppContext, s: &Started, id: (&'static str, usize), text: &str) {
    replace_input(cx, s.win, None, id, text);
    press_key(cx, s.win, "enter");
    cx.run_until_parked();
    frame(cx, s.win);
}

#[gpui_kit::test]
fn the_entry_points_open_the_dock_on_the_variables_tab(cx: &mut TestAppContext) {
    let col = collection("http://127.0.0.1:9/{{shared}}");
    let s = app(cx, &col, Some("dev"));
    cx.update_window(s.win, |_, w, cx| w.click("variables-button", cx))
        .unwrap();
    frame(cx, s.win);
    assert_eq!(dock_tab(cx, &s), (true, DockTab::Variables));
    assert!(present(cx, s.win, "variables-overview"));
    // No dialog any more: the list lives in the dock.
    assert!(!present(cx, s.win, "variables-overview-scroll"));
}

#[gpui_kit::test]
fn an_undefined_name_has_a_row_and_defining_it_clears_row_and_badge(cx: &mut TestAppContext) {
    let col = collection("http://127.0.0.1:9/{{x}}");
    let s = app(cx, &col, None);
    show_tab(cx, &s);
    assert!(present(cx, s.win, BADGE));
    assert_eq!(labels(cx, &s, "var-undefined-name"), ["x"]);
    // With no environment selected, the value goes to the runtime scope.
    type_and_enter(cx, &s, ("var-undefined-input", 0), "42");
    assert!(labels(cx, &s, "var-undefined-name").is_empty());
    assert!(!present(cx, s.win, BADGE));
    let vars =
        s.ws.read_with(cx, |w, cx| w.runtime_vars(&s.root, cx))
            .unwrap();
    assert_eq!(runtime_text(&vars, "x").as_deref(), Some("42"));
}

#[gpui_kit::test]
fn an_undefined_name_defined_in_place_goes_to_the_selected_environment(cx: &mut TestAppContext) {
    let col = collection("http://127.0.0.1:9/{{x}}");
    let s = app(cx, &col, Some("dev"));
    show_tab(cx, &s);
    type_and_enter(cx, &s, ("var-undefined-input", 0), "from-dock");
    wait_for(cx, "x in dev.yml", |_| {
        std::fs::read_to_string(col.path().join("environments/dev.yml"))
            .unwrap()
            .contains("from-dock")
    });
    frame(cx, s.win);
    assert!(labels(cx, &s, "var-undefined-name").is_empty());
    assert!(!present(cx, s.win, BADGE));
}

#[gpui_kit::test]
fn an_environment_value_edited_in_place_is_written_to_its_file(cx: &mut TestAppContext) {
    let col = collection("http://127.0.0.1:9/{{shared}}");
    let s = app(cx, &col, Some("dev"));
    show_tab(cx, &s);
    let ix = row_of(cx, &s, "shared");
    type_and_enter(cx, &s, ("var-overview-input", ix), "edited");
    wait_for(cx, "edited value in dev.yml", |_| {
        std::fs::read_to_string(col.path().join("environments/dev.yml"))
            .unwrap()
            .contains("value: edited")
    });
    let runtime =
        s.ws.read_with(cx, |w, cx| w.runtime_vars(&s.root, cx))
            .unwrap();
    assert_eq!(
        runtime_text(&runtime, "shared"),
        None,
        "not a runtime value"
    );
    frame(cx, s.win);
    let ix = row_of(cx, &s, "shared");
    assert_eq!(
        label(cx, &s, ("var-overview-value", ix)).as_deref(),
        Some("edited")
    );
}

#[gpui_kit::test]
fn a_runtime_value_edited_in_place_stays_in_the_runtime_scope(cx: &mut TestAppContext) {
    let col = collection("http://127.0.0.1:9/{{tok}}");
    let s = app(cx, &col, Some("dev"));
    let vars =
        s.ws.read_with(cx, |w, cx| w.runtime_vars(&s.root, cx))
            .unwrap();
    vars.set_text("tok", "old");
    show_tab(cx, &s);
    let before = std::fs::read_to_string(col.path().join("environments/dev.yml")).unwrap();
    let ix = row_of(cx, &s, "tok");
    type_and_enter(cx, &s, ("var-overview-input", ix), "new");
    assert_eq!(runtime_text(&vars, "tok").as_deref(), Some("new"));
    settle(cx);
    let after = std::fs::read_to_string(col.path().join("environments/dev.yml")).unwrap();
    assert_eq!(before, after);
}

#[gpui_kit::test]
fn a_secret_stays_masked_and_is_not_edited_in_place(cx: &mut TestAppContext) {
    let col = collection("http://127.0.0.1:9/{{key}}");
    let s = app(cx, &col, Some("dev"));
    show_tab(cx, &s);
    let ix = row_of(cx, &s, "key");
    assert_eq!(
        label(cx, &s, ("var-overview-value", ix)).as_deref(),
        Some(SECRET_HIDDEN)
    );
    assert!(!present(cx, s.win, ("var-overview-input", ix)));
    let all = labels(cx, &s, "var-overview-value");
    assert!(!all.iter().any(|v| v.contains("hunter2")), "{all:?}");
}

#[gpui_kit::test]
fn the_badge_opens_the_variables_tab(cx: &mut TestAppContext) {
    let col = collection("http://127.0.0.1:9/{{x}}");
    let s = app(cx, &col, None);
    cx.update_window(s.win, |_, w, cx| w.click(BADGE, cx))
        .unwrap();
    frame(cx, s.win);
    assert_eq!(dock_tab(cx, &s), (true, DockTab::Variables));
    assert_eq!(labels(cx, &s, "var-undefined-name"), ["x"]);
}

#[gpui_kit::test]
fn the_command_focuses_the_variable_at_the_caret(cx: &mut TestAppContext) {
    let col = collection("http://127.0.0.1:9/{{shared}}/{{x}}");
    let s = app(cx, &col, Some("dev"));
    let tab = s.ws.read_with(cx, |w, _| w.tabs().active().unwrap().id);
    let editor = s.shell.read_with(cx, |sh, cx| {
        sh.tab_area().read(cx).editor(tab).unwrap().clone()
    });
    let url = editor.read_with(cx, |e, _| e.url_input().clone());
    // The caret sits inside `{{shared}}`.
    let caret = "http://127.0.0.1:9/{{sha".len();
    cx.update_window(s.win, |_, w, cx| {
        url.update(cx, |u, cx| {
            u.focus(w, cx);
            u.set_selected_range(caret..caret, cx);
        })
    })
    .unwrap();
    VisualTestContext::from_window(s.win, cx).dispatch_action(ShowVariableAtCaret);
    cx.run_until_parked();
    frame(cx, s.win);
    assert_eq!(dock_tab(cx, &s), (true, DockTab::Variables));
    let panel = s.shell.read_with(cx, |sh, cx| {
        sh.dock().read(cx).variables().cloned().unwrap()
    });
    assert_eq!(
        panel.read_with(cx, |p, _| p.focused().map(str::to_owned)),
        Some("shared".to_owned())
    );
    let ix = row_of(cx, &s, "shared");
    assert!(holds_focus(cx, s.win, ("var-overview-input", ix)));
}

#[gpui_kit::test]
fn the_open_variables_tab_is_saved_and_restored(cx: &mut TestAppContext) {
    let col = collection("http://127.0.0.1:9/{{shared}}");
    let s = app(cx, &col, None);
    show_tab(cx, &s);
    let session = s.ws.read_with(cx, |w, _| w.session().clone());
    let saved = session.read_with(cx, |p, _| p.get().dock);
    assert_eq!(saved, Some(DockTab::Variables));
    VisualTestContext::from_window(s.win, cx).dispatch_action(ToggleConsole);
    frame(cx, s.win);
    assert_eq!(session.read_with(cx, |p, _| p.get().dock), None);

    // A start from a session that had the tab open shows it again.
    let files = blank_data_dir();
    let paths = apinata_app::persist::Paths::in_dir(files.path());
    let start = Session {
        open: vec![col.path().to_path_buf()],
        dock: Some(DockTab::Variables),
        ..Session::default()
    };
    apinata_app::persist::save_json(&paths.session, &start).unwrap();
    let again = started_blank(cx, files);
    assert_eq!(dock_tab(cx, &again), (true, DockTab::Variables));
    assert!(present(cx, again.win, "variables-overview"));
}

// The field may lose its caret while the palette has focus, so the caret seen when the
// palette opened is the one the command reads.
#[gpui_kit::test]
fn the_command_uses_the_caret_from_before_the_palette_opened(cx: &mut TestAppContext) {
    let col = collection("http://127.0.0.1:9/{{shared}}/{{x}}");
    let s = app(cx, &col, Some("dev"));
    let url = url_input(cx, &s);
    let caret = "http://127.0.0.1:9/{{shared}}/{{".len();
    cx.update_window(s.win, |_, w, cx| {
        url.update(cx, |u, cx| {
            u.focus(w, cx);
            u.set_selected_range(caret..caret, cx);
        })
    })
    .unwrap();
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    // The caret moves away while the palette holds focus.
    url.update(cx, |u, cx| u.set_selected_range(0..0, cx));
    cx.update_window(s.win, |_, w, cx| w.input("Show variable at caret", cx))
        .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "enter");
    cx.run_until_parked();
    frame(cx, s.win);
    assert_eq!(focused_name(cx, &s), Some("x".to_owned()));
}

// A secret with no stored value is undefined, but its value belongs in the environment editor,
// never in a plain field.
#[gpui_kit::test]
fn an_unstored_secret_opens_the_environments_instead_of_an_input(cx: &mut TestAppContext) {
    let col = collection("http://127.0.0.1:9/{{key}}");
    let s = started(cx, &col);
    let select = Command::SelectEnvironment(Some("dev".into()));
    s.ws.update(cx, |w, cx| w.send(&s.root, select, cx));
    s.ws.update(cx, |w, cx| w.open_item(&s.root, ip("r.yml"), false, cx));
    cx.run_until_parked();
    show_tab(cx, &s);
    assert_eq!(labels(cx, &s, "var-undefined-name"), ["key"]);
    assert!(!present(cx, s.win, ("var-undefined-input", 0usize)));
    cx.update_window(s.win, |_, w, cx| {
        w.click(("var-undefined-open", 0usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    let kind =
        s.ws.read_with(cx, |w, _| w.tabs().active().map(|e| e.kind.clone()));
    assert_eq!(kind, Some(apinata_app::tabs::TabKind::Environments));
}

// A secret of the active global environment with no stored value never gets a plain field,
// so its value cannot land in a collection file.
#[gpui_kit::test]
fn an_unstored_global_secret_offers_the_global_editor_not_an_input(cx: &mut TestAppContext) {
    let globals = write_tree(&[(
        "g.yml",
        "name: g\nvariables:\n  - secret: true\n    name: gkey\n",
    )]);
    let col = collection("http://127.0.0.1:9/{{gkey}}");
    let s = started(cx, &col);
    let dir = globals.path().to_path_buf();
    s.ws.update(cx, |w, cx| w.set_global_dir(dir, cx));
    cx.run_until_parked();
    s.ws.update(cx, |w, cx| {
        w.select_global_environment(Some("g".into()), cx)
    });
    let select = Command::SelectEnvironment(Some("dev".into()));
    s.ws.update(cx, |w, cx| w.send(&s.root, select, cx));
    s.ws.update(cx, |w, cx| w.open_item(&s.root, ip("r.yml"), false, cx));
    cx.run_until_parked();
    show_tab(cx, &s);
    assert_eq!(labels(cx, &s, "var-undefined-name"), ["gkey"]);
    assert!(!present(cx, s.win, ("var-undefined-input", 0usize)));
    assert!(present(cx, s.win, ("var-undefined-open", 0usize)));
    assert!(!dev_yml(&col).contains("gkey"));
}

// A secret of a global environment that is not active is still a secret: no plain field.
#[gpui_kit::test]
fn a_secret_of_an_inactive_global_environment_gets_no_input(cx: &mut TestAppContext) {
    let globals = write_tree(&[
        (
            "g.yml",
            "name: g\nvariables:\n  - secret: true\n    name: gkey\n",
        ),
        ("h.yml", "name: h\nvariables: []\n"),
    ]);
    let col = collection("http://127.0.0.1:9/{{gkey}}");
    let s = started(cx, &col);
    let dir = globals.path().to_path_buf();
    s.ws.update(cx, |w, cx| w.set_global_dir(dir, cx));
    cx.run_until_parked();
    s.ws.update(cx, |w, cx| {
        w.select_global_environment(Some("h".into()), cx)
    });
    let select = Command::SelectEnvironment(Some("dev".into()));
    s.ws.update(cx, |w, cx| w.send(&s.root, select, cx));
    s.ws.update(cx, |w, cx| w.open_item(&s.root, ip("r.yml"), false, cx));
    cx.run_until_parked();
    show_tab(cx, &s);
    assert_eq!(labels(cx, &s, "var-undefined-name"), ["gkey"]);
    assert!(!present(cx, s.win, ("var-undefined-input", 0usize)));
    assert!(present(cx, s.win, ("var-undefined-open", 0usize)));
    settle(cx);
    assert!(!dev_yml(&col).contains("gkey"));
}

// A name that is a secret only in another environment is still a secret: no plain field.
#[gpui_kit::test]
fn a_secret_of_another_environment_offers_the_editor_not_an_input(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", "name: dev\nvariables: []\n"),
        (
            "environments/prod.yml",
            "name: prod\nvariables:\n  - secret: true\n    name: pkey\n",
        ),
        ("r.yml", &request("r", "http://127.0.0.1:9/{{pkey}}")),
    ]);
    let s = app(cx, &col, Some("dev"));
    show_tab(cx, &s);
    assert_eq!(labels(cx, &s, "var-undefined-name"), ["pkey"]);
    assert!(!present(cx, s.win, ("var-undefined-input", 0usize)));
    assert!(present(cx, s.win, ("var-undefined-open", 0usize)));
    settle(cx);
    assert!(!dev_yml(&col).contains("pkey"));
}

// A refused write leaves the field dirty, so Enter tries again rather than doing nothing.
#[gpui_kit::test]
fn a_refused_write_stays_pending_for_a_retry(cx: &mut TestAppContext) {
    let col = collection("http://127.0.0.1:9/{{x!}}");
    let s = app(cx, &col, Some("dev"));
    show_tab(cx, &s);
    assert_eq!(labels(cx, &s, "var-undefined-name"), ["x!"]);
    type_and_enter(cx, &s, ("var-undefined-input", 0), "v1");
    let panel = s.shell.read_with(cx, |sh, cx| {
        sh.dock().read(cx).variables().cloned().unwrap()
    });
    assert!(panel.read_with(cx, |p, cx| p.pending("x!", cx)));
    settle(cx);
    assert!(!dev_yml(&col).contains("v1"));
}

// An edit still in a field when the active request changes is written, not dropped.
#[gpui_kit::test]
fn switching_tabs_commits_an_edit_in_progress(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("environments/prod.yml", "name: prod\nvariables: []\n"),
        ("r.yml", &request("r", "http://127.0.0.1:9/{{shared}}")),
        ("q.yml", &request("q", "http://127.0.0.1:9/q")),
    ]);
    let s = app(cx, &col, Some("dev"));
    show_tab(cx, &s);
    let ix = row_of(cx, &s, "shared");
    replace_input(cx, s.win, None, ("var-overview-input", ix), "pending");
    s.ws.update(cx, |w, cx| w.open_item(&s.root, ip("q.yml"), false, cx));
    cx.run_until_parked();
    frame(cx, s.win);
    wait_for(cx, "the pending edit in dev.yml", |_| {
        dev_yml(&col).contains("value: pending")
    });
}

// Text a failed leave-save left behind never follows into the next request's field of the
// same name, so a later blur there cannot write it.
#[gpui_kit::test]
fn a_failed_save_on_leaving_leaves_no_text_in_the_next_request(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("environments/prod.yml", "name: prod\nvariables: []\n"),
        ("r.yml", &request("r", "http://127.0.0.1:9/{{x!}}")),
        ("q.yml", &request("q", "http://127.0.0.1:9/{{x!}}")),
    ]);
    let s = app(cx, &col, Some("dev"));
    // Both tabs are open first, so q's undefined names are known the moment it is switched to.
    for item in ["q.yml", "r.yml"] {
        s.ws.update(cx, |w, cx| w.open_item(&s.root, ip(item), false, cx));
        cx.run_until_parked();
        frame(cx, s.win);
    }
    show_tab(cx, &s);
    replace_input(cx, s.win, None, ("var-undefined-input", 0usize), "leftover");
    s.ws.update(cx, |w, cx| w.open_item(&s.root, ip("q.yml"), false, cx));
    cx.run_until_parked();
    frame(cx, s.win);
    assert_eq!(labels(cx, &s, "var-undefined-name"), ["x!"]);
    let panel = s.shell.read_with(cx, |sh, cx| {
        sh.dock().read(cx).variables().cloned().unwrap()
    });
    assert!(!panel.read_with(cx, |p, cx| p.pending("x!", cx)));
    press_key(cx, s.win, "enter");
    settle(cx);
    assert!(!dev_yml(&col).contains("leftover"));
}

// A request whose variables cannot be read says so in the tab.
#[gpui_kit::test]
fn a_failed_lookup_shows_a_note(cx: &mut TestAppContext) {
    let col = collection("http://127.0.0.1:9/{{x}}");
    std::fs::create_dir(col.path().join(".env")).unwrap();
    let s = app(cx, &col, None);
    show_tab(cx, &s);
    assert!(present(cx, s.win, "var-overview-failed"));
}

// Opening the tab without a name leaves focus where it was; with one, its field takes it.
#[gpui_kit::test]
fn opening_the_tab_keeps_focus_on_the_field_in_use(cx: &mut TestAppContext) {
    let col = collection("http://127.0.0.1:9/{{shared}}");
    let s = app(cx, &col, Some("dev"));
    let url = url_input(cx, &s);
    cx.update_window(s.win, |_, w, cx| url.update(cx, |u, cx| u.focus(w, cx)))
        .unwrap();
    show_tab(cx, &s);
    assert!(holds_focus(cx, s.win, "url"));
}

// The badge is a keyboard stop: Tab reaches it and Enter opens the tab.
#[gpui_kit::test]
fn the_badge_opens_the_tab_from_the_keyboard(cx: &mut TestAppContext) {
    let col = collection("http://127.0.0.1:9/{{x}}");
    let s = app(cx, &col, None);
    enter_on(cx, s.win, "undefined-variables-button");
    cx.run_until_parked();
    frame(cx, s.win);
    assert_eq!(dock_tab(cx, &s), (true, DockTab::Variables));
}

#[gpui_kit::test]
fn console_and_network_come_back_open_too(cx: &mut TestAppContext) {
    let col = collection("http://127.0.0.1:9/");
    for tab in [DockTab::Console, DockTab::Network] {
        let files = blank_data_dir();
        let paths = apinata_app::persist::Paths::in_dir(files.path());
        let start = Session {
            open: vec![col.path().to_path_buf()],
            dock: Some(tab),
            ..Session::default()
        };
        apinata_app::persist::save_json(&paths.session, &start).unwrap();
        let s = started_blank(cx, files);
        assert_eq!(dock_tab(cx, &s), (true, tab));
    }
}

#[test]
fn an_unknown_or_missing_dock_loads_as_closed() {
    let unknown: Session = serde_json::from_str(r#"{"open": [], "dock": "sidebar"}"#).unwrap();
    assert_eq!(unknown.dock, None);
    let old: Session = serde_json::from_str(r#"{"open": []}"#).unwrap();
    assert_eq!(old.dock, None);
    let kept: Session = serde_json::from_str(r#"{"dock": "network"}"#).unwrap();
    assert_eq!(kept.dock, Some(DockTab::Network));
}
