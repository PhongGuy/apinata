//! What the app's own script editors save is approved for MCP, without deciding trust.

use crate::support;

use apinata_app::command::Command;
use apinata_app::config_editor::ConfigSection;
use apinata_app::request_editor::Section;
use apinata_app::tabs::TabKind;
use apinata_core::{ScriptTrust, script_hash};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, TestAppContext};
use std::path::{Path, PathBuf};
use support::*;

fn plain() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x")),
    ])
}

fn type_and_save(cx: &mut TestAppContext, s: &Started, text: &str) {
    type_and_save_in(cx, s, &s.root, text);
}

fn type_and_save_in(cx: &mut TestAppContext, s: &Started, root: &Path, text: &str) {
    let (win, ed) = editor_for(cx, &s.ws, root, "a.yml", false);
    cx.update_window(win, |_, w, cx| {
        w.click(("section", Section::Script as usize), cx)
    })
    .unwrap();
    frame(cx, win);
    replace_code(
        cx,
        win,
        Some("script-section"),
        "script-pre-request-code",
        text,
    );
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| ed.update(cx, |e, cx| e.save(w, cx)))
        .unwrap();
}

fn trust_path(s: &Started) -> PathBuf {
    s.files.path().join("script-trust.json")
}

fn approved(path: &Path, root: &Path, text: &str) -> bool {
    ScriptTrust::load(path.to_path_buf())
        .map(|t| t.is_approved(root, &script_hash(text)))
        .unwrap_or(false)
}

#[gpui_kit::test]
fn a_saved_script_is_approved_trimmed_and_undecided(cx: &mut TestAppContext) {
    let col = plain();
    let s = started(cx, &col);
    type_and_save(cx, &s, "  let a = 1  ");
    let path = trust_path(&s);
    wait_real(cx, |_| approved(&path, &s.root, "let a = 1"));
    let t = ScriptTrust::load(path.clone()).unwrap();
    assert_eq!(t.decision(&s.root), None);
    assert!(!t.is_approved(&s.root, &script_hash("  let a = 1  ")));
    assert!(
        std::fs::read_to_string(col.path().join("a.yml"))
            .unwrap()
            .contains("let a = 1")
    );
}

#[gpui_kit::test]
fn an_unreadable_trust_file_is_left_alone(cx: &mut TestAppContext) {
    let col = plain();
    let s = started(cx, &col);
    let path = trust_path(&s);
    std::fs::write(&path, "not json").unwrap();
    type_and_save(cx, &s, "let a = 1");
    wait_real(cx, |_| {
        std::fs::read_to_string(col.path().join("a.yml"))
            .unwrap()
            .contains("let a = 1")
    });
    settle(cx);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "not json");
}

// A script already in the file came from someone else; saving beside it must not vouch for it.
#[gpui_kit::test]
fn a_script_loaded_from_the_file_is_not_approved_by_saving(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &format!(
                "{}runtime:\n  scripts:\n    - type: before-request\n      code: \"bru.setVar('a', 1)\"\n",
                request("a", "http://x")
            ),
        ),
    ]);
    let s = started(cx, &col);
    let (win, ed) = editor_for(cx, &s.ws, &s.root, "a.yml", false);
    cx.update_window(win, |_, w, cx| {
        w.click(("section", Section::Tests as usize), cx)
    })
    .unwrap();
    frame(cx, win);
    replace_code(
        cx,
        win,
        Some("tests-section"),
        "script-tests-code",
        "test('t', () => {})",
    );
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| ed.update(cx, |e, cx| e.save(w, cx)))
        .unwrap();
    let path = trust_path(&s);
    wait_real(cx, |_| approved(&path, &s.root, "test('t', () => {})"));
    settle(cx);
    assert!(!approved(&path, &s.root, "bru.setVar('a', 1)"));
}

#[gpui_kit::test]
fn collection_settings_approve_the_script_they_save(cx: &mut TestAppContext) {
    let col = plain();
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
            .position(|x| *x == ConfigSection::Script)
    });
    cx.update_window(s.win, |_, w, cx| {
        w.click(("config-section", ix.unwrap()), cx)
    })
    .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click(("script-tab", 1usize), cx))
        .unwrap();
    frame(cx, s.win);
    replace_code(
        cx,
        s.win,
        Some("script-section"),
        "script-post-response-code",
        "let b = 2",
    );
    cx.run_until_parked();
    ed.update(cx, |e, cx| e.save(cx));
    let path = trust_path(&s);
    wait_real(cx, |_| approved(&path, &s.root, "let b = 2"));
    assert_eq!(ScriptTrust::load(path).unwrap().decision(&s.root), None);
}

