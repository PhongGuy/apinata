use crate::support;

use apinata_app::command::{Command, Edit, Target};
use apinata_app::kv_table::KvRow;
use apinata_app::request_editor::Section;
use apinata_app::tabs::TabKind;
use apinata_core::{Collection, VarValue};
use gpui_kit::AppContext;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{SharedString, TestAppContext, VisualTestContext};
use support::*;

const A: &str = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://x/0?a=1\"\n  params:\n    - name: a\n      value: \"1\"\n      type: query\n  headers:\n    - name: X-One\n      value: \"1\"\n      description: kept\nruntime:\n  variables:\n    - name: token\n      value: abc\n";

fn tree() -> tempfile::TempDir {
    write_tree(&[("opencollection.yml", ROOT), ("a.yml", A)])
}

fn section(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle, s: Section) {
    cx.update_window(win, |_, w, cx| w.click(("section", s as usize), cx))
        .unwrap();
}

#[gpui_kit::test]
fn header_and_var_edits_reach_the_draft(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    section(cx, win, Section::Headers);
    type_at_end(cx, win, Some("headers"), ("kv-value", 0usize), "2");
    cx.run_until_parked();
    section(cx, win, Section::Vars);
    type_at_end(cx, win, Some("vars"), ("kv-name", 1usize), "n");
    cx.run_until_parked();
    let view = view_of(cx, &ws, &root, "a.yml");
    assert_eq!(view.headers[0].value, "12");
    assert_eq!(view.vars.len(), 2);
    assert!(view.dirty);
}

