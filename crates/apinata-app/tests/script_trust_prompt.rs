//! The per-collection script question: asked once before the first send that needs scripts,
//! answered by the user, changeable in collection settings and silenced by Preferences.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::command::Command;
use apinata_app::config_editor::ConfigSection;
use apinata_app::response_pane::ResponsePane;
use apinata_app::runner_model::{RunState, RunnerModel};
use apinata_app::script_gate::{Gate, approved_only, gate};
use apinata_app::script_gate::{trust_question, trust_question_about};
use apinata_app::settings::RunScripts;
use apinata_app::tabs::TabKind;
use apinata_core::{
    ScriptDecision, ScriptOptions, ScriptPhase, ScriptTextKind, ScriptTrust, script_hash,
};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext, WindowOptions};
use std::collections::BTreeSet;
use support::*;

const CODE: &str = "bru.setVar('a', 1)";

fn scripted() -> tempfile::TempDir {
    let a = format!(
        "{}runtime:\n  scripts:\n    - type: before-request\n      code: \"{CODE}\"\n",
        request("a", "http://127.0.0.1:9/ok")
    );
    write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)])
}

fn trust(s: &Started) -> ScriptTrust {
    ScriptTrust::load(s.files.path().join("script-trust.json")).unwrap()
}

fn start_send(
    cx: &mut TestAppContext,
    s: &Started,
) -> gpui_kit::Entity<apinata_app::send_model::SendModel> {
    let model = sender(cx, &s.ws);
    model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx));
    model
}

fn asked(cx: &mut TestAppContext, s: &Started) -> bool {
    cx.run_until_parked();
    frame(cx, s.win);
    present(cx, s.win, "script-trust-run")
}

fn click(cx: &mut TestAppContext, s: &Started, id: &'static str) {
    cx.update_window(s.win, |_, w, cx| w.click(id, cx)).unwrap();
    frame(cx, s.win);
}

#[test]
fn the_gate_asks_only_for_an_undecided_collection_that_needs_scripts() {
    use ScriptDecision::{Allowed, Blocked};
    assert_eq!(gate(RunScripts::Ask, None, true), Gate::Ask);
    assert_eq!(gate(RunScripts::Ask, None, false), Gate::Run);
    assert_eq!(gate(RunScripts::Ask, Some(Allowed), true), Gate::Run);
    assert_eq!(gate(RunScripts::Ask, Some(Blocked), true), Gate::Skip);
    assert_eq!(gate(RunScripts::Never, Some(Allowed), true), Gate::Skip);
    assert_eq!(gate(RunScripts::Never, None, true), Gate::Skip);
}

#[test]
fn undecided_scripts_all_approved_run_only_from_the_approved_set() {
    let on = ScriptOptions::new().enabled(true);
    let approved: BTreeSet<String> = [script_hash("a"), script_hash("b")].into();
    let all = [script_hash("a"), script_hash("b")];
    let got = approved_only(&all, &approved, on.clone()).unwrap();
    assert_eq!(got, on.clone().only_approved(approved.clone()));
    // A script outside the set would not run under these options.
    assert_ne!(got, on.clone());
    let one_new = [script_hash("a"), script_hash("c")];
    assert!(approved_only(&one_new, &approved, on).is_none());
}

#[gpui_kit::test]
fn the_first_send_asks_and_enter_does_not_run(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = scripted();
    let s = started(cx, &col);
    let model = start_send(cx, &s);
    wait_real(cx, |cx| asked(cx, &s));
    cx.simulate_keystrokes(s.win, "enter");
    let result = finished(cx, &model);
    assert_eq!(trust(&s).decision(&s.root), None);
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.to_string().contains("script")),
        "{:?}",
        result.warnings
    );
}

#[gpui_kit::test]
fn run_scripts_allows_approves_and_is_not_asked_again(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = scripted();
    let s = started(cx, &col);
    let model = start_send(cx, &s);
    wait_real(cx, |cx| asked(cx, &s));
    click(cx, &s, "script-trust-run");
    finished(cx, &model);
    let store = trust(&s);
    assert_eq!(store.decision(&s.root), Some(ScriptDecision::Allowed));
    assert!(store.is_approved(&s.root, &script_hash(CODE)));
    let again = start_send(cx, &s);
    finished(cx, &again);
    assert!(!asked(cx, &s));
}

