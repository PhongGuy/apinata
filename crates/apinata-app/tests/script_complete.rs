//! Script editors: JavaScript highlighting and completion of the bru, req and res members.

use crate::support;

use apinata_app::script_complete::{BRU, REQ, RES, items};
use apinata_app::script_section::ScriptSection;
use apinata_core::ScriptPhase::{PostResponse, PreRequest, Tests};
use gpui_kit::AppContext as _;
use gpui_kit::component::Rope;
use lsp_types::{CompletionTextEdit, Position, Range, TextEdit};
use support::*;

fn labels(text: &str) -> Vec<String> {
    let mut l: Vec<_> = items(&Rope::from(text), text.len())
        .into_iter()
        .map(|i| i.label)
        .collect();
    l.sort();
    l
}

#[test]
fn members_complete_after_the_object_and_its_prefix() {
    assert_eq!(labels("x = bru.getEnv"), ["getEnvName", "getEnvVar"]);
    assert_eq!(
        labels("res.getS"),
        ["getSize", "getStatus", "getStatusText"]
    );
    assert!(labels("req.").contains(&"setHeader".to_owned()));
    assert_eq!(labels("req.").len(), REQ.len());
}

#[test]
fn the_edit_replaces_only_the_typed_prefix() {
    let text = "x = bru.getEnvN";
    let all = items(&Rope::from(text), text.len());
    assert_eq!(all.len(), 1);
    assert_eq!(
        all[0].text_edit,
        Some(CompletionTextEdit::Edit(TextEdit {
            range: Range::new(Position::new(0, 8), Position::new(0, 15)),
            new_text: "getEnvName".into(),
        }))
    );
}

#[test]
fn other_objects_and_deeper_paths_get_nothing() {
    assert!(labels("mybru.get").is_empty());
    assert!(labels("foo.").is_empty());
    assert!(labels("bru.runner.").is_empty());
    assert!(labels("bru").is_empty());
}

#[test]
fn bru_methods_are_offered() {
    for name in [
        "runRequest",
        "getOauth2CredentialVar",
        "resetOauth2Credential",
    ] {
        assert!(BRU.contains(&name), "{name}");
    }
    for name in ["disableParsingResponseJson", "onFail"] {
        assert!(!REQ.contains(&name), "{name}");
    }
}

// Every offered member exists in the runtime's JavaScript, so the lists cannot drift from it.
#[test]
fn every_member_is_defined_by_the_script_runtime() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../apinata-core/src/script");
    let mut js = String::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "js") {
            js.push_str(&std::fs::read_to_string(path).unwrap());
        }
    }
    for (object, members) in [("bru", BRU), ("req", REQ), ("res", RES)] {
        for m in members {
            let defined = [
                format!("{m}:"),
                format!("{m},"),
                format!("{object}.{m} ="),
                format!("field(\"{m}\""),
            ]
            .iter()
            .any(|p| js.contains(p.as_str()));
            assert!(defined, "{object}.{m} is not defined in the script runtime");
        }
    }
}

#[gpui_kit::test]
fn script_and_test_editors_are_javascript_with_completion(cx: &mut gpui_kit::TestAppContext) {
    for phases in [&[PreRequest, PostResponse][..], &[Tests][..]] {
        let (_win, host) = host_section(cx, |w, cx| ScriptSection::new(phases, w, cx));
        let s = host.read_with(cx, |h, _| h.view.clone());
        for &phase in phases {
            let code = s.read_with(cx, |s, _| s.code(phase).cloned().expect("an editor"));
            code.read_with(cx, |c, _| {
                assert_eq!(c.language_name(), "javascript");
                assert!(c.lsp().completion_provider.is_some());
            });
        }
    }
}

#[test]
fn a_member_typed_in_full_offers_nothing() {
    assert!(labels("res.status").is_empty());
}

// The pre-request editor, focused and empty.
fn focused_script_editor(
    cx: &mut gpui_kit::TestAppContext,
) -> (
    gpui_kit::AnyWindowHandle,
    gpui_kit::Entity<gpui_kit::component::input::EditorState>,
) {
    use gpui_kit::test::TestWindowExt as _;
    let (win, host) = host_section(cx, |w, cx| ScriptSection::new(&[PreRequest], w, cx));
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| {
        w.click_at("script-pre-request-code", code_at(), cx);
        w.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    let code = s.read_with(cx, |s, _| s.code(PreRequest).cloned().expect("an editor"));
    (win, code)
}

// Type `text`, then press Enter as a platform keystroke, which also carries a newline whenever
// the bound action lets the key propagate.
fn type_then_enter(cx: &mut gpui_kit::TestAppContext, text: &str) -> String {
    use gpui_kit::test::TestWindowExt as _;
    let (win, code) = focused_script_editor(cx);
    for ch in text.chars() {
        cx.update_window(win, |_, w, cx| {
            w.input(&ch.to_string(), cx);
            w.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.dispatch_keystroke(gpui_kit::Keystroke::parse("enter").unwrap(), cx);
        w.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    code.read_with(cx, |c, _| c.value().to_string())
}

#[gpui_kit::test]
fn enter_accepts_a_member_without_a_newline(cx: &mut gpui_kit::TestAppContext) {
    assert_eq!(type_then_enter(cx, "res.getSi"), "res.getSize");
}

#[gpui_kit::test]
fn enter_after_a_full_member_breaks_the_line(cx: &mut gpui_kit::TestAppContext) {
    assert_eq!(type_then_enter(cx, "res.body"), "res.body\n");
}

#[gpui_kit::test]
fn enter_after_other_text_breaks_the_line(cx: &mut gpui_kit::TestAppContext) {
    assert_eq!(type_then_enter(cx, "a.b"), "a.b\n");
}
