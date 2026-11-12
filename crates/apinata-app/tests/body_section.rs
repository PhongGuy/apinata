use crate::support;

use apinata_app::body_section::{BodyMode, BodySection};
use apinata_app::command::Edit;
use apinata_core::BodySpec;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use std::path::PathBuf;
use support::*;

fn body(
    cx: &mut TestAppContext,
    spec: BodySpec,
) -> (AnyWindowHandle, Entity<SectionHost<BodySection>>) {
    let (win, host) = host_section(cx, |w, cx| BodySection::new(PathBuf::from("/c"), w, cx));
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| s.update(cx, |s, cx| s.load(&spec, w, cx)))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    (win, host)
}

fn view(cx: &mut TestAppContext, host: &Entity<SectionHost<BodySection>>) -> Entity<BodySection> {
    host.read_with(cx, |h, _| h.view.clone())
}

fn last_body(cx: &mut TestAppContext, host: &Entity<SectionHost<BodySection>>) -> BodySpec {
    match host.read_with(cx, |h, _| h.edits.last().cloned()) {
        Some(Edit::Body(b)) => b,
        _ => panic!("no body edit"),
    }
}

fn code(cx: &mut TestAppContext, s: &Entity<BodySection>) -> (String, String) {
    let code = s.read_with(cx, |s, _| s.code().clone());
    code.read_with(cx, |c, _| {
        (c.value().to_string(), c.language_name().to_string())
    })
}

fn set_mode(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    s: &Entity<BodySection>,
    mode: BodyMode,
) {
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.set_mode(mode, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
}

fn shown<const N: usize>(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    ids: [&'static str; N],
) -> [bool; N] {
    cx.update_window(win, |_, w, _| ids.map(|id| w.try_find(id).is_some()))
        .unwrap()
}

#[gpui_kit::test]
fn a_json_body_shows_highlighted_code(cx: &mut TestAppContext) {
    let (win, host) = body(cx, BodySpec::Json("{\"a\": 1}".into()));
    let s = view(cx, &host);
    assert_eq!(s.read_with(cx, |s, _| s.mode()), Some(BodyMode::Json));
    assert_eq!(code(cx, &s), ("{\"a\": 1}".into(), "json".into()));
    assert_eq!(
        shown(
            cx,
            win,
            ["body-mode", "body-code", "body-table", "body-none"]
        ),
        [true, true, false, false]
    );
    assert!(host.read_with(cx, |h, _| h.edits.is_empty()));
}

#[gpui_kit::test]
fn typing_in_the_code_editor_sends_the_body(cx: &mut TestAppContext) {
    let (win, host) = body(cx, BodySpec::Json("{}".into()));
    replace_code(cx, win, None, "body-code", "{\"a\": 1");
    cx.run_until_parked();
    assert_eq!(last_body(cx, &host), BodySpec::Json("{\"a\": 1}".into()));
}

#[gpui_kit::test]
fn each_mode_keeps_its_own_text_for_the_session(cx: &mut TestAppContext) {
    let (win, host) = body(cx, BodySpec::Json("{\"k\": 0}".into()));
    let s = view(cx, &host);
    set_mode(cx, win, &s, BodyMode::Text);
    assert_eq!(last_body(cx, &host), BodySpec::Text(String::new()));
    assert_eq!(code(cx, &s), (String::new(), "text".into()));
    replace_code(cx, win, None, "body-code", "hi");
    cx.run_until_parked();
    set_mode(cx, win, &s, BodyMode::Json);
    assert_eq!(last_body(cx, &host), BodySpec::Json("{\"k\": 0}".into()));
    assert_eq!(code(cx, &s).0, "{\"k\": 0}");
    set_mode(cx, win, &s, BodyMode::Text);
    assert_eq!(last_body(cx, &host), BodySpec::Text("hi".into()));
    let n = host.read_with(cx, |h, _| h.edits.len());
    set_mode(cx, win, &s, BodyMode::Text);
    assert_eq!(host.read_with(cx, |h, _| h.edits.len()), n);
}

#[gpui_kit::test]
fn xml_is_highlighted_as_markup_and_no_body_shows_no_editor(cx: &mut TestAppContext) {
    let (win, host) = body(cx, BodySpec::Xml("<a/>".into()));
    let s = view(cx, &host);
    assert_eq!(code(cx, &s), ("<a/>".into(), "html".into()));
    set_mode(cx, win, &s, BodyMode::None);
    assert_eq!(last_body(cx, &host), BodySpec::None);
    assert_eq!(shown(cx, win, ["body-none", "body-code"]), [true, false]);
}

#[gpui_kit::test]
fn a_reload_replaces_the_text_silently_and_an_echo_keeps_the_caret(cx: &mut TestAppContext) {
    let (win, host) = body(cx, BodySpec::Text("abc".into()));
    let s = view(cx, &host);
    let editor = s.read_with(cx, |s, _| s.code().clone());
    cx.update_window(win, |_, w, cx| {
        w.click_at("body-code", code_at(), cx);
        w.press("end", cx);
        w.press("left", cx)
    })
    .unwrap();
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&BodySpec::Text("abc".into()), w, cx))
    })
    .unwrap();
    assert_eq!(editor.read_with(cx, |c, _| c.cursor()), 2);
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&BodySpec::Text("xyz".into()), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(code(cx, &s).0, "xyz");
    assert!(host.read_with(cx, |h, _| h.edits.is_empty()));
}