#[gpui_kit::test]
fn preferences_never_skips_the_question(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = scripted();
    let s = started(cx, &col);
    cx.update(|cx| {
        let settings = cx.global::<AppState>().settings.clone();
        settings.update(cx, |p, cx| {
            p.update(cx, |s| s.run_scripts = RunScripts::Never)
        });
    });
    let model = start_send(cx, &s);
    finished(cx, &model);
    assert!(!asked(cx, &s));
    assert_eq!(trust(&s).decision(&s.root), None);
}

#[gpui_kit::test]
fn collection_settings_change_the_answer(cx: &mut TestAppContext) {
    let col = scripted();
    let s = started(cx, &col);
    let tab =
        s.ws.update(cx, |w, cx| {
            w.open_settings(&s.root, TabKind::Collection, cx)
        })
        .unwrap();
    frame(cx, s.win);
    let ed = s
        .shell
        .read_with(cx, |v, cx| {
            v.tab_area().read(cx).config_editor(tab).cloned()
        })
        .unwrap();
    let ix = ed.read_with(cx, |e, _| {
        e.sections()
            .iter()
            .position(|x| *x == ConfigSection::Scripts)
    });
    cx.update_window(s.win, |_, w, cx| {
        w.click(("config-section", ix.unwrap()), cx)
    })
    .unwrap();
    frame(cx, s.win);
    assert!(present(cx, s.win, "config-scripts"));
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.within("script-decision").click(1, cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(trust(&s).decision(&s.root), Some(ScriptDecision::Allowed));
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.within("script-decision").click(0, cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(trust(&s).decision(&s.root), None);
}

fn label(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> Option<String> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

#[test]
fn the_question_names_the_risk_and_defaults_to_dont_run() {
    let q = trust_question();
    assert_eq!(q.title.as_ref(), "Run scripts from this collection?");
    assert_eq!(
        q.detail.as_deref(),
        Some(
            "These scripts run on your computer and can read this collection's variables, \
             including secrets. Run them only if you trust where this collection came from."
        )
    );
    let labels: Vec<&str> = q.choices.iter().map(|c| c.label.as_ref()).collect();
    assert_eq!(
        labels,
        ["Run scripts", "Never for this collection", "Don't run"]
    );
    assert_eq!(q.choices[1].id, "script-trust-never");
    assert_eq!(q.choices[2].id, "script-trust-skip");
    assert_eq!((q.default, q.cancel), (2, 2));
}

#[gpui_kit::test]
fn the_pane_waits_without_a_timer_while_the_question_is_open(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = scripted();
    let s = started(cx, &col);
    let model = sender(cx, &s.ws);
    // `started` already inits; the pane's own window stays on the reduced-motion path.
    support::init(cx);
    let m = model.clone();
    let (pane, _view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(m, w, cx))
            })
        })
        .unwrap();
    model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx));
    wait_real(cx, |cx| asked(cx, &s));
    model.read_with(cx, |m, _| {
        assert!(m.is_sending() && m.is_waiting());
        assert_eq!(m.elapsed(), None);
    });
    assert_eq!(
        label(cx, pane, "response-waiting").as_deref(),
        Some("Waiting for your answer")
    );
    assert!(!present(cx, pane, "response-elapsed"));
    assert!(!present(cx, pane, "response-spinner"));
    cx.simulate_keystrokes(s.win, "enter");
    finished(cx, &model);
    assert!(!model.read_with(cx, |m, _| m.is_waiting()));
    assert!(!present(cx, pane, "response-waiting"));
}

#[gpui_kit::test]
fn escape_skips_scripts_and_ends_the_wait(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = scripted();
    let s = started(cx, &col);
    let model = start_send(cx, &s);
    wait_real(cx, |cx| asked(cx, &s));
    assert!(model.read_with(cx, |m, _| m.is_waiting()));
    cx.simulate_keystrokes(s.win, "escape");
    let result = finished(cx, &model);
    assert!(!model.read_with(cx, |m, _| m.is_waiting()));
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.to_string().contains("script")),
        "{:?}",
        result.warnings
    );
}

#[gpui_kit::test]
fn a_run_waits_on_the_question_while_planning(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = scripted();
    let s = started(cx, &col);
    let (ws, root) = (s.ws.clone(), s.root.clone());
    let runner = cx.new(|_| RunnerModel::new(ws, root, apinata_core::ItemPath::root()));
    assert!(runner.update(cx, |m, cx| m.start(cx)));
    wait_real(cx, |cx| asked(cx, &s));
    runner.read_with(cx, |m, _| {
        assert!(m.is_waiting());
        assert!(matches!(m.state(), RunState::Planning));
    });
    cx.simulate_keystrokes(s.win, "enter");
    wait_real(cx, |cx| runner.read_with(cx, |m, _| !m.is_running()));
    assert!(!runner.read_with(cx, |m, _| m.is_waiting()));
}

