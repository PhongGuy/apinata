//! A refresh of a field from the model (after a save, or a reveal) leaves an open IME
//! composition and its caret alone, and committing it lands the composed text in the model.

use crate::support;

use apinata_app::request_editor::{RequestEditor, Section};
use apinata_core::{BodySpec, ScriptPhase};
use gpui_kit::component::input::{EditorState, InputState};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, EntityInputHandler, TestAppContext};
use std::ops::Range;
use support::*;

const TEXT_BODY: &str = "info:\n  name: t\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: \"http://x/t\"\n  body:\n    type: text\n    data: abc\n";
const GRAPHQL: &str = "info:\n  name: g\n  type: graphql\n  seq: 1\ngraphql:\n  method: POST\n  url: \"http://x/g\"\n  body:\n    query: \"{ users }\"\n    variables: \"{}\"\n";
const SCRIPTED: &str = "info:\n  name: s\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://x/s\"\nruntime:\n  scripts:\n    - type: before-request\n      code: \"let a = 1\"\n";

// The input states a field can be, so one helper reads plain inputs and code editors alike.
trait Composable: EntityInputHandler {
    fn shown(&self) -> String;
    fn caret(&self) -> Range<usize>;
}

impl Composable for InputState {
    fn shown(&self) -> String {
        self.value().to_string()
    }
    fn caret(&self) -> Range<usize> {
        self.selected_range()
    }
}

impl Composable for EditorState {
    fn shown(&self) -> String {
        self.value().to_string()
    }
    fn caret(&self) -> Range<usize> {
        self.selected_range()
    }
}

// A field's text, IME marked range (UTF-16) and selection.
type Field = (String, Option<Range<usize>>, Range<usize>);

fn field<S: Composable>(cx: &mut TestAppContext, win: AnyWindowHandle, input: &Entity<S>) -> Field {
    cx.update_window(win, |_, w, cx| {
        input.update(cx, |s, cx| {
            (s.shown(), s.marked_text_range(w, cx), s.caret())
        })
    })
    .unwrap()
}