// Opens `a.yml` as a tab of the started shell and types `text` as its pre-request script,
// leaving the tab dirty with the typed text not yet saved.
fn type_in_tab(cx: &mut TestAppContext, s: &Started, text: &str) {
    type_in_tab_of(cx, s, &s.root, text);
}

// `type_in_tab`, for the `a.yml` of the open collection at `root`.
fn type_in_tab_of(cx: &mut TestAppContext, s: &Started, root: &Path, text: &str) {
    open_tab(cx, &s.ws, root, "a.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.click(("section", Section::Script as usize), cx)
    })
    .unwrap();
    frame(cx, s.win);
    replace_code(
        cx,
        s.win,
        Some("script-section"),
        "script-pre-request-code",
        text,
    );
    cx.run_until_parked();
}

// Waits until `a.yml` holds `text` on disk and the approval's background write had its
// chance, then reports whether `text` was approved.
fn saved_and_approved(cx: &mut TestAppContext, s: &Started, text: &str) -> bool {
    let file = s.root.join("a.yml");
    wait_real(cx, |_| {
        std::fs::read_to_string(&file)
            .unwrap_or_default()
            .contains(text)
    });
    settle(cx);
    approved(&trust_path(s), &s.root, text)
}

#[gpui_kit::test]
fn the_palette_save_approves_the_typed_script(cx: &mut TestAppContext) {
    use apinata_app::keymap::SaveActive;
    let col = plain();
    let s = started(cx, &col);
    type_in_tab(cx, &s, "let c = 3");
    gpui_kit::VisualTestContext::from_window(s.win, cx).dispatch_action(SaveActive);
    assert!(
        saved_and_approved(cx, &s, "let c = 3"),
        "Save from the palette left the typed script unapproved"
    );
}

#[gpui_kit::test]
fn save_all_approves_the_typed_script(cx: &mut TestAppContext) {
    use apinata_app::tab_area::SaveAll;
    let col = plain();
    let s = started(cx, &col);
    type_in_tab(cx, &s, "let d = 4");
    gpui_kit::VisualTestContext::from_window(s.win, cx).dispatch_action(SaveAll);
    assert!(
        saved_and_approved(cx, &s, "let d = 4"),
        "Save all left the typed script unapproved"
    );
}

#[gpui_kit::test]
fn the_close_prompts_save_all_approves_the_typed_script(cx: &mut TestAppContext) {
    let col = plain();
    let s = started(cx, &col);
    type_in_tab(cx, &s, "let e = 5");
    assert!(!gpui_kit::VisualTestContext::from_window(s.win, cx).simulate_close());
    answer_confirm(cx, "confirm-save");
    assert!(
        saved_and_approved(cx, &s, "let e = 5"),
        "Save all from the close prompt left the typed script unapproved"
    );
}

// Each dirty editor vouches only for its own typing, and only in its own collection.
#[gpui_kit::test]
fn save_all_approves_each_editors_texts_against_its_own_collection(cx: &mut TestAppContext) {
    use apinata_app::tab_area::SaveAll;
    let col = plain();
    let other = plain();
    let s = started(cx, &col);
    let root2 = other.path().to_path_buf();
    s.ws.update(cx, |w, cx| w.open(root2.clone(), cx));
    cx.run_until_parked();
    type_in_tab(cx, &s, "let f = 6");
    type_in_tab_of(cx, &s, &root2, "let g = 7");
    gpui_kit::VisualTestContext::from_window(s.win, cx).dispatch_action(SaveAll);
    let file2 = root2.join("a.yml");
    wait_real(cx, |_| {
        std::fs::read_to_string(&file2)
            .unwrap_or_default()
            .contains("let g = 7")
    });
    assert!(saved_and_approved(cx, &s, "let f = 6"));
    let path = trust_path(&s);
    assert!(approved(&path, &root2, "let g = 7"));
    assert!(!approved(&path, &s.root, "let g = 7"));
    assert!(!approved(&path, &root2, "let f = 6"));
}