// Creates `Fresh` through the switcher's New collection entry and waits until it is open.
pub(crate) fn create_collection(
    cx: &mut TestAppContext,
    s: &Started,
) -> (tempfile::TempDir, std::path::PathBuf) {
    let parent = tempfile::tempdir().unwrap();
    let fresh = parent.path().join("Fresh");
    switcher_click(cx, s.win, "switcher-new-collection");
    let answer = fresh.clone();
    cx.simulate_new_path_selection(move |_| Some(answer));
    wait_real(cx, |cx| {
        s.ws.read_with(cx, |w, _| {
            w.collections().iter().any(|c| c.root() == fresh.as_path())
        })
    });
    (parent, fresh)
}

#[gpui_kit::test]
fn a_collection_made_in_the_app_is_not_trusted_wholesale(cx: &mut TestAppContext) {
    let col = scripted();
    let s = started(cx, &col);
    let (_parent, fresh) = create_collection(cx, &s);
    let store = trust(&s);
    assert_eq!(store.decision(&fresh), None);
    assert!(store.approved(&fresh).is_empty(), "nothing is approved");
    assert_eq!(
        store.decision(&s.root),
        None,
        "an opened folder stays undecided"
    );
}

#[gpui_kit::test]
fn a_broken_trust_file_still_opens_the_new_collection(cx: &mut TestAppContext) {
    let col = scripted();
    let s = started_with(cx, &col, |p| {
        std::fs::write(&p.script_trust, "not json").unwrap()
    });
    let (_parent, _fresh) = create_collection(cx, &s);
    let kept = std::fs::read_to_string(s.files.path().join("script-trust.json")).unwrap();
    assert_eq!(
        kept, "not json",
        "a store that fails to load is never overwritten"
    );
}

// Writes `a.yml` into the created collection from outside the app and waits for the reload.
pub(crate) fn add_request(
    cx: &mut TestAppContext,
    s: &Started,
    fresh: &std::path::Path,
    yaml: &str,
) {
    std::fs::write(fresh.join("a.yml"), yaml).unwrap();
    // `app::start` installs no file watcher, so a reload brings the new file in.
    s.ws.update(cx, |w, cx| w.send(fresh, Command::ReloadAll, cx));
    wait_real(cx, |cx| {
        ws_snap(cx, &s.ws, fresh)
            .content
            .requests
            .contains_key(&ip("a.yml"))
    });
}

#[gpui_kit::test]
fn a_script_written_outside_the_app_in_a_created_collection_is_asked(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = scripted();
    let s = started(cx, &col);
    let (_parent, fresh) = create_collection(cx, &s);
    let a = format!(
        "{}runtime:\n  scripts:\n    - type: before-request\n      code: \"{CODE}\"\n",
        request("a", "http://127.0.0.1:9/ok")
    );
    add_request(cx, &s, &fresh, &a);
    let model = sender(cx, &s.ws);
    model.update(cx, |m, cx| m.send(&fresh, ip("a.yml"), cx));
    wait_real(cx, |cx| asked(cx, &s));
    cx.simulate_keystrokes(s.win, "escape");
    finished(cx, &model);
    assert_eq!(trust(&s).decision(&fresh), None);
}

#[gpui_kit::test]
fn the_default_dont_run_sits_rightmost_and_enter_still_means_dont_run(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = scripted();
    let s = started(cx, &col);
    let model = start_send(cx, &s);
    wait_real(cx, |cx| asked(cx, &s));
    cx.update_window(s.win, |_, w, _| {
        let footer = w.within("confirm-dialog").find("dialog-footer").bounds();
        let run = w.find("script-trust-run").bounds();
        let never = w.find("script-trust-never").bounds();
        let skip = w.find("script-trust-skip").bounds();
        assert!(run.right() < never.left(), "Never sits right of Run");
        assert!(never.right() < skip.left(), "Don't run sits rightmost");
        assert_eq!(skip.right(), footer.right());
    })
    .unwrap();
    cx.simulate_keystrokes(s.win, "enter");
    finished(cx, &model);
    assert_eq!(trust(&s).decision(&s.root), None);
}