// Opens a two-character preedit at the caret with the IME caret between them, runs `refresh`,
// and returns the field before and after it.
fn compose_across<S: Composable>(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    input: &Entity<S>,
    refresh: impl FnOnce(&mut TestAppContext),
) -> [Field; 2] {
    cx.update_window(win, |_, w, cx| {
        input.update(cx, |s, cx| {
            s.replace_and_mark_text_in_range(None, "かな", Some(1..1), w, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    let before = field(cx, win, input);
    assert!(before.1.is_some(), "the composition opened");
    refresh(cx);
    [before, field(cx, win, input)]
}

// Commits the open composition as it reads.
fn commit<S: Composable>(cx: &mut TestAppContext, win: AnyWindowHandle, input: &Entity<S>) {
    cx.update_window(win, |_, w, cx| {
        input.update(cx, |s, cx| s.replace_text_in_range(None, "かな", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

struct Rig {
    _col: tempfile::TempDir,
    _state: tempfile::TempDir,
    ws: Entity<apinata_app::workspace::Workspace>,
    root: std::path::PathBuf,
    win: AnyWindowHandle,
    ed: Entity<RequestEditor>,
    rel: &'static str,
}

fn rig(cx: &mut TestAppContext, rel: &'static str, text: &str) -> Rig {
    support::init(cx);
    let col = write_tree(&[("opencollection.yml", ROOT), (rel, text)]);
    let (state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, rel, false);
    Rig {
        _col: col,
        _state: state,
        ws,
        root,
        win,
        ed,
        rel,
    }
}

impl Rig {
    // Shows `section`, clicks into its code editor `id` within `scope`, and types `new1` at
    // the end, one keystroke per character.
    fn type_in_code(
        &self,
        cx: &mut TestAppContext,
        section: Section,
        scope: &'static str,
        id: &'static str,
    ) {
        cx.update_window(self.win, |_, w, cx| {
            w.click(("section", section as usize), cx)
        })
        .unwrap();
        cx.run_until_parked();
        frame(cx, self.win);
        cx.update_window(self.win, |_, w, cx| {
            w.within(scope).click_at(id, code_at(), cx);
            w.press("end", cx);
        })
        .unwrap();
        cx.run_until_parked();
        for ch in "new1".chars() {
            cx.simulate_keystrokes(self.win, &ch.to_string());
            cx.run_until_parked();
        }
    }

    fn save(&self, cx: &mut TestAppContext) {
        cx.update_window(self.win, |_, w, cx| {
            self.ed.update(cx, |e, cx| e.save(w, cx))
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn view(&self, cx: &mut TestAppContext) -> std::sync::Arc<apinata_app::snapshot::RequestView> {
        view_of(cx, &self.ws, &self.root, self.rel)
    }

    // Composes across a save in `code`, checks the field kept the composition, and commits it.
    fn compose_across_a_save(
        &self,
        cx: &mut TestAppContext,
        code: &Entity<EditorState>,
        want: &str,
    ) {
        let [before, after] = compose_across(cx, self.win, code, |cx| self.save(cx));
        assert_eq!(before.0, want);
        assert_eq!(after, before);
        commit(cx, self.win, code);
        assert_eq!(code.read_with(cx, |s, _| s.shown()), want);
    }
}

#[gpui_kit::test]
fn a_save_leaves_an_open_composition_in_a_text_body_alone(cx: &mut TestAppContext) {
    let r = rig(cx, "t.yml", TEXT_BODY);
    r.type_in_code(cx, Section::Body, "body", "body-code");
    let code = r.ed.read_with(cx, |e, cx| e.body().read(cx).code().clone());
    r.compose_across_a_save(cx, &code, "abcnew1かな");
    assert_eq!(r.view(cx).body, BodySpec::Text("abcnew1かな".into()));
}

// The GraphQL query or variables text the model holds.
fn graphql_text(r: &Rig, cx: &mut TestAppContext, variables: bool) -> String {
    match r.view(cx).body.clone() {
        BodySpec::Graphql {
            query,
            variables: v,
        } => {
            if variables {
                v
            } else {
                query
            }
        }
        other => panic!("{other:?}"),
    }
}

#[gpui_kit::test]
fn a_save_leaves_an_open_composition_in_a_graphql_query_alone(cx: &mut TestAppContext) {
    let r = rig(cx, "g.yml", GRAPHQL);
    r.type_in_code(cx, Section::Body, "body", "graphql-query");
    let code =
        r.ed.read_with(cx, |e, cx| e.graphql().read(cx).query().clone());
    r.compose_across_a_save(cx, &code, "{ users }new1かな");
    assert_eq!(graphql_text(&r, cx, false), "{ users }new1かな");
}

#[gpui_kit::test]
fn a_save_leaves_an_open_composition_in_graphql_variables_alone(cx: &mut TestAppContext) {
    let r = rig(cx, "g.yml", GRAPHQL);
    r.type_in_code(cx, Section::Body, "body", "graphql-variables");
    let code =
        r.ed.read_with(cx, |e, cx| e.graphql().read(cx).variables().clone());
    r.compose_across_a_save(cx, &code, "{}new1かな");
    assert_eq!(graphql_text(&r, cx, true), "{}new1かな");
}

#[gpui_kit::test]
fn a_save_leaves_an_open_composition_in_a_script_alone(cx: &mut TestAppContext) {
    let r = rig(cx, "s.yml", SCRIPTED);
    r.type_in_code(cx, Section::Script, "script", "script-pre-request-code");
    let code = r.ed.read_with(cx, |e, cx| {
        e.script()
            .read(cx)
            .code(ScriptPhase::PreRequest)
            .unwrap()
            .clone()
    });
    r.compose_across_a_save(cx, &code, "let a = 1new1かな");
    assert_eq!(r.view(cx).scripts.pre_request, "let a = 1new1かな");
}

#[gpui_kit::test]
fn a_save_leaves_an_open_composition_in_the_docs_alone(cx: &mut TestAppContext) {
    let r = rig(cx, "t.yml", TEXT_BODY);
    r.type_in_code(cx, Section::Docs, "docs", "docs-code");
    let code = r.ed.read_with(cx, |e, cx| e.docs().read(cx).code().clone());
    r.compose_across_a_save(cx, &code, "new1かな");
    assert_eq!(r.view(cx).docs, "new1かな");
}

#[gpui_kit::test]
fn a_save_leaves_an_open_composition_in_the_presets_url_alone(cx: &mut TestAppContext) {
    use apinata_app::config_editor::{ConfigEditor, ConfigSection, ConfigTarget};
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let collection = format!(
        "{ROOT}extensions:\n  bruno:\n    presets:\n      request:\n        type: http\n        url: https://old.test\n"
    );
    let col = write_tree(&[("opencollection.yml", collection.as_str())]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (ws2, r) = (ws.clone(), root.clone());
    let (win, ed) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ConfigEditor::new(ws2, r, ConfigTarget::Collection, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    ed.update(cx, |e, cx| e.set_section(ConfigSection::Presets, cx));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.click("presets-url", cx);
        w.press("end", cx);
    })
    .unwrap();
    cx.run_until_parked();
    for ch in "new1".chars() {
        cx.simulate_keystrokes(win, &ch.to_string());
        cx.run_until_parked();
    }
    let input = ed.read_with(cx, |e, cx| {
        e.presets().unwrap().read(cx).url_input().clone()
    });
    let [before, after] = compose_across(cx, win, &input, |cx| {
        ed.update(cx, |e, cx| e.save(cx));
        cx.run_until_parked();
    });
    assert_eq!(before.0, "https://old.testnew1かな");
    assert_eq!(after, before);
    commit(cx, win, &input);
    let snap = ws.read_with(cx, |w, cx| w.snapshot(&root, cx)).unwrap();
    let presets = snap.content.collection.presets.clone().unwrap();
    assert_eq!(presets.request_url, "https://old.testnew1かな");
}

// Revealing a masked row whose value is empty forces its field to resync with unchanged text,
// so a composition typed into the field survives and Enter saves the composed text.
#[gpui_kit::test]
fn a_reveal_leaves_an_open_composition_in_a_runtime_value_alone(cx: &mut TestAppContext) {
    use apinata_app::runtime_vars::open_runtime_vars;
    use apinata_core::{EnvOverlay, RuntimeVars};
    support::init(cx);
    let vars = RuntimeVars::new();
    vars.set_text("tok", "");
    let (win, _) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |_, cx| {
                cx.new(|_| Blank)
            })
        })
        .unwrap();
    let v = vars.clone();
    let view = cx
        .update_window(win, |_, w, cx| {
            open_runtime_vars(v, EnvOverlay::new(), vec!["tok".into()], Vec::new(), w, cx)
        })
        .unwrap();
    frame(cx, win);
    // Focus events carry empty paths while the window is inactive, so no blur would fire.
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click(("runtime-var-value", 0usize), cx))
        .unwrap();
    frame(cx, win);
    let input = view.read_with(cx, |v, _| v.captured_input(0).unwrap().clone());
    let [before, after] = compose_across(cx, win, &input, |cx| {
        cx.update_window(win, |_, w, cx| w.click(("runtime-var-reveal", 0usize), cx))
            .unwrap();
        frame(cx, win);
    });
    assert!(present(cx, win, ("runtime-var-hide", 0usize)), "revealed");
    assert_eq!(before.0, "かな");
    assert_eq!(after, before);
    commit(cx, win, &input);
    cx.update_window(win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    frame(cx, win);
    let stored: Vec<_> = (vars.entries().into_iter())
        .map(|v| (v.name().to_owned(), v.value().map(str::to_owned)))
        .collect();
    assert_eq!(stored, [("tok".to_owned(), Some("かな".to_owned()))]);
}

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

// The composition is cut by UTF-16 units; a range that does not land on character boundaries
// (one splitting a surrogate pair) or does not fit leaves the value whole.
#[test]
fn the_committed_text_cuts_the_composition_at_any_position() {
    use apinata_app::ui::committed_text;
    assert_eq!(committed_text("かなabc", Some(0..2)), "abc");
    assert_eq!(committed_text("abcかな", Some(3..5)), "abc");
    assert_eq!(committed_text("かな", Some(0..2)), "");
    assert_eq!(committed_text("a😀b", Some(1..3)), "ab");
    assert_eq!(committed_text("a😀b", Some(2..4)), "a😀b");
    assert_eq!(committed_text("a😀b", Some(1..2)), "a😀b");
    assert_eq!(committed_text("a😀", Some(2..3)), "a😀");
    assert_eq!(committed_text("abc", Some(1..1)), "abc");
}