// Closing one collection saves and vouches for only that collection's typing; a dirty tab
// of another open collection stays unsaved and unapproved.
#[gpui_kit::test]
fn a_scoped_close_prompt_approves_only_its_collection(cx: &mut TestAppContext) {
    use apinata_app::closing::{CloseScope, confirm_close};
    let col = plain();
    let other = plain();
    let s = started(cx, &col);
    let root2 = other.path().to_path_buf();
    s.ws.update(cx, |w, cx| w.open(root2.clone(), cx));
    cx.run_until_parked();
    type_in_tab(cx, &s, "let h = 8");
    type_in_tab_of(cx, &s, &root2, "let i = 9");
    let (ws, scope) = (s.ws.clone(), CloseScope::Collection(s.root.clone()));
    cx.update_window(s.win, |_, w, cx| confirm_close(ws, scope, w, cx))
        .unwrap();
    cx.run_until_parked();
    answer_confirm(cx, "confirm-save");
    assert!(saved_and_approved(cx, &s, "let h = 8"));
    let path = trust_path(&s);
    assert!(!approved(&path, &root2, "let i = 9"));
    assert!(
        !std::fs::read_to_string(root2.join("a.yml"))
            .unwrap()
            .contains("let i = 9")
    );
}

// Sends `a.yml` from the shell; reports whether the trust question opened and what the
// finished send warned about.
fn send_a(cx: &mut TestAppContext, s: &Started) -> (bool, Vec<String>) {
    let model = sender(cx, &s.ws);
    model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx));
    cx.run_until_parked();
    frame(cx, s.win);
    if present(cx, s.win, "script-trust-run") {
        return (true, Vec::new());
    }
    let result = finished(cx, &model);
    let warnings = result.warnings.iter().map(|w| w.to_string()).collect();
    (false, warnings)
}

fn script_yaml(code: &str) -> String {
    format!(
        "{}runtime:\n  scripts:\n    - type: before-request\n      code: \"{code}\"\n",
        request("a", "http://127.0.0.1:9/ok")
    )
}

// The value the mock server saw for header `name` on its only request, if any.
fn seen_header(
    cx: &mut TestAppContext,
    server: &wiremock::MockServer,
    name: &str,
) -> Option<String> {
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let got = handle.block_on(server.received_requests()).unwrap();
    assert_eq!(got.len(), 1, "exactly one request reached the server");
    got[0]
        .headers
        .get(name)
        .map(|v| v.to_str().unwrap().to_owned())
}

const SETS_HEADER: &str = "req.setHeader('x-ran', 'yes')";

#[gpui_kit::test]
fn a_script_typed_in_the_app_runs_without_asking(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    type_and_save(cx, &s, SETS_HEADER);
    let path = trust_path(&s);
    wait_real(cx, |_| approved(&path, &s.root, SETS_HEADER));
    let (asked, warnings) = send_a(cx, &s);
    assert!(!asked, "a script the user typed is not asked about");
    assert!(
        warnings.iter().all(|w| !w.contains("script")),
        "{warnings:?}"
    );
    // The script's header reached the server, so the script phase really ran.
    assert_eq!(seen_header(cx, &server, "x-ran").as_deref(), Some("yes"));
    assert_eq!(ScriptTrust::load(path).unwrap().decision(&s.root), None);
}

#[gpui_kit::test]
fn a_script_typed_in_a_created_collection_runs_without_asking(cx: &mut TestAppContext) {
    use crate::script_trust_prompt::{add_request, create_collection};
    http_ready(cx);
    let server = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let col = plain();
    let s = started(cx, &col);
    let (_parent, fresh) = create_collection(cx, &s);
    add_request(
        cx,
        &s,
        &fresh,
        &request("a", &format!("{}/ok", server.uri())),
    );
    type_and_save_in(cx, &s, &fresh, SETS_HEADER);
    let path = trust_path(&s);
    wait_real(cx, |_| approved(&path, &fresh, SETS_HEADER));
    let model = sender(cx, &s.ws);
    model.update(cx, |m, cx| m.send(&fresh, ip("a.yml"), cx));
    cx.run_until_parked();
    frame(cx, s.win);
    assert!(
        !present(cx, s.win, "script-trust-run"),
        "typed, so not asked"
    );
    finished(cx, &model);
    assert_eq!(seen_header(cx, &server, "x-ran").as_deref(), Some("yes"));
    assert_eq!(ScriptTrust::load(path).unwrap().decision(&fresh), None);
}

