//! Script approvals filed by the MCP server: shown with the current text, approved by hash,
//! dismissed without a record, and dropped when the text changed.

use crate::support;

use apinata_app::approvals::Approvals;
use apinata_core::{ApprovalInbox, ScriptDecision, ScriptTrust, script_hash};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use std::path::PathBuf;
use support::*;
use tempfile::TempDir;

const CODE: &str = "const t = 'dotenv-secret-123'";

struct Fx {
    state: TempDir,
    col: TempDir,
}

impl Fx {
    fn new() -> Fx {
        let a = format!(
            "{}runtime:\n  scripts:\n    - type: before-request\n      code: \"{CODE}\"\n",
            request("a", "http://127.0.0.1:9/ok")
        );
        Fx {
            state: TempDir::new().unwrap(),
            col: write_tree(&[
                ("opencollection.yml", ROOT),
                (".env", "TOKEN=dotenv-secret-123\n"),
                ("a.yml", &a),
            ]),
        }
    }
    fn trust_file(&self) -> PathBuf {
        self.state.path().join("script-trust.json")
    }
    fn inbox(&self) -> ApprovalInbox {
        ApprovalInbox::new(self.state.path().join("approvals"))
    }
    fn trust(&self) -> ScriptTrust {
        ScriptTrust::load(self.trust_file()).unwrap()
    }
    fn file(&self, hashes: &[String]) {
        let trust = ScriptTrust::empty(self.trust_file());
        self.inbox()
            .request_scripts(&trust, self.col.path(), "a.yml", hashes)
            .unwrap();
    }
    fn model(&self, cx: &mut TestAppContext) -> Entity<Approvals> {
        let hosts = self.state.path().join("trusted-hosts.json");
        let inbox = self.state.path().join("approvals");
        let script_trust = self.trust_file();
        let model = cx.new(|cx| {
            let mut a = Approvals::new(hosts, inbox, cx);
            a.set_script_trust(script_trust, cx);
            a
        });
        cx.run_until_parked();
        model
    }
}

#[gpui_kit::test]
fn a_filed_request_shows_only_current_unapproved_texts_masked(cx: &mut TestAppContext) {
    let fx = Fx::new();
    fx.file(&[script_hash(CODE), script_hash("gone")]);
    let model = fx.model(cx);
    let queue = model.read_with(cx, |a, _| a.script_queue().to_vec());
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].request, "a.yml");
    let texts: Vec<&str> = queue[0]
        .texts
        .iter()
        .map(|(_, t)| t.text.as_str())
        .collect();
    assert_eq!(texts, [CODE]);
    assert_eq!(queue[0].texts[0].1.shown, "const t = '<secret>'");
}

#[gpui_kit::test]
fn approving_allows_the_collection_and_clears_the_entry(cx: &mut TestAppContext) {
    let fx = Fx::new();
    fx.file(&[script_hash(CODE)]);
    let model = fx.model(cx);
    let prompt = model.read_with(cx, |a, _| a.script_queue()[0].clone());
    model
        .update(cx, |a, cx| a.approve_scripts(&prompt, cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        fx.trust().decision(fx.col.path()),
        Some(ScriptDecision::Allowed)
    );
    assert!(fx.trust().is_approved(fx.col.path(), &script_hash(CODE)));
    assert!(fx.inbox().pending_scripts().unwrap().is_empty());
    assert!(model.read_with(cx, |a, _| a.script_queue().is_empty()));
}

#[gpui_kit::test]
fn dismissing_records_nothing_and_clears_the_entry(cx: &mut TestAppContext) {
    let fx = Fx::new();
    fx.file(&[script_hash(CODE)]);
    let model = fx.model(cx);
    let prompt = model.read_with(cx, |a, _| a.script_queue()[0].clone());
    model.update(cx, |a, cx| a.dismiss_scripts(&prompt, cx));
    cx.run_until_parked();
    assert!(!fx.trust_file().exists());
    assert!(fx.inbox().pending_scripts().unwrap().is_empty());
}

#[gpui_kit::test]
fn a_dismissal_that_cannot_delete_the_entry_sets_it_aside(cx: &mut TestAppContext) {
    let fx = Fx::new();
    fx.file(&[script_hash(CODE)]);
    let model = fx.model(cx);
    let prompt = model.read_with(cx, |a, _| a.script_queue()[0].clone());
    let inbox = fx.state.path().join("approvals");
    let guard = crate::support::ModeGuard::set(&inbox, 0o555);
    model.update(cx, |a, cx| a.dismiss_scripts(&prompt, cx));
    cx.run_until_parked();
    let still_filed = fx.inbox().pending_scripts().unwrap().len();
    guard.restore();
    assert_eq!(still_filed, 1);
    assert!(model.read_with(cx, |a, _| a.script_queue().is_empty()));
}

#[gpui_kit::test]
fn approved_texts_of_an_undecided_collection_are_still_shown(cx: &mut TestAppContext) {
    let fx = Fx::new();
    let mut trust = ScriptTrust::empty(fx.trust_file());
    trust.approve(fx.col.path(), [script_hash(CODE)]);
    trust.save().unwrap();
    fx.file(&[script_hash(CODE)]);
    let model = fx.model(cx);
    let queue = model.read_with(cx, |a, _| a.script_queue().to_vec());
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].texts[0].1.text, CODE);
}

#[gpui_kit::test]
fn an_entry_whose_texts_all_changed_is_deleted_without_a_prompt(cx: &mut TestAppContext) {
    let fx = Fx::new();
    fx.file(&[script_hash("old text")]);
    let model = fx.model(cx);
    assert!(model.read_with(cx, |a, _| a.script_queue().is_empty()));
    assert!(fx.inbox().pending_scripts().unwrap().is_empty());
}