#[gpui_kit::test]
fn escape_stores_nothing_and_the_next_send_asks_again(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = scripted();
    let s = started(cx, &col);
    let model = start_send(cx, &s);
    wait_real(cx, |cx| asked(cx, &s));
    cx.simulate_keystrokes(s.win, "escape");
    finished(cx, &model);
    assert_eq!(trust(&s).decision(&s.root), None);
    let again = start_send(cx, &s);
    wait_real(cx, |cx| asked(cx, &s));
    cx.simulate_keystrokes(s.win, "escape");
    finished(cx, &again);
}

#[gpui_kit::test]
fn dont_run_skips_this_send_and_stores_nothing(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = scripted();
    let s = started(cx, &col);
    let model = start_send(cx, &s);
    wait_real(cx, |cx| asked(cx, &s));
    click(cx, &s, "script-trust-skip");
    let result = finished(cx, &model);
    assert_eq!(trust(&s).decision(&s.root), None);
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.to_string().contains("script"))
    );
    let again = start_send(cx, &s);
    wait_real(cx, |cx| asked(cx, &s));
    cx.simulate_keystrokes(s.win, "escape");
    finished(cx, &again);
}

#[gpui_kit::test]
fn never_blocks_the_collection_and_later_sends_skip_without_asking(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = scripted();
    let s = started(cx, &col);
    let model = start_send(cx, &s);
    wait_real(cx, |cx| asked(cx, &s));
    click(cx, &s, "script-trust-never");
    finished(cx, &model);
    assert_eq!(trust(&s).decision(&s.root), Some(ScriptDecision::Blocked));
    let again = start_send(cx, &s);
    let result = finished(cx, &again);
    assert!(!asked(cx, &s));
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.to_string().contains("script"))
    );
}

#[gpui_kit::test]
fn the_not_executed_notice_offers_ask_again_for_a_blocked_collection(cx: &mut TestAppContext) {
    http_ready(cx);
    // A live server, so the pane shows the response with its warnings above the tabs.
    let server = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let a = format!(
        "{}runtime:\n  scripts:\n    - type: before-request\n      code: \"{CODE}\"\n",
        request("a", &format!("{}/ok", server.uri()))
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let s = started(cx, &col);
    let model = sender(cx, &s.ws);
    support::init(cx);
    let m = model.clone();
    let (win, _view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(m, w, cx))
            })
        })
        .unwrap();
    ScriptTrust::update(s.files.path().join("script-trust.json"), |t| {
        t.set_decision(&s.root, Some(ScriptDecision::Blocked))
    })
    .unwrap();
    refresh_approvals(cx);
    model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx));
    finished(cx, &model);
    assert!(present(cx, win, "response-warning-ask-again"));
    cx.update_window(win, |_, w, cx| w.click("response-warning-ask-again", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(trust(&s).decision(&s.root), None);
    assert!(!present(cx, win, "response-warning-ask-again"));
}

// Tells the approvals model its trust file changed, as the file watcher would.
fn refresh_approvals(cx: &mut TestAppContext) {
    let approvals = cx.update(|cx| cx.global::<AppState>().approvals.clone());
    approvals.update(cx, |a, cx| a.refresh(cx));
    cx.run_until_parked();
}

fn blocked_notice(cx: &mut TestAppContext) -> (Started, AnyWindowHandle, tempfile::TempDir) {
    http_ready(cx);
    let server = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let a = format!(
        "{}runtime:\n  scripts:\n    - type: before-request\n      code: \"{CODE}\"\n",
        request("a", &format!("{}/ok", server.uri()))
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let s = started(cx, &col);
    let model = sender(cx, &s.ws);
    support::init(cx);
    let m = model.clone();
    let (win, _view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(m, w, cx))
            })
        })
        .unwrap();
    ScriptTrust::update(s.files.path().join("script-trust.json"), |t| {
        t.set_decision(&s.root, Some(ScriptDecision::Blocked))
    })
    .unwrap();
    refresh_approvals(cx);
    model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx));
    finished(cx, &model);
    (s, win, col)
}

#[gpui_kit::test]
fn the_notice_renders_from_memory_not_from_the_trust_file(cx: &mut TestAppContext) {
    let (s, win, _col) = blocked_notice(cx);
    assert!(present(cx, win, "response-warning-ask-again"));
    // The file changes with no event: a render that read it would drop the button.
    ScriptTrust::update(s.files.path().join("script-trust.json"), |t| {
        t.set_decision(&s.root, None)
    })
    .unwrap();
    frame(cx, win);
    assert!(present(cx, win, "response-warning-ask-again"));
    // Once the model re-reads, the notice follows.
    refresh_approvals(cx);
    frame(cx, win);
    assert!(!present(cx, win, "response-warning-ask-again"));
}

