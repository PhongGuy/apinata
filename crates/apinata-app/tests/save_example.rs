use crate::support;

use apinata_app::command::{Command, TreeOp};
use apinata_app::save_example::{
    BODY_LIMIT, SaveExampleForm, SecretCheck, can_save, example_response, initial_name, secrets_in,
};
use apinata_core::{
    Collection, ContextOptions, ExampleBodyKind, ItemPath, MemoryStore, Outcome, RequestResult,
    Response, SecretStore, VarContext, derived_secrets, named_secrets,
};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext, TestAppContext};
use std::sync::Arc;
use support::*;

#[test]
fn the_first_free_example_name_is_offered() {
    let names = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(initial_name(&[]), "example");
    assert_eq!(initial_name(&names(&["other"])), "example");
    assert_eq!(
        initial_name(&names(&["example", "example (1)"])),
        "example (2)"
    );
    assert_eq!(
        initial_name(&names(&["example (1)", "example"])),
        "example (2)"
    );
    assert_eq!(
        initial_name(&names(&["example", "example (2)"])),
        "example (1)"
    );
}

#[test]
fn only_a_whole_response_under_the_limit_can_be_saved() {
    let ok = Response::new(200).with_text_body("ok");
    assert!(can_save(Some(&ok)));
    assert!(!can_save(None));
    assert!(!can_save(Some(
        &Response::new(200).with_text_body("ok").with_truncated(10)
    )));
    let big = Response::new(200).with_body(vec![b'a'; BODY_LIMIT as usize], "");
    assert!(!can_save(Some(&big)));
    let under = Response::new(200).with_body(vec![b'a'; BODY_LIMIT as usize - 1], "");
    assert!(can_save(Some(&under)));
}

#[test]
fn a_json_body_is_saved_pretty() {
    let r = Response::new(200)
        .with_header("content-type", "application/json")
        .with_text_body("{\"a\":1,\"b\":[true]}");
    let body = example_response(&r).body.expect("a body");
    assert_eq!(body.kind, ExampleBodyKind::Json);
    assert_eq!(body.data, "{\n  \"a\": 1,\n  \"b\": [\n    true\n  ]\n}");
    let broken = Response::new(200)
        .with_header("content-type", "application/json")
        .with_text_body("{oops");
    assert_eq!(example_response(&broken).body.unwrap().data, "{oops");
}

#[gpui_kit::test]
fn the_form_answers_trimmed_text_and_needs_a_name(cx: &mut TestAppContext) {
    support::init(cx);
    let (win, form) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| {
                    SaveExampleForm::new("example (1)", SecretCheck::Found(Vec::new()), w, cx)
                })
            })
        })
        .unwrap();
    frame(cx, win);
    assert!(present(cx, win, "save-example-note"));
    let answer = form
        .read_with(cx, |f, cx| f.answer(cx))
        .expect("the default name");
    assert_eq!(
        (answer.name.as_str(), answer.description.as_str()),
        ("example (1)", "")
    );
    let (name, description) = form.read_with(cx, |f, _| {
        (f.name_input().clone(), f.description_input().clone())
    });
    cx.update_window(win, |_, w, cx| {
        name.update(cx, |i, cx| i.set_value("  created  ", w, cx));
        description.update(cx, |i, cx| i.set_value(" a new user ", w, cx));
    })
    .unwrap();
    let answer = form.read_with(cx, |f, cx| f.answer(cx)).expect("a name");
    assert_eq!(
        (answer.name.as_str(), answer.description.as_str()),
        ("created", "a new user")
    );
    cx.update_window(win, |_, w, cx| {
        name.update(cx, |i, cx| i.set_value("  ", w, cx))
    })
    .unwrap();
    assert!(form.read_with(cx, |f, cx| f.answer(cx)).is_none());
}

#[gpui_kit::test]
fn the_toolbar_saves_the_shown_response_with_the_unresolved_request(cx: &mut TestAppContext) {
    // `id` resolves, so the secret check runs and the body is saved; the file keeps the template.
    let col = write_tree(&[
        (
            "opencollection.yml",
            &format!("{ROOT}request:\n  variables:\n    - name: id\n      value: \"42\"\n"),
        ),
        ("a.yml", &request("a", "http://x/{{id}}")),
    ]);
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    assert!(
        !present(cx, s.win, "response-save-example"),
        "no response, no button"
    );
    let area = s.shell.read_with(cx, |sh, _| sh.tab_area().clone());
    let editor = area
        .read_with(cx, |a, _| a.editor(tab).cloned())
        .expect("the editor");
    let send = editor.read_with(cx, |e, _| e.send_model().clone());
    let response = Response::new(200)
        .with_status_text("OK")
        .with_header("content-type", "application/json")
        .with_text_body("{\"id\":\"resolved-42\"}");
    let result = RequestResult::from_parts(
        ItemPath::new("a.yml").unwrap(),
        "a",
        Outcome::Passed,
        Some(response),
        Vec::new(),
    );
    send.update(cx, |m, cx| m.show(Arc::new(result), cx));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("response-save-example", cx))
        .unwrap();
    frame(cx, s.win);
    assert!(present(cx, s.win, "save-example-dialog"));
    cx.update_window(s.win, |_, w, cx| {
        w.input("first", cx);
        w.press("enter", cx);
    })
    .unwrap();
    wait_for(cx, "the example", |_| {
        std::fs::read_to_string(col.path().join("a.yml"))
            .unwrap()
            .contains("examples:")
    });
    frame(cx, s.win);
    assert!(!present(cx, s.win, "save-example-dialog"));
    let disk = std::fs::read_to_string(col.path().join("a.yml")).unwrap();
    assert!(disk.contains("  - name: first\n"), "{disk}");
    assert_eq!(disk.matches("http://x/{{id}}").count(), 2, "{disk}");
    assert!(
        disk.contains("\"id\": \"resolved-42\""),
        "the body is pretty JSON: {disk}"
    );
    assert!(disk.contains("statusText: OK"), "{disk}");
}