// Collection and folder scripts are approved by hash like request scripts: approved text runs
// unasked, and the same file edited on disk asks again.
fn approved_config_script_asks_again_when_changed(
    cx: &mut TestAppContext,
    file: &str,
    other_files: &[(&str, String)],
) {
    http_ready(cx);
    let yaml = |code: &str| {
        let head = if file == "opencollection.yml" {
            ROOT.to_owned()
        } else {
            folder("f")
        };
        format!("{head}request:\n  scripts:\n    - type: before-request\n      code: \"{code}\"\n")
    };
    let mut files: Vec<(&str, String)> = other_files.to_vec();
    files.push((file, yaml("bru.setVar('a', 1)")));
    if file != "opencollection.yml" {
        files.push(("opencollection.yml", ROOT.to_owned()));
    }
    let refs: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
    let col = write_tree(&refs);
    let s = started_with(cx, &col, |p| {
        let root = col.path().to_path_buf();
        ScriptTrust::update(&p.script_trust, |t| {
            t.approve(&root, [script_hash("bru.setVar('a', 1)")])
        })
        .unwrap();
    });
    let target = if file == "opencollection.yml" {
        "a.yml"
    } else {
        "f/a.yml"
    };
    let send = |cx: &mut TestAppContext| {
        let model = sender(cx, &s.ws);
        model.update(cx, |m, cx| m.send(&s.root, ip(target), cx));
        cx.run_until_parked();
        frame(cx, s.win);
        let asked = present(cx, s.win, "script-trust-run");
        if !asked {
            finished(cx, &model);
        }
        asked
    };
    assert!(!send(cx), "the approved config script runs unasked");
    std::fs::write(col.path().join(file), yaml("bru.setVar('b', 2)")).unwrap();
    s.ws.update(cx, |w, cx| w.send(&s.root, Command::ReloadAll, cx));
    // Wait until the snapshot shows the edited script rather than for a fixed time.
    wait_real(cx, |cx| {
        let content = &ws_snap(cx, &s.ws, &s.root).content;
        let scripts = match file {
            "opencollection.yml" => Some(&content.collection.scripts),
            _ => content.folders.get(&ip("f")).map(|f| &f.scripts),
        };
        scripts.is_some_and(|c| c.pre_request.contains("bru.setVar('b', 2)"))
    });
    assert!(send(cx), "a changed config script is asked about");
}

#[gpui_kit::test]
fn a_collection_script_changed_on_disk_asks_again(cx: &mut TestAppContext) {
    let a = request("a", "http://127.0.0.1:9/ok");
    approved_config_script_asks_again_when_changed(cx, "opencollection.yml", &[("a.yml", a)]);
}

#[gpui_kit::test]
fn a_folder_script_changed_on_disk_asks_again(cx: &mut TestAppContext) {
    let a = request("a", "http://127.0.0.1:9/ok");
    approved_config_script_asks_again_when_changed(cx, "f/folder.yml", &[("f/a.yml", a)]);
}

#[gpui_kit::test]
fn a_blocked_collection_skips_even_approved_scripts(cx: &mut TestAppContext) {
    use apinata_core::ScriptDecision;
    http_ready(cx);
    let server = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let a = format!(
        "{}runtime:\n  scripts:\n    - type: before-request\n      code: \"{SETS_HEADER}\"\n",
        request("a", &format!("{}/ok", server.uri()))
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let s = started_with(cx, &col, |p| {
        let root = col.path().to_path_buf();
        ScriptTrust::update(&p.script_trust, |t| {
            t.approve(&root, [script_hash(SETS_HEADER)]);
            t.set_decision(&root, Some(ScriptDecision::Blocked));
        })
        .unwrap();
    });
    let (asked, warnings) = send_a(cx, &s);
    assert!(!asked, "a blocked collection is never asked");
    assert!(
        warnings.iter().any(|w| w.contains("script")),
        "{warnings:?}"
    );
    assert_eq!(seen_header(cx, &server, "x-ran"), None);
}

#[gpui_kit::test]
fn a_script_changed_on_disk_after_a_save_asks_again(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://127.0.0.1:9/ok")),
    ]);
    let s = started(cx, &col);
    type_and_save(cx, &s, "bru.setVar('a', 1)");
    let path = trust_path(&s);
    wait_real(cx, |_| approved(&path, &s.root, "bru.setVar('a', 1)"));
    std::fs::write(col.path().join("a.yml"), script_yaml("bru.setVar('b', 2)")).unwrap();
    s.ws.update(cx, |w, cx| w.send(&s.root, Command::ReloadAll, cx));
    wait_real(cx, |cx| {
        ws_snap(cx, &s.ws, &s.root)
            .content
            .requests
            .get(&ip("a.yml"))
            .is_some_and(|r| r.scripts.pre_request.contains("'b'"))
    });
    let (asked, _) = send_a(cx, &s);
    assert!(asked, "an unapproved script on disk is asked about");
}