#[gpui_kit::test]
fn an_unsupported_body_is_shown_but_not_editable(cx: &mut TestAppContext) {
    let (win, host) = body(
        cx,
        BodySpec::Unsupported {
            kind: "msgpack".into(),
        },
    );
    let s = view(cx, &host);
    assert_eq!(s.read_with(cx, |s, _| s.mode()), None);
    assert_eq!(
        shown(cx, win, ["body-unsupported", "body-code", "body-table"]),
        [true, false, false]
    );
    assert!(host.read_with(cx, |h, _| h.edits.is_empty()));
    set_mode(cx, win, &s, BodyMode::Json);
    assert_eq!(last_body(cx, &host), BodySpec::Json(String::new()));
}

#[gpui_kit::test]
fn the_body_editor_fills_its_slot(cx: &mut TestAppContext) {
    let text: String = (0..40).map(|i| format!("line {i}\n")).collect();
    let (win, host) = body(cx, BodySpec::Text(text));
    let s = view(cx, &host);
    let code = s.read_with(cx, |s, _| s.code().clone());
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let slot = w.find("body-code").bounds();
        let input = code.read(cx).input_bounds();
        assert!(
            input.size.height >= slot.size.height - gpui_kit::px(20.),
            "editor {input:?} short of its slot {slot:?}"
        );
        assert!(
            input.bottom() <= slot.bottom() + gpui_kit::px(0.5),
            "{input:?} past {slot:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn sparql_is_listed_last_as_plain_code(cx: &mut TestAppContext) {
    assert_eq!(BodyMode::ALL.last(), Some(&BodyMode::Sparql));
    assert_eq!(BodyMode::Sparql.label(), "SPARQL");
    let (win, host) = body(cx, BodySpec::Sparql("ASK {}".into()));
    let s = view(cx, &host);
    assert_eq!(s.read_with(cx, |s, _| s.mode()), Some(BodyMode::Sparql));
    assert_eq!(code(cx, &s), ("ASK {}".into(), "text".into()));
    assert!(!s.read_with(cx, |s, cx| s.can_reformat(cx)));
    replace_code(cx, win, None, "body-code", "SELECT * WHERE { ?s ?p ?o }");
    cx.run_until_parked();
    assert_eq!(
        last_body(cx, &host),
        BodySpec::Sparql("SELECT * WHERE { ?s ?p ?o }".into())
    );
}

#[gpui_kit::test]
fn a_sparql_edit_marks_the_request_dirty_and_saves_as_sparql(cx: &mut TestAppContext) {
    use apinata_app::command::{Command, Target};
    let text = "info:\n  name: q\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: \"http://127.0.0.1:9/q\"\n  body:\n    type: sparql\n    data: ASK {}\n";
    let dir = write_tree(&[("opencollection.yml", ROOT), ("q.yml", text)]);
    let model = open(cx, &dir);
    let edit = |body: &str| Command::Edit {
        target: Target::Request(ip("q.yml")),
        edit: Edit::Body(BodySpec::Sparql(body.into())),
    };
    send(cx, &model, edit("ASK {}"));
    cx.run_until_parked();
    assert!(!snap(cx, &model).content.requests[&ip("q.yml")].dirty);
    send(cx, &model, edit("SELECT ?s WHERE { ?s ?p ?o }"));
    cx.run_until_parked();
    assert!(snap(cx, &model).content.requests[&ip("q.yml")].dirty);
    send(cx, &model, Command::Save(Target::Request(ip("q.yml"))));
    cx.run_until_parked();
    let disk = std::fs::read_to_string(dir.path().join("q.yml")).unwrap();
    assert!(
        disk.contains("  body:\n    type: sparql\n    data: SELECT ?s WHERE { ?s ?p ?o }\n"),
        "{disk}"
    );
}