#[gpui_kit::test]
fn an_unsaved_draft_cannot_save_its_response_as_an_example(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| w.new_draft(&s.root, ItemPath::root(), cx))
        .expect("a draft");
    cx.run_until_parked();
    let (tab, item) = s.ws.read_with(cx, |w, _| {
        let entry = w.tabs().active().expect("the draft tab");
        (entry.id, entry.item().cloned().expect("a request"))
    });
    assert!(item.is_draft());
    let area = s.shell.read_with(cx, |sh, _| sh.tab_area().clone());
    let editor = area
        .read_with(cx, |a, _| a.editor(tab).cloned())
        .expect("the editor");
    let send = editor.read_with(cx, |e, _| e.send_model().clone());
    let response = Response::new(200).with_text_body("ok");
    let result =
        RequestResult::from_parts(item, "draft", Outcome::Passed, Some(response), Vec::new());
    send.update(cx, |m, cx| m.show(Arc::new(result), cx));
    frame(cx, s.win);
    assert!(
        present(cx, s.win, "response-save-example"),
        "shown, but disabled"
    );
    cx.update_window(s.win, |_, w, cx| w.click("response-save-example", cx))
        .unwrap();
    frame(cx, s.win);
    assert!(
        !present(cx, s.win, "save-example-dialog"),
        "a disabled button asks nothing"
    );
    let before = toasts(cx, &s);
    cx.update_window(s.win, |_, w, cx| {
        w.dispatch_action(Box::new(apinata_app::tab_area::SaveAsExample), cx)
    })
    .unwrap();
    frame(cx, s.win);
    assert!(
        !present(cx, s.win, "save-example-dialog"),
        "the command asks nothing either"
    );
    assert_eq!(toasts(cx, &s), before + 1, "a notice gives the reason");
}

/// `dev` holds the secret `token`; `a.yml` sends it in its `x-token` header.
const DEV: &str = "name: dev\nvariables:\n  - name: token\n    secret: true\n";
const TOKEN: &str = "s3cr3t-Token";

fn secret_collection() -> tempfile::TempDir {
    let file = format!(
        "{}  headers:\n    - name: x-token\n      value: \"{{{{token}}}}\"\n",
        request("a", "http://x/a")
    );
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("a.yml", &file),
    ])
}

/// A store holding `token` for `dev` of the collection at `dir`.
fn token_store(dir: &tempfile::TempDir) -> Arc<MemoryStore> {
    let c = Collection::open(dir.path()).expect("opens");
    let store = MemoryStore::new();
    store.set(&c.secret_key("dev", "token"), TOKEN).unwrap();
    Arc::new(store)
}

fn echoed() -> Response {
    Response::new(200)
        .with_header("x-token", TOKEN)
        .with_text_body(format!("{{\"x-token\":\"{TOKEN}\"}}"))
}

#[test]
fn secrets_are_found_in_headers_and_body() {
    let dir = secret_collection();
    let store = token_store(&dir);
    let c = Collection::open(dir.path()).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new().env("dev").secrets(store)).unwrap();
    let file = c.tree().requests().first().expect("a.yml");
    let secrets = named_secrets(file, &ctx).expect("resolves");
    let mut response = example_response(&echoed());
    assert_eq!(secrets_in(&response, &secrets), vec!["token".to_owned()]);
    response.scrub(Some(&secrets));
    assert_eq!(
        response.headers,
        vec![("x-token".to_owned(), "{{token}}".to_owned())]
    );
    assert_eq!(response.body.unwrap().data, "{\"x-token\":\"{{token}}\"}");
}