// Types `text` as the pre-request script of a new draft in the started shell, leaving the
// draft tab active and dirty.
fn type_in_draft(cx: &mut TestAppContext, s: &Started, text: &str) {
    new_draft_at(cx, s, apinata_core::ItemPath::root());
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.click(("section", Section::Script as usize), cx)
    })
    .unwrap();
    frame(cx, s.win);
    replace_code(
        cx,
        s.win,
        Some("script-section"),
        "script-pre-request-code",
        text,
    );
    cx.run_until_parked();
}

// Saves the active draft through the Save dialog under `name`.
fn save_draft_as(cx: &mut TestAppContext, s: &Started, name: &str) {
    use apinata_app::keymap::SaveActive;
    gpui_kit::VisualTestContext::from_window(s.win, cx).dispatch_action(SaveActive);
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.input(name, cx);
        w.press("enter", cx);
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_draft_save_approves_the_typed_script(cx: &mut TestAppContext) {
    let col = plain();
    let s = started(cx, &col);
    type_in_draft(cx, &s, "let h = 8");
    save_draft_as(cx, &s, "Fresh");
    let file = col.path().join("Fresh.yml");
    wait_real(cx, |_| file.exists());
    settle(cx);
    assert!(
        approved(&trust_path(&s), &s.root, "let h = 8"),
        "saving a draft left its typed script unapproved"
    );
    let t = ScriptTrust::load(trust_path(&s)).unwrap();
    assert_eq!(t.decision(&s.root), None);
}

#[gpui_kit::test]
fn a_cancelled_draft_save_approves_nothing(cx: &mut TestAppContext) {
    let col = plain();
    let s = started(cx, &col);
    type_in_draft(cx, &s, "let i = 9");
    gpui_kit::VisualTestContext::from_window(s.win, cx)
        .dispatch_action(apinata_app::keymap::SaveActive);
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.press("escape", cx))
        .unwrap();
    settle(cx);
    assert!(!approved(&trust_path(&s), &s.root, "let i = 9"));
}

#[gpui_kit::test]
fn the_close_prompt_saving_a_draft_approves_the_typed_script(cx: &mut TestAppContext) {
    let col = plain();
    let s = started_without_drafts(cx, &col);
    type_in_draft(cx, &s.0, "let j = 10");
    assert!(!gpui_kit::VisualTestContext::from_window(s.0.win, cx).simulate_close());
    answer_confirm(cx, "confirm-save");
    frame(cx, s.0.win);
    cx.update_window(s.0.win, |_, w, cx| {
        w.input("Closing", cx);
        w.press("enter", cx);
    })
    .unwrap();
    let file = col.path().join("Closing.yml");
    wait_real(cx, |_| file.exists());
    settle(cx);
    assert!(
        approved(&trust_path(&s.0), &s.0.root, "let j = 10"),
        "saving a draft from the close prompt left its typed script unapproved"
    );
}

// A copy made by the tree carries text nobody typed in this app, so it is not vouched for;
// the hash of an already approved script keeps covering its copy.
#[gpui_kit::test]
fn a_duplicate_is_approved_only_when_its_source_script_was(cx: &mut TestAppContext) {
    use apinata_app::command::{Command, TreeOp};
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &format!(
                "{}runtime:\n  scripts:\n    - type: before-request\n      code: \"let k = 1\"\n",
                request("a", "http://x")
            ),
        ),
        (
            "b.yml",
            &format!(
                "{}runtime:\n  scripts:\n    - type: before-request\n      code: \"let l = 2\"\n",
                request("b", "http://x")
            ),
        ),
    ]);
    let s = started(cx, &col);
    let path = trust_path(&s);
    ScriptTrust::update(&path, |t| t.approve(&s.root, &[script_hash("let l = 2")])).unwrap();
    for (item, name) in [("a.yml", "a copy"), ("b.yml", "b copy")] {
        let op = TreeOp::Clone {
            item: ip(item),
            name: name.into(),
        };
        s.ws.update(cx, |w, cx| w.send(&s.root, Command::Tree(op), cx));
    }
    wait_real(cx, |_| {
        col.path().join("a copy.yml").exists() && col.path().join("b copy.yml").exists()
    });
    settle(cx);
    assert!(!approved(&path, &s.root, "let k = 1"));
    assert!(approved(&path, &s.root, "let l = 2"));
}