#[gpui_kit::test]
fn the_method_select_sets_and_follows_the_method(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.set_method("PUT", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(view_of(cx, &ws, &root, "a.yml").method, "PUT");
    let custom = Command::Edit {
        target: Target::Request(ip("a.yml")),
        edit: Edit::Method("PROPFIND".into()),
    };
    ws.update(cx, |w, cx| w.send(&root, custom, cx));
    cx.run_until_parked();
    assert_eq!(
        ed.read_with(cx, |e, cx| e.method(cx)),
        Some(SharedString::from("PROPFIND"))
    );
}

#[gpui_kit::test]
fn every_section_shows_its_own_view(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let all = [
        (Section::Params, "params"),
        (Section::Body, "body"),
        (Section::Headers, "headers"),
        (Section::Auth, "auth"),
        (Section::Vars, "vars"),
        (Section::Script, "script"),
        (Section::Assertions, "assertions"),
        (Section::Tests, "tests"),
        (Section::Docs, "docs"),
        (Section::Settings, "settings"),
    ];
    for (s, id) in all {
        section(cx, win, s);
        let shown = cx
            .update_window(win, |_, w, cx| {
                w.render_frame(cx);
                (
                    w.try_find(id).is_some(),
                    w.try_find("section-pending").is_some(),
                )
            })
            .unwrap();
        assert_eq!(shown, (true, false), "{id}");
    }
}
#[gpui_kit::test]
fn editing_pins_a_preview_tab(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", true);
    assert!(ws.read_with(cx, |w, _| w.tabs().active().unwrap().preview));
    type_at_end(cx, win, None, "url", "x");
    cx.run_until_parked();
    let entry = ws.read_with(cx, |w, _| w.tabs().active().cloned()).unwrap();
    assert!(!entry.preview);
    assert_eq!(entry.kind, TabKind::Request(ip("a.yml")));
}

#[gpui_kit::test]
fn save_writes_the_draft(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    type_at_end(cx, win, None, "url", "&s=1");
    cx.run_until_parked();
    let mut vcx = VisualTestContext::from_window(win, cx);
    vcx.simulate_keystrokes(&key("s"));
    vcx.run_until_parked();
    let disk = std::fs::read_to_string(col.path().join("a.yml")).unwrap();
    assert!(disk.contains("http://x/0?a=1&s=1"), "{disk}");
    assert!(!view_of(cx, &ws, &root, "a.yml").dirty);
}

#[gpui_kit::test]
fn a_refused_edit_marks_its_section_until_the_next_edit(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    section(cx, win, Section::Headers);
    let bad = vec![KvRow {
        name: "X\nY".into(),
        value: "1".into(),
        enabled: true,
        source: Some(0),
        ..KvRow::default()
    }];
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.edit_rows(Section::Headers, bad, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    assert!(
        cx.update_window(win, |_, w, _| w
            .within("headers")
            .try_find("kv-error")
            .is_some())
            .unwrap()
    );
    assert_eq!(view_of(cx, &ws, &root, "a.yml").headers[0].name, "X-One");
    type_at_end(cx, win, Some("headers"), ("kv-value", 0usize), "2");
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    assert!(
        cx.update_window(win, |_, w, _| w
            .within("headers")
            .try_find("kv-error")
            .is_none())
            .unwrap()
    );
}

#[gpui_kit::test]
fn view_edits_save_the_bytes_core_saves(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    type_at_end(cx, win, None, "url", "&b=2");
    cx.run_until_parked();
    section(cx, win, Section::Headers);
    type_at_end(cx, win, Some("headers"), ("kv-value", 0usize), "2");
    cx.run_until_parked();
    section(cx, win, Section::Vars);
    type_at_end(cx, win, Some("vars"), ("kv-value", 0usize), "x");
    cx.run_until_parked();
    let mut vcx = VisualTestContext::from_window(win, cx);
    vcx.simulate_keystrokes(&key("s"));
    vcx.run_until_parked();

    let twin = tree();
    let mut c = Collection::open(twin.path()).unwrap();
    let r = c.request_mut(&ip("a.yml")).unwrap();
    r.set_url("http://x/0?a=1&b=2");
    let mut headers = r.headers();
    headers[0].value = "12".into();
    r.set_headers(headers).unwrap();
    let mut vars = r.vars();
    vars[0].value = VarValue::Text("abcx".into());
    r.set_vars(vars);
    r.save().unwrap();
    let ours = std::fs::read_to_string(col.path().join("a.yml")).unwrap();
    let core = std::fs::read_to_string(twin.path().join("a.yml")).unwrap();
    assert_eq!(ours, core);
    assert!(ours.contains("description: kept"), "{ours}");
}

fn first_value(
    cx: &mut TestAppContext,
    ed: &gpui_kit::Entity<apinata_app::request_editor::RequestEditor>,
    s: Section,
) -> String {
    ed.read_with(cx, |e, cx| {
        let table = e.table(s).unwrap().read(cx);
        table.value_input(0).unwrap().read(cx).value().to_string()
    })
}

// A notify while a later keystroke is in flight must not write the older snapshot value
// back over the table.
fn an_ack_keeps_the_later_keystroke(cx: &mut TestAppContext, s: Section, scope: &'static str) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    section(cx, win, s);
    type_at_end(cx, win, Some(scope), ("kv-value", 0usize), "c");
    cx.run_until_parked();
    let acked = first_value(cx, &ed, s);
    cx.update_window(win, |_, w, cx| w.input("d", cx)).unwrap();
    ws.update(cx, |_, cx| cx.notify());
    assert_eq!(first_value(cx, &ed, s), format!("{acked}d"));
    cx.run_until_parked();
    let view = view_of(cx, &ws, &root, "a.yml");
    let stored = match s {
        Section::Headers => view.headers[0].value.clone(),
        _ => match &view.vars[0].value {
            VarValue::Text(t) => t.clone(),
            _ => String::new(),
        },
    };
    assert_eq!(stored, format!("{acked}d"));
}

#[gpui_kit::test]
fn an_ack_for_an_earlier_header_keystroke_keeps_the_later_one(cx: &mut TestAppContext) {
    an_ack_keeps_the_later_keystroke(cx, Section::Headers, "headers");
}

#[gpui_kit::test]
fn an_ack_for_an_earlier_var_keystroke_keeps_the_later_one(cx: &mut TestAppContext) {
    an_ack_keeps_the_later_keystroke(cx, Section::Vars, "vars");
}

#[gpui_kit::test]
fn the_empty_rows_reach_the_url_and_file_only_once_typed(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    for s in [Section::Params, Section::Headers, Section::Vars] {
        section(cx, win, s);
        cx.run_until_parked();
    }
    let view = view_of(cx, &ws, &root, "a.yml");
    assert!(!view.dirty);
    assert_eq!(
        (view.params.len(), view.headers.len(), view.vars.len()),
        (1, 1, 1)
    );
    assert_eq!(
        ed.read_with(cx, |e, cx| e.section_count(Section::Headers, cx)),
        Some(1)
    );
    section(cx, win, Section::Params);
    type_at_end(cx, win, Some("params"), ("kv-name", 1usize), "q");
    cx.run_until_parked();
    let view = view_of(cx, &ws, &root, "a.yml");
    assert_eq!(view.params.len(), 2);
    assert!(view.url.contains("a=1&q"), "{}", view.url);
    let mut vcx = VisualTestContext::from_window(win, cx);
    vcx.simulate_keystrokes(&key("s"));
    vcx.run_until_parked();
    let disk = std::fs::read_to_string(col.path().join("a.yml")).unwrap();
    assert_eq!(disk.matches("- name:").count(), 4, "{disk}");
    assert!(!disk.contains("name: \"\""), "{disk}");
}

#[gpui_kit::test]
fn a_clicked_section_tab_shows_no_ring_until_the_keyboard_moves(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(("section", 2usize), cx)
    })
    .unwrap();
    frame(cx, win);
    let handle = ed.read_with(cx, |e, _| e.section_focus(2));
    let focused = cx
        .update_window(win, |_, w, _| handle.is_focused(w))
        .unwrap();
    assert!(focused, "the click focused the tab");
    let ring =
        |cx: &mut TestAppContext| (0..8usize).any(|ix| present(cx, win, ("section-focus", ix)));
    assert!(!ring(cx), "a click leaves no ring");
    cx.update_window(win, |_, w, cx| w.press("tab", cx))
        .unwrap();
    frame(cx, win);
    assert!(ring(cx), "Tab shows the ring");
}