/// The app over `secret_collection` with `dev` selected and its token stored, `a.yml` open
/// and showing `response`.
fn showing(cx: &mut TestAppContext, col: &tempfile::TempDir, response: Response) -> Started {
    let s = started_with_store(cx, col, token_store(col));
    s.ws.update(cx, |w, cx| {
        w.select_environment(&s.root, Some("dev".into()), cx)
    });
    cx.run_until_parked();
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let area = s.shell.read_with(cx, |sh, _| sh.tab_area().clone());
    let editor = area
        .read_with(cx, |a, _| a.editor(tab).cloned())
        .expect("the editor");
    let send = editor.read_with(cx, |e, _| e.send_model().clone());
    let result = RequestResult::from_parts(
        ItemPath::new("a.yml").unwrap(),
        "a",
        Outcome::Passed,
        Some(response),
        Vec::new(),
    );
    send.update(cx, |m, cx| m.show(Arc::new(result), cx));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("response-save-example", cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    assert!(present(cx, s.win, "save-example-dialog"));
    s
}

fn disk(col: &tempfile::TempDir) -> String {
    std::fs::read_to_string(col.path().join("a.yml")).unwrap()
}

#[gpui_kit::test]
fn enter_on_the_focused_cancel_button_saves_nothing(cx: &mut TestAppContext) {
    let col = secret_collection();
    let s = showing(cx, &col, Response::new(200).with_text_body("plain"));
    let before = disk(&col);
    enter_on(cx, s.win, "save-example-cancel");
    frame(cx, s.win);
    assert!(!present(cx, s.win, "save-example-dialog"));
    assert_eq!(disk(&col), before);
}

#[gpui_kit::test]
fn no_secret_means_the_plain_form(cx: &mut TestAppContext) {
    let col = secret_collection();
    let s = showing(cx, &col, Response::new(200).with_text_body("plain"));
    assert!(!present(cx, s.win, "example-secrets"));
    assert!(!present(cx, s.win, "example-keep-secrets"));
    cx.update_window(s.win, |_, w, cx| w.click("save-example-save", cx))
        .unwrap();
    wait_for(cx, "the example", |_| disk(&col).contains("examples:"));
    assert!(disk(&col).contains("data: plain"), "{}", disk(&col));
}

#[gpui_kit::test]
fn the_default_save_replaces_secrets(cx: &mut TestAppContext) {
    let col = secret_collection();
    let s = showing(cx, &col, echoed());
    assert!(present(cx, s.win, "example-secrets"));
    cx.update_window(s.win, |_, w, cx| w.click("save-example-save", cx))
        .unwrap();
    wait_for(cx, "the example", |_| disk(&col).contains("examples:"));
    let disk = disk(&col);
    assert!(!disk.contains(TOKEN), "{disk}");
    // The request's own header, its copy in the example, and the response header.
    assert_eq!(disk.matches("value: \"{{token}}\"").count(), 3, "{disk}");
    assert!(
        disk.contains("data: '{\"x-token\":\"{{token}}\"}'"),
        "{disk}"
    );
}

#[gpui_kit::test]
fn keep_values_needs_a_second_yes(cx: &mut TestAppContext) {
    let col = secret_collection();
    let s = showing(cx, &col, echoed());
    cx.update_window(s.win, |_, w, cx| w.click("example-keep-secrets", cx))
        .unwrap();
    assert!(confirm_open(cx));
    answer_confirm(cx, "confirm-cancel");
    settle(cx);
    assert!(!disk(&col).contains("examples:"), "Cancel saves nothing");

    cx.update_window(s.win, |_, w, cx| w.click("response-save-example", cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("example-keep-secrets", cx))
        .unwrap();
    answer_confirm(cx, "confirm-save-secrets");
    wait_for(cx, "the example", |_| disk(&col).contains("examples:"));
    assert!(disk(&col).contains(TOKEN), "{}", disk(&col));
}

#[gpui_kit::test]
fn the_form_lists_the_secret_labels(cx: &mut TestAppContext) {
    support::init(cx);
    let found = SecretCheck::Found(vec!["token".into(), "auth credentials".into()]);
    let (win, form) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| SaveExampleForm::new("example", found, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    assert!(present(cx, win, "example-secrets"));
    assert_eq!(
        form.read_with(cx, |f, _| f.secrets_note()).as_deref(),
        Some(
            "Contains secret values: token, auth credentials. Saving replaces them with \
             placeholders."
        )
    );
}

fn toasts(cx: &mut TestAppContext, s: &Started) -> usize {
    cx.update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

#[gpui_kit::test]
fn a_saved_example_is_confirmed_in_a_notification(cx: &mut TestAppContext) {
    let col = secret_collection();
    let s = showing(cx, &col, Response::new(200).with_text_body("plain"));
    let before = toasts(cx, &s);
    cx.update_window(s.win, |_, w, cx| w.click("save-example-save", cx))
        .unwrap();
    wait_for(cx, "the example", |_| disk(&col).contains("examples:"));
    cx.run_until_parked();
    assert_eq!(toasts(cx, &s), before + 1, "one notification says it saved");
}

#[gpui_kit::test]
fn an_example_lands_in_its_request_renamed_while_the_dialog_was_open(cx: &mut TestAppContext) {
    let col = secret_collection();
    let s = showing(cx, &col, Response::new(200).with_text_body("plain"));
    let rename = TreeOp::Rename {
        item: ItemPath::new("a.yml").unwrap(),
        name: "b".into(),
        rename_file: true,
    };
    s.ws.update(cx, |w, cx| w.send(&s.root, Command::Tree(rename), cx));
    cx.run_until_parked();
    assert!(col.path().join("b.yml").exists(), "the rename landed first");
    let before = toasts(cx, &s);
    cx.update_window(s.win, |_, w, cx| w.click("save-example-save", cx))
        .unwrap();
    cx.run_until_parked();
    let renamed = || std::fs::read_to_string(col.path().join("b.yml")).unwrap();
    wait_for(cx, "the example in b.yml", |_| {
        renamed().contains("examples:")
    });
    assert!(renamed().contains("data: plain"), "{}", renamed());
    assert_eq!(toasts(cx, &s), before + 1, "a success note, not a failure");
}

// Closing the tab while the dialog is open leaves nothing to re-read, so the example goes
// to the item captured when the dialog opened.
#[gpui_kit::test]
fn an_example_saved_after_its_tab_closed_lands_in_the_captured_request(cx: &mut TestAppContext) {
    let col = secret_collection();
    let s = showing(cx, &col, Response::new(200).with_text_body("plain"));
    let tab =
        s.ws.read_with(cx, |w, _| w.tabs().active().expect("a tab").id);
    let area = s.shell.read_with(cx, |sh, _| sh.tab_area().clone());
    cx.update_window(s.win, |_, w, cx| {
        area.update(cx, |a, cx| a.close(vec![tab], w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        s.ws.read_with(cx, |w, _| w.tabs().get(tab).is_none()),
        "the tab closed"
    );
    assert!(present(cx, s.win, "save-example-dialog"));
    let before = toasts(cx, &s);
    cx.update_window(s.win, |_, w, cx| w.click("save-example-save", cx))
        .unwrap();
    wait_for(cx, "the example", |_| disk(&col).contains("examples:"));
    cx.run_until_parked();
    assert!(disk(&col).contains("data: plain"), "{}", disk(&col));
    assert_eq!(toasts(cx, &s), before + 1, "a success note, not a failure");
}

fn tips(cx: &mut TestAppContext) -> Vec<String> {
    cx.update(|cx| {
        cx.global::<apinata_app::app::AppState>()
            .settings
            .read(cx)
            .get()
            .dismissed_tips
            .clone()
    })
}

// "Don't show again" stores the tip as dismissed at once; the next dialog has neither the
// note nor the checkbox.
#[gpui_kit::test]
fn dont_show_again_hides_the_commit_note_from_then_on(cx: &mut TestAppContext) {
    let col = secret_collection();
    let s = showing(cx, &col, Response::new(200).with_text_body("plain"));
    assert!(present(cx, s.win, "save-example-note"));
    assert!(tips(cx).is_empty());
    cx.update_window(s.win, |_, w, cx| w.click("save-example-note-dismiss", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        tips(cx),
        vec![apinata_app::save_example::NOTE_TIP.to_owned()]
    );
    cx.update_window(s.win, |_, w, cx| w.click("save-example-cancel", cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    assert!(!present(cx, s.win, "save-example-dialog"));
    cx.update_window(s.win, |_, w, cx| w.click("response-save-example", cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    assert!(present(cx, s.win, "save-example-dialog"));
    assert!(!present(cx, s.win, "save-example-note"));
    assert!(!present(cx, s.win, "save-example-note-dismiss"));
}

// Unticking "Don't show again" takes the tip back out, and ticking it a second time stores
// it once, not twice.
#[gpui_kit::test]
fn unticking_dont_show_again_keeps_the_commit_note(cx: &mut TestAppContext) {
    let col = secret_collection();
    let s = showing(cx, &col, Response::new(200).with_text_body("plain"));
    let tick = |cx: &mut TestAppContext| {
        cx.update_window(s.win, |_, w, cx| w.click("save-example-note-dismiss", cx))
            .unwrap();
        cx.run_until_parked();
        frame(cx, s.win);
    };
    tick(cx);
    tick(cx);
    assert!(tips(cx).is_empty(), "{:?}", tips(cx));
    tick(cx);
    assert_eq!(
        tips(cx),
        vec![apinata_app::save_example::NOTE_TIP.to_owned()]
    );
}

// In a narrow pane the bookmark folds into the overflow menu; its entry opens the same dialog.
#[gpui_kit::test]
fn the_overflow_menu_entry_opens_the_save_example_dialog(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    gpui_kit::VisualTestContext::from_window(s.win, cx)
        .simulate_resize(gpui_kit::size(gpui_kit::px(520.), gpui_kit::px(700.)));
    cx.run_until_parked();
    frame(cx, s.win);
    let area = s.shell.read_with(cx, |sh, _| sh.tab_area().clone());
    let editor = area
        .read_with(cx, |a, _| a.editor(tab).cloned())
        .expect("the editor");
    let send = editor.read_with(cx, |e, _| e.send_model().clone());
    let result = RequestResult::from_parts(
        ItemPath::new("a.yml").unwrap(),
        "a",
        Outcome::Passed,
        Some(Response::new(200).with_text_body("plain")),
        Vec::new(),
    );
    send.update(cx, |m, cx| m.show(Arc::new(result), cx));
    frame(cx, s.win);
    assert!(!present(cx, s.win, "response-save-example"));
    cx.update_window(s.win, |_, w, cx| w.click("response-more", cx))
        .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.within("popup-menu").click(0usize, cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    assert!(present(cx, s.win, "save-example-dialog"));
}

// A response echoing an environment secret the request never references still has it
// replaced, by its name, before the example reaches the git-tracked file.
#[gpui_kit::test]
fn an_unreferenced_environment_secret_is_replaced_too(cx: &mut TestAppContext) {
    const OTHER: &str = "other-Secret-99";
    let col = secret_collection();
    std::fs::write(
        col.path().join("environments/dev.yml"),
        "name: dev\nvariables:\n  - name: token\n    secret: true\n  - name: other\n    secret: true\n",
    )
    .unwrap();
    let store = token_store(&col);
    let c = Collection::open(col.path()).expect("opens");
    store.set(&c.secret_key("dev", "other"), OTHER).unwrap();
    let s = started_with_store(cx, &col, store);
    s.ws.update(cx, |w, cx| {
        w.select_environment(&s.root, Some("dev".into()), cx)
    });
    cx.run_until_parked();
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let area = s.shell.read_with(cx, |sh, _| sh.tab_area().clone());
    let editor = area.read_with(cx, |a, _| a.editor(tab).cloned()).unwrap();
    let send = editor.read_with(cx, |e, _| e.send_model().clone());
    let response = Response::new(200)
        .with_header("x-echo", OTHER)
        .with_text_body(format!("{{\"echo\":\"{OTHER}\"}}"));
    let result = RequestResult::from_parts(
        ItemPath::new("a.yml").unwrap(),
        "a",
        Outcome::Passed,
        Some(response),
        Vec::new(),
    );
    send.update(cx, |m, cx| m.show(Arc::new(result), cx));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("response-save-example", cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("save-example-save", cx))
        .unwrap();
    wait_for(cx, "the example", |_| disk(&col).contains("examples:"));
    let disk = disk(&col);
    assert!(!disk.contains(OTHER), "{disk}");
    assert!(disk.contains("{{other}}"), "{disk}");
}

#[test]
fn a_derived_value_the_server_echoes_is_found_and_redacted() {
    let header = "Digest username=\"bob\", response=\"9f3a77c1d2e44b60\"";
    let response = Response::new(200)
        .with_header("x-echo", header)
        .with_text_body(format!("{{\"auth\":\"{header}\"}}"));
    let secrets = derived_secrets(&[header.to_owned()]);
    let mut example = example_response(&response);
    assert_eq!(
        secrets_in(&example, &secrets),
        vec!["derived credential".to_owned()]
    );
    example.scrub(Some(&secrets));
    assert_eq!(
        example.headers,
        vec![("x-echo".to_owned(), "[redacted]".to_owned())]
    );
    assert_eq!(example.body.unwrap().data, "{\"auth\":\"[redacted]\"}");
}

/// Shows a 200 response on the GraphQL tab `g.yml` and dispatches the palette's Save as example.
fn dispatch_save_as_example(cx: &mut TestAppContext, s: &Started, tab: apinata_app::tabs::TabId) {
    let area = s.shell.read_with(cx, |sh, _| sh.tab_area().clone());
    let editor = area
        .read_with(cx, |a, _| a.editor(tab).cloned())
        .expect("the editor");
    let send = editor.read_with(cx, |e, _| e.send_model().clone());
    let item = ItemPath::new("g.yml").unwrap();
    let response = Response::new(200).with_text_body("ok");
    let result = RequestResult::from_parts(item, "g", Outcome::Passed, Some(response), Vec::new());
    send.update(cx, |m, cx| m.show(Arc::new(result), cx));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.dispatch_action(Box::new(apinata_app::tab_area::SaveAsExample), cx)
    })
    .unwrap();
    frame(cx, s.win);
}

#[gpui_kit::test]
fn the_command_explains_why_a_graphql_request_cannot_save_an_example(cx: &mut TestAppContext) {
    let gql = "info:\n  name: g\n  type: graphql\n  seq: 1\ngraphql:\n  url: u\n";
    let col = write_tree(&[("opencollection.yml", ROOT), ("g.yml", gql)]);
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "g.yml");
    frame(cx, s.win);
    let before = toasts(cx, &s);
    dispatch_save_as_example(cx, &s, tab);
    assert!(!present(cx, s.win, "save-example-dialog"));
    assert_eq!(toasts(cx, &s), before + 1, "a notice gives the reason");
}

/// Bytes that are not UTF-8 around the token, as a binary download that echoes it.
fn binary_echo() -> Response {
    let bytes = [b"\xff\x00".as_slice(), TOKEN.as_bytes(), b"\xfe\x01"].concat();
    Response::new(200)
        .with_header("content-type", "application/octet-stream")
        .with_body(bytes, String::new())
}

#[test]
fn a_secret_inside_a_binary_body_is_found_and_the_body_withheld() {
    let dir = secret_collection();
    let store = token_store(&dir);
    let c = Collection::open(dir.path()).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new().env("dev").secrets(store)).unwrap();
    let file = c.tree().requests().first().expect("a.yml");
    let secrets = named_secrets(file, &ctx).expect("resolves");
    let mut example = example_response(&binary_echo());
    assert_eq!(example.body.as_ref().unwrap().kind, ExampleBodyKind::Binary);
    assert_eq!(secrets_in(&example, &secrets), vec!["token".to_owned()]);
    assert!(example.scrub(Some(&secrets)).withheld);
    assert!(example.body.is_none());
    assert_eq!(example.status, Some(200));
}

#[test]
fn a_clean_binary_body_is_kept_as_it_is() {
    let secrets = derived_secrets(&[TOKEN.to_owned()]);
    let clean = Response::new(200)
        .with_header("content-type", "application/octet-stream")
        .with_body(vec![0xff, 0x00, 1, 2], String::new());
    let mut example = example_response(&clean);
    let before = example.body.clone();
    assert!(secrets_in(&example, &secrets).is_empty());
    assert!(!example.scrub(Some(&secrets)).withheld);
    assert_eq!(example.body, before);
}

#[gpui_kit::test]
fn a_binary_body_holding_a_secret_is_never_written_and_says_so(cx: &mut TestAppContext) {
    let col = secret_collection();
    let s = showing(cx, &col, binary_echo());
    assert!(present(cx, s.win, "example-secrets"));
    let before = toasts(cx, &s);
    cx.update_window(s.win, |_, w, cx| w.click("save-example-save", cx))
        .unwrap();
    wait_for(cx, "the example", |_| disk(&col).contains("examples:"));
    cx.run_until_parked();
    let disk = disk(&col);
    assert!(!disk.contains(TOKEN), "{disk}");
    let b64 = base64_of(&binary_echo());
    assert!(!disk.contains(&b64), "{disk}");
    assert!(disk.contains("status: 200"), "{disk}");
    assert!(disk.contains("withheld"), "the note is in the file: {disk}");
    assert_eq!(toasts(cx, &s), before + 2, "saved, and body withheld");
}

fn base64_of(r: &Response) -> String {
    example_response(r).body.expect("a body").data
}

#[gpui_kit::test]
fn a_clean_binary_body_saves_unchanged(cx: &mut TestAppContext) {
    let col = secret_collection();
    let clean = Response::new(200)
        .with_header("content-type", "application/octet-stream")
        .with_body(vec![0xff, 0x00, 1, 2], String::new());
    let s = showing(cx, &col, clean.clone());
    assert!(!present(cx, s.win, "example-secrets"));
    cx.update_window(s.win, |_, w, cx| w.click("save-example-save", cx))
        .unwrap();
    wait_for(cx, "the example", |_| disk(&col).contains("examples:"));
    let disk = disk(&col);
    assert!(disk.contains(&base64_of(&clean)), "{disk}");
    assert!(!disk.contains("withheld"), "{disk}");
}

#[gpui_kit::test]
fn keep_values_keeps_a_binary_body_after_the_second_yes(cx: &mut TestAppContext) {
    let col = secret_collection();
    let s = showing(cx, &col, binary_echo());
    cx.update_window(s.win, |_, w, cx| w.click("example-keep-secrets", cx))
        .unwrap();
    answer_confirm(cx, "confirm-save-secrets");
    wait_for(cx, "the example", |_| disk(&col).contains("examples:"));
    assert!(disk(&col).contains(&base64_of(&binary_echo())));
}

#[test]
fn the_form_note_names_the_withheld_binary_body() {
    let note = apinata_app::save_example::binary_secrets_note(&["token".to_owned()]);
    assert!(
        note.contains("token") && note.contains("binary body will be withheld"),
        "{note}"
    );
}

const SID_COOKIE: &str = "sid=abc123; Path=/";

#[test]
fn mask_headers_keeps_only_the_cookie_name() {
    let mut response = example_response(&Response::new(200).with_header("set-cookie", SID_COOKIE));
    response.scrub(None);
    assert_eq!(
        response.headers,
        vec![("set-cookie".to_owned(), "sid=[redacted]".to_owned())]
    );
}

#[test]
fn a_har_export_of_the_same_response_masks_the_cookie() {
    let response = Response::new(200).with_header("set-cookie", SID_COOKIE);
    let exchange = apinata_core::export::HarExchange::from_response(
        &response,
        std::time::SystemTime::UNIX_EPOCH,
    );
    let text = apinata_core::export::har(&exchange.redacted(&[]));
    assert!(text.contains("sid=[redacted]"), "{text}");
    assert!(!text.contains("abc123"), "{text}");
}

#[gpui_kit::test]
fn a_saved_example_never_holds_a_server_issued_cookie(cx: &mut TestAppContext) {
    let col = secret_collection();
    let s = showing(cx, &col, echoed().with_header("set-cookie", SID_COOKIE));
    cx.update_window(s.win, |_, w, cx| w.click("save-example-save", cx))
        .unwrap();
    wait_for(cx, "the example", |_| disk(&col).contains("examples:"));
    let disk = disk(&col);
    assert!(disk.contains("sid=[redacted]"), "{disk}");
    assert!(!disk.contains("abc123"), "{disk}");
}

#[gpui_kit::test]
fn keeping_secrets_keeps_the_raw_cookie(cx: &mut TestAppContext) {
    let col = secret_collection();
    let s = showing(cx, &col, echoed().with_header("set-cookie", SID_COOKIE));
    cx.update_window(s.win, |_, w, cx| w.click("example-keep-secrets", cx))
        .unwrap();
    answer_confirm(cx, "confirm-save-secrets");
    wait_for(cx, "the example", |_| disk(&col).contains("examples:"));
    assert!(disk(&col).contains("abc123"), "{}", disk(&col));
}

/// As `secret_collection`, but `a.yml`'s URL names a variable nothing defines, so its
/// secrets cannot be resolved and the example cannot be checked.
fn unchecked_collection() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("a.yml", &request("a", "http://x/{{missing}}")),
    ])
}

const UNCHECKED_TEXT: &str = "text-body-never-checked";

fn unchecked_text() -> Response {
    Response::new(200).with_text_body(UNCHECKED_TEXT)
}

#[gpui_kit::test]
fn an_unchecked_example_says_its_body_will_be_withheld(cx: &mut TestAppContext) {
    let col = unchecked_collection();
    let s = showing(cx, &col, unchecked_text());
    assert!(present(cx, s.win, "example-unchecked"));
    assert!(present(cx, s.win, "example-keep-secrets"));
    assert!(!present(cx, s.win, "example-secrets"));
}

#[gpui_kit::test]
fn the_unchecked_finding_names_the_withheld_body(cx: &mut TestAppContext) {
    support::init(cx);
    let check = SecretCheck::Unchecked("undefined variable".into());
    let (win, form) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| SaveExampleForm::new("example", check, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    assert!(present(cx, win, "example-unchecked"));
    let text = form.read_with(cx, |f, _| f.finding()).expect("a finding");
    assert!(
        text.starts_with("Could not check this example for secret values: undefined variable."),
        "{text}"
    );
    assert!(text.contains("body will be withheld"), "{text}");
}

#[gpui_kit::test]
fn an_unchecked_text_body_is_withheld_by_default(cx: &mut TestAppContext) {
    let col = unchecked_collection();
    let s = showing(cx, &col, unchecked_text());
    let before = toasts(cx, &s);
    cx.update_window(s.win, |_, w, cx| w.click("save-example-save", cx))
        .unwrap();
    wait_for(cx, "the example", |_| disk(&col).contains("examples:"));
    cx.run_until_parked();
    let disk = disk(&col);
    assert!(!disk.contains(UNCHECKED_TEXT), "{disk}");
    assert!(disk.contains("status: 200"), "{disk}");
    assert!(
        disk.contains("could not be checked"),
        "the note is in the file: {disk}"
    );
    assert_eq!(toasts(cx, &s), before + 2, "saved, and body withheld");
}

#[gpui_kit::test]
fn an_unchecked_binary_body_is_withheld_by_default(cx: &mut TestAppContext) {
    let col = unchecked_collection();
    let s = showing(cx, &col, binary_echo());
    cx.update_window(s.win, |_, w, cx| w.click("save-example-save", cx))
        .unwrap();
    wait_for(cx, "the example", |_| disk(&col).contains("examples:"));
    let disk = disk(&col);
    assert!(!disk.contains(&base64_of(&binary_echo())), "{disk}");
    assert!(disk.contains("could not be checked"), "{disk}");
}

#[gpui_kit::test]
fn an_unchecked_empty_body_saves_without_a_withheld_note(cx: &mut TestAppContext) {
    let col = unchecked_collection();
    let s = showing(cx, &col, Response::new(200).with_text_body(""));
    let before = toasts(cx, &s);
    cx.update_window(s.win, |_, w, cx| w.click("save-example-save", cx))
        .unwrap();
    wait_for(cx, "the example", |_| disk(&col).contains("examples:"));
    cx.run_until_parked();
    let disk = disk(&col);
    assert!(!disk.contains("withheld"), "{disk}");
    assert_eq!(toasts(cx, &s), before + 1, "only the saved toast");
}

#[gpui_kit::test]
fn an_unchecked_reason_ending_in_a_period_is_not_doubled(cx: &mut TestAppContext) {
    support::init(cx);
    let check = SecretCheck::Unchecked("undefined variable.".into());
    let (_, form) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| SaveExampleForm::new("example", check, w, cx))
            })
        })
        .unwrap();
    let text = form.read_with(cx, |f, _| f.finding()).expect("a finding");
    assert!(text.contains("undefined variable. Saving"), "{text}");
    assert!(!text.contains(".."), "{text}");
}

#[gpui_kit::test]
fn keep_values_keeps_an_unchecked_text_body(cx: &mut TestAppContext) {
    let col = unchecked_collection();
    let s = showing(cx, &col, unchecked_text());
    cx.update_window(s.win, |_, w, cx| w.click("example-keep-secrets", cx))
        .unwrap();
    answer_confirm(cx, "confirm-save-secrets");
    wait_for(cx, "the example", |_| disk(&col).contains("examples:"));
    let disk = disk(&col);
    assert!(disk.contains(UNCHECKED_TEXT), "{disk}");
    assert!(!disk.contains("withheld"), "{disk}");
}

#[gpui_kit::test]
fn keep_values_keeps_an_unchecked_binary_body(cx: &mut TestAppContext) {
    let col = unchecked_collection();
    let s = showing(cx, &col, binary_echo());
    cx.update_window(s.win, |_, w, cx| w.click("example-keep-secrets", cx))
        .unwrap();
    answer_confirm(cx, "confirm-save-secrets");
    wait_for(cx, "the example", |_| disk(&col).contains("examples:"));
    assert!(
        disk(&col).contains(&base64_of(&binary_echo())),
        "{}",
        disk(&col)
    );
}

/// Opens the dialog on `rel`'s tab over a plain 200 response.
pub fn ask_on(cx: &mut TestAppContext, s: &Started, rel: &str) {
    let tab = open_tab(cx, &s.ws, &s.root, rel);
    frame(cx, s.win);
    let area = s.shell.read_with(cx, |sh, _| sh.tab_area().clone());
    let editor = area
        .read_with(cx, |a, _| a.editor(tab).cloned())
        .expect("the editor");
    let send = editor.read_with(cx, |e, _| e.send_model().clone());
    let response = Response::new(200).with_text_body("plain");
    let item = ItemPath::new(rel).unwrap();
    let result = RequestResult::from_parts(item, "r", Outcome::Passed, Some(response), Vec::new());
    send.update(cx, |m, cx| m.show(Arc::new(result), cx));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("response-save-example", cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    assert!(present(cx, s.win, "save-example-dialog"));
}

// The name and description as drawn.
fn example_fields(cx: &mut TestAppContext, s: &Started) -> (String, String) {
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        let value = |id| w.find(id).value().unwrap_or("").to_owned();
        (
            value("save-example-name"),
            value("save-example-description"),
        )
    })
    .unwrap()
}

fn two_plain() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/a")),
        ("b.yml", &request("b", "http://x/b")),
    ])
}

