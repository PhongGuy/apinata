//! The approval prompt says which proxy a refused proxy host belongs to.

use crate::support;

use apinata_app::app::AppState;
use apinata_core::{ApprovalInbox, TrustStore};
use gpui_kit::TestAppContext;
use support::*;

fn file(cx: &mut TestAppContext, s: &Started, host: &str, proxy: Option<&str>) {
    let store = TrustStore::empty(s.files.path().join("trusted-hosts.json"));
    ApprovalInbox::new(s.files.path().join("approvals"))
        .request_via(&store, &s.root, host, "a.yml", proxy)
        .unwrap();
    let approvals = cx.update(|cx| cx.global::<AppState>().approvals.clone());
    approvals.update(cx, |a, cx| a.refresh(cx));
    cx.run_until_parked();
    frame(cx, s.win);
}

#[gpui_kit::test]
fn a_proxy_approval_says_which_proxy(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    file(
        cx,
        &s,
        "proxy.corp.example",
        Some("http://proxy.corp.example:3128"),
    );
    let shown = s
        .shell
        .read_with(cx, |v, _| v.approval_shown().cloned())
        .expect("a prompt");
    assert_eq!(
        shown.proxy.as_deref(),
        Some("http://proxy.corp.example:3128")
    );
    assert!(present(cx, s.win, "approval-host") && present(cx, s.win, "approval-proxy"));
}

#[gpui_kit::test]
fn a_host_approval_has_no_proxy_line(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    file(cx, &s, "api.example.com", None);
    let shown = s
        .shell
        .read_with(cx, |v, _| v.approval_shown().cloned())
        .expect("a prompt");
    assert_eq!(shown.proxy, None);
    assert!(present(cx, s.win, "approval-host") && !present(cx, s.win, "approval-proxy"));
}

/// Writes an inbox file by hand, since filing refuses a second request for one pair.
fn raw_file(s: &Started, name: &str, host: &str, proxy: Option<&str>) {
    let dir = s.files.path().join("approvals");
    std::fs::create_dir_all(&dir).unwrap();
    let collection = std::fs::canonicalize(&s.root).unwrap();
    let mut body = serde_json::json!({
        "version": 1,
        "collection": collection.to_string_lossy(),
        "host": host,
        "request": "a.yml",
        "requested_at": "2026-10-11T00:00:00Z",
    });
    if let Some(proxy) = proxy {
        body["proxy"] = proxy.into();
    }
    std::fs::write(dir.join(name), body.to_string()).unwrap();
}

#[gpui_kit::test]
fn a_later_file_supplies_the_proxy_an_earlier_one_lacks(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    raw_file(&s, "1-1.json", "proxy.corp.example", None);
    raw_file(
        &s,
        "1-2.json",
        "proxy.corp.example",
        Some("http://proxy.corp.example:3128"),
    );
    let approvals = cx.update(|cx| cx.global::<AppState>().approvals.clone());
    approvals.update(cx, |a, cx| a.refresh(cx));
    cx.run_until_parked();
    frame(cx, s.win);
    let shown = s
        .shell
        .read_with(cx, |v, _| v.approval_shown().cloned())
        .expect("a prompt");
    assert_eq!(
        shown.proxy.as_deref(),
        Some("http://proxy.corp.example:3128")
    );
}

#[gpui_kit::test]
fn a_mapped_approval_shows_the_name_beside_the_address(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let store = TrustStore::empty(s.files.path().join("trusted-hosts.json"));
    ApprovalInbox::new(s.files.path().join("approvals"))
        .request_mapped(&store, &s.root, "203.0.113.5", "a.yml", "api.example.com")
        .unwrap();
    let approvals = cx.update(|cx| cx.global::<AppState>().approvals.clone());
    approvals.update(cx, |a, cx| a.refresh(cx));
    cx.run_until_parked();
    frame(cx, s.win);
    let shown = s
        .shell
        .read_with(cx, |v, _| v.approval_shown().cloned())
        .expect("a prompt");
    assert_eq!(shown.host, "203.0.113.5");
    assert_eq!(shown.mapped_from.as_deref(), Some("api.example.com"));
    assert!(present(cx, s.win, "approval-mapped"));
}

#[gpui_kit::test]
fn an_unmapped_approval_has_no_mapped_line(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    file(cx, &s, "api.example.com", None);
    assert!(!present(cx, s.win, "approval-mapped"));
}