#[gpui_kit::test]
fn a_refused_header_name_marks_only_its_own_cell(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let three = Command::Edit {
        target: Target::Request(ip("a.yml")),
        edit: Edit::Headers(vec![
            apinata_core::KeyValue::new("X-One", "1"),
            apinata_core::KeyValue::new("X-Two", "2"),
            apinata_core::KeyValue::new("X-Three", "3"),
        ]),
    };
    ws.update(cx, |w, cx| w.send(&root, three, cx));
    cx.run_until_parked();
    section(cx, win, Section::Headers);
    frame(cx, win);
    let bounds = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, _| {
            w.within("headers").find(("kv-name", 2usize)).bounds()
        })
        .unwrap()
    };
    let unmarked = bounds(cx);
    let bad = |name: &str, source| KvRow {
        name: name.into(),
        value: "1".into(),
        enabled: true,
        source: Some(source),
        ..KvRow::default()
    };
    let rows = vec![bad("X-One", 0), bad("X-Two", 1), bad("X\nY", 2)];
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.edit_rows(Section::Headers, rows, w, cx))
    })
    .unwrap();
    frame(cx, win);
    let found = |cx: &mut TestAppContext, id: &'static str, ix: usize| {
        cx.update_window(win, |_, w, _| {
            w.within("headers").try_find((id, ix)).is_some()
        })
        .unwrap()
    };
    assert!(
        found_plain(cx, win, "kv-error"),
        "the section message shows"
    );
    for ix in 0..3 {
        let marked = ix == 2;
        assert_eq!(found(cx, "kv-name-invalid-icon", ix), marked, "icon {ix}");
        assert_eq!(found(cx, "kv-name-danger-edge", ix), marked, "edge {ix}");
    }
    assert_eq!(bounds(cx), unmarked, "the marked cell keeps its bounds");
}

