//! The approvals model: the inbox read as a queue of prompts, never as decisions.

use apinata_app::approvals::{Approvals, Decision, Outcome, Prompt};
use apinata_core::{ApprovalInbox, Filing, TrustStore};
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use tempfile::TempDir;

struct Fx {
    state: TempDir,
    coll: TempDir,
}

impl Fx {
    fn new() -> Fx {
        let coll = TempDir::new().unwrap();
        fs::write(
            coll.path().join("opencollection.yml"),
            "opencollection: 1.0.0\n",
        )
        .unwrap();
        Fx {
            state: TempDir::new().unwrap(),
            coll,
        }
    }
    fn trust_file(&self) -> PathBuf {
        self.state.path().join("trusted-hosts.json")
    }
    fn inbox_dir(&self) -> PathBuf {
        self.state.path().join("approvals")
    }
    fn root(&self) -> PathBuf {
        fs::canonicalize(self.coll.path()).unwrap()
    }
    fn store(&self) -> TrustStore {
        TrustStore::load(self.trust_file()).unwrap()
    }
    fn pending(&self) -> usize {
        ApprovalInbox::new(self.inbox_dir())
            .pending()
            .unwrap()
            .len()
    }
    // Files a request as the MCP server does.
    fn file(&self, host: &str, request: &str) -> PathBuf {
        let store = TrustStore::empty(self.trust_file());
        let inbox = ApprovalInbox::new(self.inbox_dir());
        match inbox
            .request(&store, self.coll.path(), host, request)
            .unwrap()
        {
            Filing::Written(path) => path,
            other => panic!("expected a new file, got {other:?}"),
        }
    }
    // Writes an inbox file by hand, as any local process could.
    fn forge(&self, name: &str, host: &str, extra: serde_json::Value) {
        let mut doc = json!({ "version": 1, "collection": self.root(), "host": host,
            "request": "a.yml", "requested_at": "2026-10-09T21:14:03Z" });
        if let serde_json::Value::Object(more) = extra {
            doc.as_object_mut().unwrap().extend(more);
        }
        fs::create_dir_all(self.inbox_dir()).unwrap();
        fs::write(self.inbox_dir().join(name), doc.to_string()).unwrap();
    }
}

fn model(cx: &mut TestAppContext, fx: &Fx) -> Entity<Approvals> {
    let m = cx.new(|cx| Approvals::new(fx.trust_file(), fx.inbox_dir(), cx));
    cx.run_until_parked();
    m
}

fn queue(cx: &mut TestAppContext, m: &Entity<Approvals>) -> Vec<Prompt> {
    m.read_with(cx, |a, _| a.queue().to_vec())
}

fn hosts(cx: &mut TestAppContext, m: &Entity<Approvals>) -> Vec<String> {
    queue(cx, m).into_iter().map(|p| p.host).collect()
}

#[gpui_kit::test]
fn requests_queue_once_per_pair_in_file_order(cx: &mut TestAppContext) {
    let fx = Fx::new();
    fx.file("api.example.com", "users/get.yml");
    fx.file("other.example", "b.yml");
    // A second file for a queued pair, as a second server process would leave.
    fx.forge(
        "zz-dup.json",
        "api.example.com",
        json!({ "request": "c.yml" }),
    );
    let m = model(cx, &fx);
    let q = queue(cx, &m);
    assert_eq!(q.len(), 2, "{q:?}");
    assert_eq!(
        (
            q[0].collection.clone(),
            q[0].host.as_str(),
            q[0].request.as_str()
        ),
        (fx.root(), "api.example.com", "users/get.yml")
    );
    assert_eq!(
        (q[1].host.as_str(), q[1].request.as_str()),
        ("other.example", "b.yml")
    );
}

#[gpui_kit::test]
fn trust_writes_the_store_and_clears_only_that_pair(cx: &mut TestAppContext) {
    let fx = Fx::new();
    fx.file("api.example.com", "a.yml");
    fx.file("other.example", "a.yml");
    let m = model(cx, &fx);
    let first = queue(cx, &m)[0].clone();
    assert_eq!(
        m.update(cx, |a, cx| a.decide(&first, Decision::Trust, cx)),
        Ok(Outcome::Recorded)
    );
    cx.run_until_parked();
    assert!(fx.store().is_trusted(&fx.root(), "api.example.com"));
    assert!(!fx.store().is_denied(&fx.root(), "api.example.com"));
    assert_eq!(hosts(cx, &m), ["other.example"]);
    assert_eq!(fx.pending(), 1);
}

#[gpui_kit::test]
fn deny_writes_the_denied_list(cx: &mut TestAppContext) {
    let fx = Fx::new();
    fx.file("evil.example", "a.yml");
    let m = model(cx, &fx);
    let p = queue(cx, &m)[0].clone();
    m.update(cx, |a, cx| a.decide(&p, Decision::Deny, cx))
        .unwrap();
    cx.run_until_parked();
    assert!(fx.store().is_denied(&fx.root(), "evil.example"));
    assert!(!fx.store().is_trusted(&fx.root(), "evil.example"));
    assert!(queue(cx, &m).is_empty());
    assert_eq!(fx.pending(), 0);
}

