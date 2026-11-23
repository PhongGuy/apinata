//! The AI agents (MCP) page in Preferences: the connect command and each open collection's
//! AI access link.

use crate::support;

use apinata_app::config_editor::ConfigSection;
use apinata_app::tabs::TabKind;
use gpui_kit::component::WindowExt;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, ElementId, TestAppContext};
use support::*;

fn open_prefs(cx: &mut TestAppContext, s: &Started) {
    cx.update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap();
    frame(cx, s.win);
}

fn search(cx: &mut TestAppContext, s: &Started, text: &str) {
    replace_input(cx, s.win, None, "prefs-search", text);
    frame(cx, s.win);
}

fn drawn_text(cx: &mut TestAppContext, s: &Started, id: impl Into<ElementId>) -> String {
    let id = id.into();
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        gpui_kit::base::test_support::texts(w)
            .into_iter()
            .filter(|t| t.path().contains(&id))
            .map(|t| t.text().to_string())
            .collect::<Vec<_>>()
            .join(" ")
    })
    .unwrap()
}

#[gpui_kit::test]
fn searching_mcp_ai_agent_or_trust_finds_the_page(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    for query in ["mcp", "MCP", "ai", "agent", "trust"] {
        search(cx, &s, query);
        assert!(present(cx, s.win, "prefs-mcp"), "{query:?}");
        assert!(!present(cx, s.win, "prefs-no-match"), "{query:?}");
    }
}

#[gpui_kit::test]
fn the_page_shows_the_connect_command_and_one_link_per_open_collection(cx: &mut TestAppContext) {
    let (one, two) = (two_requests(), two_requests());
    let s = started_with_session(cx, &[one.path(), two.path()]);
    open_prefs(cx, &s);
    search(cx, &s, "mcp");
    assert!(
        drawn_text(cx, &s, "mcp-command").contains("apinata mcp --collection"),
        "{}",
        drawn_text(cx, &s, "mcp-command")
    );
    assert!(present(cx, s.win, ("prefs-mcp-access", 0usize)));
    assert!(present(cx, s.win, ("prefs-mcp-access", 1usize)));
    assert!(!present(cx, s.win, ("prefs-mcp-access", 2usize)));
    // The page never claims a connection it cannot see.
    let note = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.find("prefs-mcp-note").label().map(str::to_lowercase)
        })
        .unwrap()
        .unwrap_or_default();
    assert!(note.contains("approval"), "{note}");
    assert!(!note.contains("connected"), "{note}");
}

#[gpui_kit::test]
fn a_collection_link_opens_its_ai_access_tab(cx: &mut TestAppContext) {
    let (one, two) = (two_requests(), two_requests());
    let s = started_with_session(cx, &[one.path(), two.path()]);
    open_prefs(cx, &s);
    search(cx, &s, "trust");
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.click(("prefs-mcp-access", 1usize), cx);
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    assert!(
        !cx.update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
            .unwrap()
    );
    let (root, kind, tab) = s.ws.read_with(cx, |w, _| {
        let t = w.tabs().active().unwrap();
        (t.root.clone(), t.kind.clone(), t.id)
    });
    assert_eq!(kind, TabKind::Collection);
    assert_eq!(
        std::fs::canonicalize(root).unwrap(),
        std::fs::canonicalize(two.path()).unwrap()
    );
    let ed = s
        .shell
        .read_with(cx, |v, cx| {
            v.tab_area().read(cx).config_editor(tab).cloned()
        })
        .unwrap();
    assert_eq!(ed.read_with(cx, |e, _| e.section()), ConfigSection::Hosts);
    assert!(present(cx, s.win, "config-hosts"));
}

#[gpui_kit::test]
fn a_collection_shows_how_many_requests_wait_for_approval(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let store = apinata_core::TrustStore::empty(s.files.path().join("trusted-hosts.json"));
    let inbox = apinata_core::ApprovalInbox::new(s.files.path().join("approvals"));
    for host in ["a.example", "b.example"] {
        inbox.request(&store, &s.root, host, "a.yml").unwrap();
    }
    let approvals = cx.update(|cx| cx.global::<apinata_app::app::AppState>().approvals.clone());
    approvals.update(cx, |a, cx| a.refresh(cx));
    frame(cx, s.win);
    open_prefs(cx, &s);
    search(cx, &s, "mcp");
    assert_eq!(
        drawn_text(cx, &s, ("prefs-mcp-pending", 0usize)),
        "2 requests waiting for approval"
    );
}