// Overwrite answers a changed-on-disk banner with what this editor holds, so it is a save.
#[gpui_kit::test]
fn overwriting_a_changed_file_approves_the_typed_script(cx: &mut TestAppContext) {
    let col = plain();
    let s = started(cx, &col);
    type_in_tab(cx, &s, "let m = 11");
    let path = col.path().join("a.yml");
    std::fs::write(&path, request("a", "http://x/disk")).unwrap();
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, Command::Reload(vec![path.clone()]), cx)
    });
    cx.run_until_parked();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("banner-overwrite", cx))
        .unwrap();
    assert!(
        saved_and_approved(cx, &s, "let m = 11"),
        "Overwrite left the typed script unapproved"
    );
}

// Approval follows a confirmed write: autosave, the banner's Save on a deleted file and the
// draft save approve what was typed only once the file is written.

// Removes `a.yml` under the open editor and lets the workspace see it go.
fn delete_on_disk(cx: &mut TestAppContext, s: &Started, rel: &str) {
    let path = s.root.join(rel);
    std::fs::remove_file(&path).unwrap();
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, Command::Reload(vec![path.clone()]), cx)
    });
    cx.run_until_parked();
    frame(cx, s.win);
}

fn enable_autosave(cx: &mut TestAppContext, s: &Started) {
    let auto = s.shell.read_with(cx, |v, _| v.autosave().clone());
    auto.update(cx, |a, cx| a.set_enabled(true, cx));
    cx.run_until_parked();
    cx.executor()
        .advance_clock(apinata_app::autosave::AUTOSAVE_DELAY * 2);
    cx.run_until_parked();
}