#[gpui_kit::test]
fn ask_again_updates_the_approvals_model_and_the_settings_view(cx: &mut TestAppContext) {
    let (s, win, _col) = blocked_notice(cx);
    let file = s.files.path().join("script-trust.json");
    let view = cx.update(|cx| {
        let root = s.root.clone();
        cx.new(|cx| apinata_app::script_settings::ScriptSettingsView::new(file, root, cx))
    });
    assert_eq!(
        view.read_with(cx, |v, _| v.decision()),
        Some(ScriptDecision::Blocked)
    );
    cx.update_window(win, |_, w, cx| w.click("response-warning-ask-again", cx))
        .unwrap();
    cx.run_until_parked();
    // Neither a manual reload nor a watcher event ran.
    let approvals = cx.update(|cx| cx.global::<AppState>().approvals.clone());
    assert_eq!(
        approvals.read_with(cx, |a, _| a.script_decision(&s.root)),
        None
    );
    assert_eq!(view.read_with(cx, |v, _| v.decision()), None);
}

#[test]
fn settings_options_label_the_stored_block_never() {
    assert_eq!(
        apinata_app::script_settings::choice_labels(),
        ["Ask", "Run scripts", "Never"]
    );
}

// Texts that are only assertion or variable expressions name what runs and show it, while the
// choices, default and what Run stores stay those of the script question.
#[test]
fn the_question_names_an_assertion_or_variable_when_no_script_needs_running() {
    let right = ScriptTextKind::AssertionRight;
    let q = trust_question_about(false, [(&right, "`${1+1}`")]);
    assert_eq!(q.title.as_ref(), "This assertion runs JavaScript");
    let detail = q.detail.as_deref().unwrap();
    assert!(detail.contains("`${1+1}`"), "{detail}");
    assert!(detail.contains("secrets"), "{detail}");
    let plain = trust_question();
    let parts = |q: &apinata_app::confirm::Ask| {
        let c: Vec<(&str, String)> = q
            .choices
            .iter()
            .map(|c| (c.id, c.label.to_string()))
            .collect();
        (c, q.default, q.cancel)
    };
    assert_eq!(parts(&q), parts(&plain));
    let var = ScriptTextKind::Variable { name: "n".into() };
    let q = trust_question_about(false, [(&var, "1 + 1")]);
    assert_eq!(q.title.as_ref(), "This variable runs JavaScript");
    let left = ScriptTextKind::AssertionLeft;
    let q = trust_question_about(false, [(&left, "a()"), (&right, "b()")]);
    assert_eq!(q.title.as_ref(), "These assertions run JavaScript");
    let q = trust_question_about(false, [(&left, "a()"), (&var, "b()")]);
    assert_eq!(
        q.title.as_ref(),
        "These assertions and variables run JavaScript"
    );
    let scratch = trust_question_about(true, [(&right, "`${1+1}`")]);
    assert!(!scratch.detail.as_deref().unwrap().contains("collection"));
    // Any script among the texts keeps the script question.
    let script = ScriptTextKind::Script {
        phase: ScriptPhase::PreRequest,
        level: "request".into(),
    };
    let q = trust_question_about(false, [(&right, "`${1+1}`"), (&script, CODE)]);
    assert_eq!(q.title.as_ref(), plain.title.as_ref());
    assert_eq!(q.detail, plain.detail);
}

#[gpui_kit::test]
fn an_assertion_value_that_runs_javascript_is_named_in_the_question(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = format!(
        "{}runtime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"`${{1+1}}`\"\n",
        request("a", "http://127.0.0.1:9/ok")
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let s = started(cx, &col);
    let model = start_send(cx, &s);
    wait_real(cx, |cx| asked(cx, &s));
    assert_eq!(
        label(cx, s.win, "confirm-dialog").as_deref(),
        Some("This assertion runs JavaScript")
    );
    let detail = label(cx, s.win, "confirm-detail").unwrap_or_default();
    assert!(detail.contains("`${1+1}`"), "{detail}");
    cx.simulate_keystrokes(s.win, "enter");
    finished(cx, &model);
    assert_eq!(trust(&s).decision(&s.root), None);
}