#[gpui_kit::test]
fn file_content_is_never_a_decision(cx: &mut TestAppContext) {
    let fx = Fx::new();
    fx.forge(
        "1-1.json",
        "evil.example",
        json!({ "decision": "trust", "approved": true }),
    );
    fx.forge("1-2.json", "API.Example.COM.", json!({}));
    fx.forge("1-3.json", "exa mple", json!({}));
    let bare = TempDir::new().unwrap(); // no opencollection.yml
    fx.forge(
        "1-4.json",
        "bare.example",
        json!({ "collection": bare.path() }),
    );
    let m = model(cx, &fx);
    assert!(
        !fx.trust_file().exists(),
        "reading the inbox wrote no trust"
    );
    assert_eq!(hosts(cx, &m), ["evil.example", "api.example.com"]);
    assert_eq!(
        fx.pending(),
        2,
        "the bad host and the non-collection are deleted"
    );
}

#[gpui_kit::test]
fn pairs_already_decided_are_resolved_without_a_prompt(cx: &mut TestAppContext) {
    let fx = Fx::new();
    fx.file("api.example.com", "a.yml");
    fx.file("evil.example", "a.yml");
    fx.file("new.example", "a.yml");
    let mut store = TrustStore::empty(fx.trust_file());
    store.trust(fx.coll.path(), "api.example.com").unwrap();
    store.deny(fx.coll.path(), "evil.example").unwrap();
    store.save().unwrap();
    let m = model(cx, &fx);
    assert_eq!(hosts(cx, &m), ["new.example"]);
    assert_eq!(fx.pending(), 1, "decided pairs' files are deleted");
}

#[gpui_kit::test]
fn requests_older_than_a_week_are_pruned_at_start(cx: &mut TestAppContext) {
    let fx = Fx::new();
    let old = fx.file("old.example", "a.yml");
    fx.file("new.example", "a.yml");
    let eight_days = SystemTime::now() - Duration::from_secs(8 * 86_400);
    fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(eight_days)
        .unwrap();
    let m = model(cx, &fx);
    assert_eq!(hosts(cx, &m), ["new.example"]);
    assert!(!old.exists());
}

#[gpui_kit::test]
fn deciding_a_pair_decided_elsewhere_writes_nothing(cx: &mut TestAppContext) {
    let fx = Fx::new();
    fx.file("api.example.com", "a.yml");
    let m = model(cx, &fx);
    let p = queue(cx, &m)[0].clone();
    let mut other = TrustStore::empty(fx.trust_file());
    other.deny(fx.coll.path(), "api.example.com").unwrap();
    other.save().unwrap();
    let before = fs::read(fx.trust_file()).unwrap();
    let got = m.update(cx, |a, cx| a.decide(&p, Decision::Trust, cx));
    assert_eq!(got, Ok(Outcome::AlreadyDecided(Decision::Deny)));
    assert_eq!(fs::read(fx.trust_file()).unwrap(), before);
    assert_eq!(fx.pending(), 0);
}

#[gpui_kit::test]
fn dismiss_removes_the_request_and_records_nothing(cx: &mut TestAppContext) {
    let fx = Fx::new();
    // A hand-written, non-canonical host: core's resolve would not match its raw text.
    fx.forge("1-1.json", "API.Example.COM.", json!({}));
    let m = model(cx, &fx);
    let p = queue(cx, &m)[0].clone();
    m.update(cx, |a, cx| a.dismiss(&p, cx));
    cx.run_until_parked();
    assert!(queue(cx, &m).is_empty());
    assert_eq!(fx.pending(), 0);
    assert!(!fx.trust_file().exists());
    // The agent's retry files a new request, which is asked again.
    fx.file("api.example.com", "a.yml");
    m.update(cx, |a, cx| a.refresh(cx));
    cx.run_until_parked();
    assert_eq!(hosts(cx, &m), ["api.example.com"]);
}

#[gpui_kit::test]
fn a_malformed_trust_file_is_never_overwritten(cx: &mut TestAppContext) {
    let fx = Fx::new();
    fx.file("api.example.com", "a.yml");
    let m = model(cx, &fx);
    fs::write(fx.trust_file(), "{ broken").unwrap();
    let p = queue(cx, &m)[0].clone();
    let result = m.update(cx, |a, cx| a.decide(&p, Decision::Trust, cx));
    cx.run_until_parked();
    assert!(result.is_err());
    assert_eq!(fs::read_to_string(fx.trust_file()).unwrap(), "{ broken");
    assert!(
        queue(cx, &m).is_empty(),
        "the pair is set aside for this run"
    );
    assert_eq!(fx.pending(), 1, "its request stays filed");
}