#[gpui_kit::test]
fn autosave_approves_the_typed_script_after_the_write(cx: &mut TestAppContext) {
    let col = plain();
    let s = started(cx, &col);
    type_in_tab(cx, &s, "let n = 12");
    enable_autosave(cx, &s);
    assert!(
        saved_and_approved(cx, &s, "let n = 12"),
        "autosave left the typed script unapproved"
    );
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_failed_autosave_write_approves_nothing(cx: &mut TestAppContext) {
    let col = plain();
    let s = started(cx, &col);
    type_in_tab(cx, &s, "let o = 13");
    let dir = col.path();
    let open = std::fs::metadata(dir).unwrap().permissions();
    let _g = crate::support::ModeGuard::set(dir, 0o555);
    let _g = crate::support::ModeGuard::set(dir.join("a.yml"), 0o444);
    // Root ignores the mode bits, so the failure cannot be provoked there.
    let provoked = std::fs::write(dir.join("a.yml"), "x").is_err();
    enable_autosave(cx, &s);
    settle(cx);
    let got = approved(&trust_path(&s), &s.root, "let o = 13");
    std::fs::set_permissions(dir, open).unwrap();
    assert!(provoked, "the write could not be made to fail");
    assert!(!got, "a failed autosave write approved the typed script");
}

// Makes `a.yml` and its folder read-only so a save of it fails; the returned closure restores
// the modes and says whether the failure could be provoked (root ignores mode bits).
#[cfg(unix)]
fn make_unwritable(dir: &Path) -> impl FnOnce() -> bool + use<> {
    // The guards move into the closure, so an abandoned closure still restores on drop.
    let dir_guard = crate::support::ModeGuard::set(dir, 0o555);
    let file_guard = crate::support::ModeGuard::set(dir.join("a.yml"), 0o444);
    let provoked = std::fs::write(dir.join("a.yml"), "x").is_err();
    move || {
        dir_guard.restore();
        file_guard.restore();
        provoked
    }
}

// Runs `save` with the typed script unsaveable and reports whether `text` got approved.
#[cfg(unix)]
fn failed_save_approves(
    cx: &mut TestAppContext,
    text: &str,
    save: impl FnOnce(&mut TestAppContext, &Started),
) -> bool {
    let col = plain();
    let s = started(cx, &col);
    type_in_tab(cx, &s, text);
    let restore = make_unwritable(col.path());
    save(cx, &s);
    settle(cx);
    cx.run_until_parked();
    let got = approved(&trust_path(&s), &s.root, text);
    assert!(restore(), "the write could not be made to fail");
    got
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_failed_manual_save_approves_nothing(cx: &mut TestAppContext) {
    use apinata_app::keymap::SaveActive;
    let got = failed_save_approves(cx, "let p = 14", |cx, s| {
        gpui_kit::VisualTestContext::from_window(s.win, cx).dispatch_action(SaveActive);
    });
    assert!(!got, "a failed manual save approved the typed script");
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_failed_editor_save_approves_nothing(cx: &mut TestAppContext) {
    let col = plain();
    let s = started(cx, &col);
    let (win, ed) = editor_for(cx, &s.ws, &s.root, "a.yml", false);
    cx.update_window(win, |_, w, cx| {
        w.click(("section", Section::Script as usize), cx)
    })
    .unwrap();
    frame(cx, win);
    replace_code(
        cx,
        win,
        Some("script-section"),
        "script-pre-request-code",
        "let s = 17",
    );
    cx.run_until_parked();
    let restore = make_unwritable(col.path());
    cx.update_window(win, |_, w, cx| ed.update(cx, |e, cx| e.save(w, cx)))
        .unwrap();
    settle(cx);
    cx.run_until_parked();
    let got = approved(&trust_path(&s), &s.root, "let s = 17");
    assert!(restore(), "the write could not be made to fail");
    assert!(
        !got,
        "a failed request editor save approved the typed script"
    );
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_failed_settings_save_approves_nothing(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let col = plain();
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
            .position(|x| *x == ConfigSection::Script)
    });
    cx.update_window(s.win, |_, w, cx| {
        w.click(("config-section", ix.unwrap()), cx)
    })
    .unwrap();
    frame(cx, s.win);
    replace_code(
        cx,
        s.win,
        Some("script-section"),
        "script-pre-request-code",
        "let t = 18",
    );
    cx.run_until_parked();
    let restore = make_unwritable(col.path());
    let config = col.path().join("opencollection.yml");
    let _g = crate::support::ModeGuard::set(&config, 0o444);
    ed.update(cx, |e, cx| e.save(cx));
    settle(cx);
    cx.run_until_parked();
    let got = approved(&trust_path(&s), &s.root, "let t = 18");
    std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(restore(), "the write could not be made to fail");
    assert!(!got, "a failed settings save approved the typed script");
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_failed_save_all_approves_nothing(cx: &mut TestAppContext) {
    use apinata_app::tab_area::SaveAll;
    let got = failed_save_approves(cx, "let q = 15", |cx, s| {
        gpui_kit::VisualTestContext::from_window(s.win, cx).dispatch_action(SaveAll);
    });
    assert!(!got, "a failed Save all approved the typed script");
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_failed_close_prompt_save_approves_nothing(cx: &mut TestAppContext) {
    let got = failed_save_approves(cx, "let r = 16", |cx, s| {
        assert!(!gpui_kit::VisualTestContext::from_window(s.win, cx).simulate_close());
        answer_confirm(cx, "confirm-save");
    });
    assert!(!got, "a failed close-prompt save approved the typed script");
}

#[gpui_kit::test]
fn the_deleted_banners_save_approves_the_typed_script(cx: &mut TestAppContext) {
    let col = plain();
    let s = started(cx, &col);
    type_in_tab(cx, &s, "let p = 14");
    delete_on_disk(cx, &s, "a.yml");
    cx.update_window(s.win, |_, w, cx| w.click("banner-save", cx))
        .unwrap();
    assert!(
        saved_and_approved(cx, &s, "let p = 14"),
        "the banner's Save left the typed script unapproved"
    );
}

#[gpui_kit::test]
fn the_deleted_banners_save_never_approves_text_from_disk(cx: &mut TestAppContext) {
    let yaml = format!(
        "{}runtime:\n  scripts:\n    - type: before-request\n      code: \"let q = 15\"\n",
        request("a", "http://x")
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &yaml)]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    // A non-script edit makes the tab dirty, so the deleted banner shows.
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, edit_url("a.yml", "http://x/y"), cx)
    });
    cx.run_until_parked();
    delete_on_disk(cx, &s, "a.yml");
    cx.update_window(s.win, |_, w, cx| w.click("banner-save", cx))
        .unwrap();
    let file = s.root.join("a.yml");
    wait_real(cx, |_| file.exists());
    settle(cx);
    assert!(!approved(&trust_path(&s), &s.root, "let q = 15"));
}

#[gpui_kit::test]
fn the_deleted_banners_save_in_folder_settings_approves_the_typed_script(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("f")),
        ("f/a.yml", &request("a", "http://x")),
    ]);
    let s = started(cx, &col);
    let tab =
        s.ws.update(cx, |w, cx| {
            w.open_settings(&s.root, TabKind::Folder(ip("f")), cx)
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
            .position(|x| *x == ConfigSection::Script)
    });
    cx.update_window(s.win, |_, w, cx| {
        w.click(("config-section", ix.unwrap()), cx)
    })
    .unwrap();
    frame(cx, s.win);
    replace_code(
        cx,
        s.win,
        Some("script-section"),
        "script-pre-request-code",
        "let r = 16",
    );
    cx.run_until_parked();
    delete_on_disk(cx, &s, "f/folder.yml");
    cx.update_window(s.win, |_, w, cx| w.click("banner-save", cx))
        .unwrap();
    let file = s.root.join("f/folder.yml");
    wait_real(cx, |_| {
        std::fs::read_to_string(&file)
            .unwrap_or_default()
            .contains("let r = 16")
    });
    settle(cx);
    assert!(
        approved(&trust_path(&s), &s.root, "let r = 16"),
        "the settings banner's Save left the typed script unapproved"
    );
}

