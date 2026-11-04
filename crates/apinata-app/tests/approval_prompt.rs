//! The approval modal: one request at a time; only a click trusts or denies, any other way out
//! dismisses; agent text marked untrusted.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::approvals::{Approvals, Prompt};
use apinata_core::{ApprovalInbox, TrustStore};
use gpui_kit::component::WindowExt;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use std::path::{Path, PathBuf};
use support::*;

fn approvals(cx: &mut TestAppContext) -> Entity<Approvals> {
    cx.update(|cx| cx.global::<AppState>().approvals.clone())
}

fn trust_file(s: &Started) -> PathBuf {
    s.files.path().join("trusted-hosts.json")
}

fn inbox(s: &Started) -> ApprovalInbox {
    ApprovalInbox::new(s.files.path().join("approvals"))
}

// Files a request as the MCP server does, then has the app look.
fn file(cx: &mut TestAppContext, s: &Started, collection: &Path, host: &str, request: &str) {
    let store = TrustStore::empty(trust_file(s));
    inbox(s).request(&store, collection, host, request).unwrap();
    approvals(cx).update(cx, |a, cx| a.refresh(cx));
    cx.run_until_parked();
    frame(cx, s.win);
}

fn showing(cx: &mut TestAppContext, s: &Started) -> Option<Prompt> {
    s.shell.read_with(cx, |v, _| v.approval_shown().cloned())
}

fn dialog_open(cx: &mut TestAppContext, s: &Started) -> bool {
    cx.update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap()
}

fn store(s: &Started) -> TrustStore {
    TrustStore::load(trust_file(s)).unwrap()
}

fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap()
}

fn answer_by_key(cx: &mut TestAppContext, s: &Started, key: &str) {
    cx.simulate_keystrokes(s.win, key);
    cx.run_until_parked();
    frame(cx, s.win);
}

fn click(cx: &mut TestAppContext, s: &Started, id: &'static str) {
    cx.update_window(s.win, |_, w, cx| w.click(id, cx)).unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
}

#[gpui_kit::test]
fn a_filed_request_opens_the_prompt_with_the_real_host(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    assert!(!dialog_open(cx, &s));
    file(cx, &s, &s.root, "API.Example.com.", "users/get.yml");
    assert!(dialog_open(cx, &s));
    let p = showing(cx, &s).expect("a prompt");
    assert_eq!(
        (p.collection, p.host.as_str(), p.request.as_str()),
        (canonical(&s.root), "api.example.com", "users/get.yml")
    );
    for id in [
        "approval-host",
        "approval-collection",
        "approval-request",
        "approval-untrusted-note",
        "approval-deny",
        "approval-trust",
    ] {
        assert!(present(cx, s.win, id), "{id}");
    }
    assert!(!present(cx, s.win, "approval-host-note"));
}

#[gpui_kit::test]
fn an_international_host_is_shown_encoded_with_a_note(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    file(cx, &s, &s.root, "bücher.example", "a.yml");
    assert_eq!(showing(cx, &s).unwrap().host, "xn--bcher-kva.example");
    assert!(present(cx, s.win, "approval-host-note"));
}

// Nothing recorded, nothing filed: the outcome of every way out that is not a click.
fn assert_dismissed(cx: &mut TestAppContext, s: &Started) {
    assert!(!trust_file(s).exists(), "no lasting record");
    assert!(
        inbox(s).pending().unwrap().is_empty(),
        "the request is removed"
    );
    assert!(!dialog_open(cx, s));
    assert_eq!(showing(cx, s), None);
}

#[gpui_kit::test]
fn enter_and_escape_dismiss(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    for key in ["enter", "escape"] {
        file(cx, &s, &s.root, "api.example.com", "a.yml");
        assert!(dialog_open(cx, &s), "{key}");
        answer_by_key(cx, &s, key);
        assert_dismissed(cx, &s);
    }
}

#[gpui_kit::test]
fn the_prompt_says_escape_dismisses(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    file(cx, &s, &s.root, "api.example.com", "a.yml");
    assert!(dialog_open(cx, &s));
    assert!(present(cx, s.win, "approval-later-hint"));
    assert_eq!(apinata_app::approval_prompt::ESCAPE_HINT, "Esc: dismiss");
    answer_by_key(cx, &s, "escape");
    assert_dismissed(cx, &s);
}

#[gpui_kit::test]
fn no_key_reaches_the_buttons(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    file(cx, &s, &s.root, "api.example.com", "a.yml");
    for key in ["tab", "space", "tab", "tab", "space"] {
        answer_by_key(cx, &s, key);
    }
    assert!(
        !trust_file(&s).exists(),
        "tab and space never press Trust or Deny"
    );
    answer_by_key(cx, &s, "tab");
    answer_by_key(cx, &s, "enter");
    assert_dismissed(cx, &s);
}