#[gpui_kit::test]
fn the_dialog_shows_the_text_and_only_approve_approves(cx: &mut TestAppContext) {
    let fx = Fx::new();
    let s = started_with(cx, &fx.col, |paths| {
        let trust = ScriptTrust::empty(&paths.script_trust);
        ApprovalInbox::new(&paths.approvals)
            .request_scripts(&trust, fx.col.path(), "a.yml", &[script_hash(CODE)])
            .unwrap();
    });
    cx.run_until_parked();
    frame(cx, s.win);
    assert!(present(cx, s.win, "script-approval-text"));
    assert!(present(cx, s.win, "script-approval-dismiss-hint"));
    cx.simulate_keystrokes(s.win, "enter");
    frame(cx, s.win);
    let trust = ScriptTrust::load(s.files.path().join("script-trust.json")).unwrap();
    assert_eq!(trust.decision(&s.root), None);
    let inbox = ApprovalInbox::new(s.files.path().join("approvals"));
    inbox
        .request_scripts(&trust, &s.root, "a.yml", &[script_hash(CODE)])
        .unwrap();
    cx.update(|cx| {
        let approvals = cx.global::<apinata_app::app::AppState>().approvals.clone();
        approvals.update(cx, |a, cx| a.refresh(cx));
    });
    cx.run_until_parked();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("script-approval-approve", cx))
        .unwrap();
    frame(cx, s.win);
    let trust = ScriptTrust::load(s.files.path().join("script-trust.json")).unwrap();
    assert!(trust.is_approved(&s.root, &script_hash(CODE)));
}

#[gpui_kit::test]
fn enter_dismisses_the_dialog_and_deletes_the_entry(cx: &mut TestAppContext) {
    let fx = Fx::new();
    let s = started_with(cx, &fx.col, |paths| {
        let trust = ScriptTrust::empty(&paths.script_trust);
        ApprovalInbox::new(&paths.approvals)
            .request_scripts(&trust, fx.col.path(), "a.yml", &[script_hash(CODE)])
            .unwrap();
    });
    cx.run_until_parked();
    frame(cx, s.win);
    assert!(
        s.shell
            .read_with(cx, |sh, _| sh.script_approval_shown().is_some())
    );
    cx.simulate_keystrokes(s.win, "enter");
    frame(cx, s.win);
    assert!(!present(cx, s.win, "script-approval-text"));
    assert!(
        s.shell
            .read_with(cx, |sh, _| sh.script_approval_shown().is_none())
    );
    let inbox = ApprovalInbox::new(s.files.path().join("approvals"));
    assert!(inbox.pending_scripts().unwrap().is_empty());
}

#[test]
fn script_text_keeps_its_lines_and_marks_hidden_characters() {
    use apinata_app::script_prompt::untrusted_lines;
    assert_eq!(
        untrusted_lines("a\r\n\tb\u{202e}c\u{fe0f}"),
        ["a", "    b\u{fffd}c"]
    );
}

#[gpui_kit::test]
fn a_secret_environment_value_from_the_store_is_masked(cx: &mut TestAppContext) {
    // The entry names no environment, so every environment's secrets must be masked.
    const ENV_CODE: &str = "const t = 'env-secret-456'";
    let a = format!(
        "{}runtime:\n  scripts:\n    - type: before-request\n      code: \"{ENV_CODE}\"\n",
        request("a", "http://127.0.0.1:9/ok")
    );
    let dev = "name: dev\nvariables:\n  - secret: true\n    name: token\n";
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", dev),
        ("a.yml", &a),
    ]);
    let store = std::sync::Arc::new(apinata_core::MemoryStore::new());
    let key = apinata_core::Collection::open(col.path())
        .unwrap()
        .secret_key("dev", "token");
    apinata_core::SecretStore::set(store.as_ref(), &key, "env-secret-456").unwrap();
    let s = started_with_store(cx, &col, store);
    let trust = ScriptTrust::empty(s.files.path().join("script-trust.json"));
    ApprovalInbox::new(s.files.path().join("approvals"))
        .request_scripts(&trust, &s.root, "a.yml", &[script_hash(ENV_CODE)])
        .unwrap();
    let approvals = cx.update(|cx| cx.global::<apinata_app::app::AppState>().approvals.clone());
    approvals.update(cx, |a, cx| a.refresh(cx));
    cx.run_until_parked();
    let queue = approvals.read_with(cx, |a, _| a.script_queue().to_vec());
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].texts[0].1.shown, "const t = '<secret>'");
}

#[gpui_kit::test]
fn approve_is_the_rightmost_button_after_the_hint(cx: &mut TestAppContext) {
    let fx = Fx::new();
    let s = started_with(cx, &fx.col, |paths| {
        let trust = ScriptTrust::empty(&paths.script_trust);
        ApprovalInbox::new(&paths.approvals)
            .request_scripts(&trust, fx.col.path(), "a.yml", &[script_hash(CODE)])
            .unwrap();
    });
    cx.run_until_parked();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        let footer = w.find("dialog-footer").bounds();
        let hint = w.find("script-approval-dismiss-hint").bounds();
        let deny = w.find("script-approval-deny").bounds();
        let approve = w.find("script-approval-approve").bounds();
        assert!(hint.right() <= deny.left() && deny.right() < approve.left());
        assert_eq!(approve.right(), footer.right());
    })
    .unwrap();
}