// A read-only collection folder makes the actor's write fail; the typed script stays unapproved.
#[cfg(unix)]
#[gpui_kit::test]
fn a_draft_save_that_fails_approves_nothing(cx: &mut TestAppContext) {
    let col = plain();
    let s = started(cx, &col);
    type_in_draft(cx, &s, "let s = 17");
    let dir = col.path();
    let open = std::fs::metadata(dir).unwrap().permissions();
    let _g = crate::support::ModeGuard::set(dir, 0o555);
    // Root ignores the mode bits, so the failure cannot be provoked there.
    let provoked = std::fs::write(dir.join("probe"), "x").is_err();
    save_draft_as(cx, &s, "Fresh");
    settle(cx);
    let got = approved(&trust_path(&s), &s.root, "let s = 17");
    let written = dir.join("Fresh.yml").exists();
    std::fs::set_permissions(dir, open).unwrap();
    assert!(provoked && !written, "the write could not be made to fail");
    assert!(!got, "a failed draft write approved the typed script");
}

// Registers an approval for `text` against command `id` of `a.yml`, as a save in flight would.
fn await_write(cx: &mut TestAppContext, s: &Started, id: u64, text: &str) {
    s.ws.update(cx, |w, _| {
        w.approve_when_saved(
            &s.root,
            Some(id),
            Some(apinata_app::command::Target::Request(ip("a.yml"))),
            vec![text.to_owned()],
        )
    });
}

// Command ids restart when a collection reopens, so an approval left from before the close
// must not fire for an unrelated save.
#[gpui_kit::test]
fn closing_a_collection_drops_its_waiting_approvals(cx: &mut TestAppContext) {
    let col = plain();
    let s = started(cx, &col);
    await_write(cx, &s, 1, "let t = 18");
    s.ws.update(cx, |w, cx| w.close(&s.root, cx));
    cx.run_until_parked();
    s.ws.update(cx, |w, cx| w.open(s.root.clone(), cx));
    cx.run_until_parked();
    s.ws.update(cx, |w, cx| {
        w.send(
            &s.root,
            Command::Save(apinata_app::command::Target::Request(ip("a.yml"))),
            cx,
        )
    });
    settle(cx);
    assert!(!approved(&trust_path(&s), &s.root, "let t = 18"));
}

// A refused or failed command acknowledges its id but vouches for nothing.
#[gpui_kit::test]
fn a_stale_or_failed_command_approves_nothing(cx: &mut TestAppContext) {
    use apinata_app::command::{Edit, Target};
    let col = plain();
    let s = started(cx, &col);
    // Failed: a draft path with no draft is not a file save.
    let failed = s.ws.update(cx, |w, cx| {
        w.send(
            &s.root,
            Command::Save(Target::Request(ip("missing.yml"))),
            cx,
        )
    });
    s.ws.update(cx, |w, _| {
        w.approve_when_saved(
            &s.root,
            failed,
            Some(Target::Request(ip("a.yml"))),
            vec!["let u = 19".into()],
        )
    });
    // Stale: an edit held back because the file changed under the sender.
    let path = col.path().join("a.yml");
    std::fs::write(&path, request("a", "http://x/disk")).unwrap();
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, Command::Reload(vec![path.clone()]), cx)
    });
    let stale = s.ws.update(cx, |w, cx| {
        w.send(
            &s.root,
            Command::Edit {
                target: Target::Request(ip("a.yml")),
                edit: Edit::Url("http://x/mine".into()),
            },
            cx,
        )
    });
    s.ws.update(cx, |w, _| {
        w.approve_when_saved(
            &s.root,
            stale,
            Some(Target::Request(ip("a.yml"))),
            vec!["let v = 20".into()],
        )
    });
    settle(cx);
    assert!(!approved(&trust_path(&s), &s.root, "let u = 19"));
    assert!(!approved(&trust_path(&s), &s.root, "let v = 20"));
}