#[gpui_kit::test]
fn closing_the_window_dismisses(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    file(cx, &s, &s.root, "api.example.com", "a.yml");
    let (trust, inbox) = (trust_file(&s), inbox(&s));
    // The Shell is released only once the test's handles to it are gone too.
    let Started {
        files,
        win,
        shell,
        ws,
        ..
    } = s;
    drop((shell, ws));
    cx.update_window(win, |_, w, _| w.remove_window()).unwrap();
    cx.run_until_parked();
    assert!(!trust.exists());
    assert!(inbox.pending().unwrap().is_empty());
    drop(files);
}

fn notes(cx: &mut TestAppContext, s: &Started) -> usize {
    cx.update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

#[gpui_kit::test]
fn a_decision_made_elsewhere_stands(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    for button in ["approval-trust", "approval-deny"] {
        file(cx, &s, &s.root, "api.example.com", "a.yml");
        // As `apinata trust deny` would, while the prompt is open; the refresh stands in
        // for the watcher's.
        let mut other = TrustStore::empty(trust_file(&s));
        other.deny(&s.root, "api.example.com").unwrap();
        other.save().unwrap();
        let before = std::fs::read(trust_file(&s)).unwrap();
        approvals(cx).update(cx, |a, cx| a.refresh(cx));
        frame(cx, s.win);
        assert!(dialog_open(cx, &s), "the prompt stays until answered");
        let shown = notes(cx, &s);
        click(cx, &s, button);
        assert!(!dialog_open(cx, &s), "{button}");
        assert_eq!(
            std::fs::read(trust_file(&s)).unwrap(),
            before,
            "{button} wrote nothing"
        );
        assert_eq!(notes(cx, &s), shown + 1, "an already-decided notice");
        std::fs::remove_file(trust_file(&s)).unwrap();
    }
}

#[gpui_kit::test]
fn only_the_trust_button_trusts(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    file(cx, &s, &s.root, "api.example.com", "a.yml");
    click(cx, &s, "approval-trust");
    assert!(store(&s).is_trusted(&s.root, "api.example.com"));
    assert!(!dialog_open(cx, &s));
    assert!(inbox(&s).pending().unwrap().is_empty());
}

#[gpui_kit::test]
fn requests_are_asked_one_at_a_time_open_collection_or_not(cx: &mut TestAppContext) {
    let col = two_requests();
    let other = two_requests(); // never opened in the app
    let s = started(cx, &col);
    file(cx, &s, other.path(), "a.example", "a.yml");
    file(cx, &s, &s.root, "b.example", "b.yml");
    let first = showing(cx, &s).unwrap();
    assert_eq!(
        (first.collection, first.host.as_str()),
        (canonical(other.path()), "a.example")
    );
    click(cx, &s, "approval-deny");
    assert!(store(&s).is_denied(other.path(), "a.example"));
    assert!(!store(&s).is_trusted(other.path(), "a.example"));
    assert!(
        inbox(&s)
            .pending()
            .unwrap()
            .iter()
            .all(|a| a.host != "a.example")
    );
    assert_eq!(showing(cx, &s).unwrap().host, "b.example");
    assert!(dialog_open(cx, &s));
}

// One half of a mouse click at `at`, as the platform reports it, then the app settles.
fn mouse(
    cx: &mut TestAppContext,
    s: &Started,
    at: gpui_kit::Point<gpui_kit::Pixels>,
    count: usize,
) {
    use gpui_kit::{InputEvent as _, MouseButton, MouseDownEvent, MouseUpEvent};
    let down = MouseDownEvent {
        button: MouseButton::Left,
        position: at,
        modifiers: Default::default(),
        click_count: count,
        first_mouse: false,
    };
    let up = MouseUpEvent {
        button: MouseButton::Left,
        position: at,
        modifiers: Default::default(),
        click_count: count,
    };
    for event in [down.to_platform_input(), up.to_platform_input()] {
        cx.update_window(s.win, |_, w, cx| {
            w.dispatch_event(event, cx);
        })
        .unwrap();
        cx.run_until_parked();
        frame(cx, s.win);
    }
}

#[gpui_kit::test]
fn a_double_click_answers_only_the_first_prompt(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    file(cx, &s, &s.root, "a.example", "a.yml");
    file(cx, &s, &s.root, "b.example", "b.yml");
    assert_eq!(showing(cx, &s).unwrap().host, "a.example");
    let at = cx
        .update_window(s.win, |_, w, _| w.find("approval-trust").bounds().center())
        .unwrap();
    mouse(cx, &s, at, 1);
    mouse(cx, &s, at, 2);
    assert!(store(&s).is_trusted(&s.root, "a.example"));
    assert!(
        !store(&s).is_trusted(&s.root, "b.example"),
        "the repeat click lands on b"
    );
    assert_eq!(showing(cx, &s).unwrap().host, "b.example");
    assert!(dialog_open(cx, &s));
}

fn press(cx: &mut TestAppContext, s: &Started, key: &str) {
    cx.update_window(s.win, |_, w, cx| w.press(key, cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
}

#[gpui_kit::test]
fn real_key_presses_never_reach_the_buttons(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    file(cx, &s, &s.root, "api.example.com", "a.yml");
    for key in [
        "tab",
        "space",
        "tab",
        "space",
        "shift-tab",
        "space",
        "shift-tab",
        "space",
    ] {
        press(cx, &s, key);
        assert!(!trust_file(&s).exists(), "{key} trusted or denied");
    }
    for key in ["tab", "tab", "shift-tab", "shift-tab", "tab"] {
        press(cx, &s, key);
        let host = cx
            .update_window(s.win, |_, w, _| w.find("approval-host").focused())
            .unwrap();
        assert_eq!(host, Some(true), "focus left the host line after {key}");
    }
}

#[gpui_kit::test]
fn the_prompt_names_the_collection_and_keeps_its_path_on_one_line(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    file(cx, &s, &s.root, "api.example.com", "a.yml");
    assert!(dialog_open(cx, &s));
    assert!(present(cx, s.win, "approval-collection"));
    assert!(present(cx, s.win, "approval-collection-path"));
    let name =
        s.ws.read_with(cx, |w, cx| w.collection_name(&canonical(&s.root), cx));
    assert_eq!(
        name.as_ref(),
        "c",
        "the canonical prompt path finds the open collection"
    );
    let line = cx
        .update_window(s.win, |_, w, _| w.find("approval-collection-path").bounds())
        .unwrap();
    let host = cx
        .update_window(s.win, |_, w, _| w.find("approval-collection").bounds())
        .unwrap();
    assert!(
        line.size.height <= host.size.height,
        "the path does not wrap: {line:?} vs {host:?}"
    );
}

// The dialog's right edge is the Trust button's: every line the agent controls must end inside
// it, and both buttons must still be there.
fn assert_lines_inside_buttons(cx: &mut TestAppContext, s: &Started, ids: &[&'static str]) {
    for id in ["approval-deny", "approval-trust"] {
        assert!(present(cx, s.win, id), "{id}");
    }
    let (edge, rights) = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            let edge = w.find("approval-trust").bounds().right();
            let rights: Vec<_> = ids
                .iter()
                .map(|id| (*id, w.find(*id).bounds().right()))
                .collect();
            (edge, rights)
        })
        .unwrap();
    for (id, right) in rights {
        assert!(
            right <= edge,
            "{id} ends at {right:?}, past the dialog edge {edge:?}"
        );
    }
}

#[gpui_kit::test]
fn a_long_unbroken_request_and_mapped_name_keep_the_buttons_and_fit(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let long = "u".repeat(10_000);
    let store = TrustStore::empty(trust_file(&s));
    inbox(&s)
        .request_mapped(&store, &s.root, "203.0.113.5", &long, &long)
        .unwrap();
    approvals(cx).update(cx, |a, cx| a.refresh(cx));
    cx.run_until_parked();
    frame(cx, s.win);
    let shown = showing(cx, &s).expect("a prompt");
    assert!(shown.request.chars().count() <= apinata_core::DISPLAY_MAX_CHARS);
    assert_eq!(
        shown.mapped_from.as_deref().map(|m| m.chars().count()),
        Some(apinata_core::DISPLAY_MAX_CHARS)
    );
    assert_lines_inside_buttons(cx, &s, &["approval-request", "approval-mapped"]);
}

#[gpui_kit::test]
fn a_long_unbroken_proxy_keeps_the_buttons_and_fits(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let long = format!("http://{}:3128", "p".repeat(10_000));
    let store = TrustStore::empty(trust_file(&s));
    inbox(&s)
        .request_via(
            &store,
            &s.root,
            "proxy.example",
            &"r".repeat(10_000),
            Some(&long),
        )
        .unwrap();
    approvals(cx).update(cx, |a, cx| a.refresh(cx));
    cx.run_until_parked();
    frame(cx, s.win);
    assert_lines_inside_buttons(cx, &s, &["approval-request", "approval-proxy"]);
}