fn type_example(cx: &mut TestAppContext, s: &Started) {
    replace_input(cx, s.win, None, "save-example-name", "Kept");
    type_at_end(
        cx,
        s.win,
        None,
        "save-example-description",
        "Why it matters",
    );
    frame(cx, s.win);
}

#[gpui_kit::test]
fn a_cancelled_example_form_comes_back_on_its_request(cx: &mut TestAppContext) {
    let col = two_plain();
    let s = started(cx, &col);
    ask_on(cx, &s, "a.yml");
    type_example(cx, &s);
    cx.update_window(s.win, |_, w, cx| w.click("save-example-cancel", cx))
        .unwrap();
    frame(cx, s.win);
    assert!(!present(cx, s.win, "save-example-dialog"));
    ask_on(cx, &s, "b.yml");
    assert_eq!(example_fields(cx, &s).1, "", "another request starts clean");
    cx.update_window(s.win, |_, w, cx| w.click("save-example-cancel", cx))
        .unwrap();
    frame(cx, s.win);
    ask_on(cx, &s, "a.yml");
    assert_eq!(
        example_fields(cx, &s),
        ("Kept".to_owned(), "Why it matters".to_owned())
    );
}

#[gpui_kit::test]
fn a_saved_example_leaves_no_draft(cx: &mut TestAppContext) {
    let col = two_plain();
    let s = started(cx, &col);
    ask_on(cx, &s, "a.yml");
    type_example(cx, &s);
    cx.update_window(s.win, |_, w, cx| w.click("save-example-save", cx))
        .unwrap();
    let disk = || std::fs::read_to_string(col.path().join("a.yml")).unwrap();
    wait_for(cx, "the example", |_| disk().contains("examples:"));
    settle(cx);
    ask_on(cx, &s, "a.yml");
    let (name, description) = example_fields(cx, &s);
    assert_ne!(name, "Kept");
    assert_eq!(description, "");
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_failed_example_write_keeps_the_draft(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt as _;
    // Makes the folder and the file read-only, and writable again on drop, even on a panic.
    struct ReadOnly(std::path::PathBuf);
    impl ReadOnly {
        fn set(&self, file: u32, dir: u32) {
            let mode = |p: &std::path::Path, m| {
                std::fs::set_permissions(p, std::fs::Permissions::from_mode(m)).unwrap()
            };
            mode(&self.0.join("a.yml"), file);
            mode(&self.0, dir);
        }
    }
    impl Drop for ReadOnly {
        fn drop(&mut self) {
            self.set(0o644, 0o755);
        }
    }
    let col = two_plain();
    let s = started(cx, &col);
    ask_on(cx, &s, "a.yml");
    type_example(cx, &s);
    let guard = ReadOnly(col.path().to_path_buf());
    guard.set(0o444, 0o555);
    // Root writes through the modes, so the refusal cannot be staged.
    if std::fs::write(col.path().join("probe"), "").is_ok() {
        return;
    }
    let before = toasts(cx, &s);
    cx.update_window(s.win, |_, w, cx| w.click("save-example-save", cx))
        .unwrap();
    wait_for(cx, "the failure note", |cx| toasts(cx, &s) > before);
    drop(guard);
    let file = col.path().join("a.yml");
    assert!(
        !std::fs::read_to_string(&file)
            .unwrap()
            .contains("examples:")
    );
    ask_on(cx, &s, "a.yml");
    assert_eq!(
        example_fields(cx, &s),
        ("Kept".to_owned(), "Why it matters".to_owned())
    );
}