fn found_plain(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle, id: &'static str) -> bool {
    cx.update_window(win, |_, w, _| w.within("headers").try_find(id).is_some())
        .unwrap()
}

/// Which of the first `rows` header name cells carry the alert icon.
fn marked_names(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle, rows: usize) -> Vec<bool> {
    cx.update_window(win, |_, w, _| {
        (0..rows)
            .map(|ix| {
                w.within("headers")
                    .try_find(("kv-name-invalid-icon", ix))
                    .is_some()
            })
            .collect()
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_header_name_with_a_bad_character_is_marked_and_still_saved(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    section(cx, win, Section::Headers);
    type_at_end(cx, win, Some("headers"), ("kv-name", 1usize), "X Bad:Name");
    cx.run_until_parked();
    frame(cx, win);
    assert_eq!(marked_names(cx, win, 3), [false, true, false]);
    let mut vcx = VisualTestContext::from_window(win, cx);
    vcx.simulate_keystrokes(&key("s"));
    vcx.run_until_parked();
    let disk = std::fs::read_to_string(col.path().join("a.yml")).unwrap();
    assert!(disk.contains("name: X Bad:Name"), "{disk}");
}

#[gpui_kit::test]
fn token_names_placeholders_and_disabled_rows_are_never_marked(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    section(cx, win, Section::Headers);
    let row = |name: &str, enabled, source| KvRow {
        name: name.into(),
        value: "1".into(),
        enabled,
        source: Some(source),
        ..KvRow::default()
    };
    let rows = vec![
        row("X-Trace_id-2", true, 0),
        row("{{name}}", true, 1),
        row("X-{{a}}", true, 2),
        row("X {{a}}", true, 3),
        row("Off Bad", false, 4),
    ];
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.edit_rows(Section::Headers, rows, w, cx))
    })
    .unwrap();
    frame(cx, win);
    assert_eq!(marked_names(cx, win, 5), [false, false, false, true, false]);
}

// A vars edit sent after a refused header edit's snapshot redrew, but before its notices
// arrived, leaves the refusal's error under the headers in the next frame.
#[gpui_kit::test]
fn an_edit_sent_before_a_refusal_is_delivered_keeps_it(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    section(cx, win, Section::Headers);
    let row = |name: &str| KvRow {
        name: name.into(),
        value: "1".into(),
        enabled: true,
        source: Some(0),
        ..KvRow::default()
    };
    let acked = move |ws: &gpui_kit::Entity<apinata_app::workspace::Workspace>,
                      cx: &gpui_kit::App| {
        ws.read(cx).snapshot(&root, cx).map_or(0, |s| s.acked)
    };
    let before = ws.read_with(cx, |_, cx| acked(&ws, cx));
    let fired = std::rc::Rc::new(std::cell::Cell::new(false));
    let (flag, gap_ed) = (fired.clone(), ed.clone());
    let _gap = cx.update(|cx| {
        cx.observe(&ws, move |ws, cx| {
            if !flag.get() && acked(&ws, cx) > before {
                flag.set(true);
                win.update(cx, |_, w, cx| {
                    gap_ed.update(cx, |e, cx| {
                        e.edit_rows(Section::Vars, vec![row("v")], w, cx)
                    })
                })
                .unwrap();
            }
        })
    });
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| {
            e.edit_rows(Section::Headers, vec![row("X\nY")], w, cx)
        })
    })
    .unwrap();
    frame(cx, win);
    assert!(fired.get(), "the vars edit went out in the gap");
    assert!(
        cx.update_window(win, |_, w, _| w
            .within("headers")
            .try_find("kv-error")
            .is_some())
            .unwrap(),
        "the refused header's error is shown"
    );
}
