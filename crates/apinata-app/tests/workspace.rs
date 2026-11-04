use crate::support;

use apinata_app::command::{NoticeKind, Target};
use apinata_app::session::Session;
use apinata_app::workspace::WorkspaceEvent;
use gpui_kit::TestAppContext;
use support::*;

#[gpui_kit::test]
fn opening_lists_the_collection_and_remembers_it(cx: &mut TestAppContext) {
    let files = tempfile::tempdir().unwrap();
    let col = collection();
    let root = col.path().to_path_buf();
    let session = session_at(cx, &files, Session::default());
    let ws = workspace(cx, &session);
    let seen = events(cx, &ws);
    ws.update(cx, |w, cx| w.open(root.clone(), cx));
    cx.run_until_parked();
    let (n, loaded) = ws.read_with(cx, |w, cx| {
        (w.collections().len(), w.snapshot(&root, cx).is_some())
    });
    assert_eq!((n, loaded), (1, true));
    let s = session_value(cx, &session);
    assert_eq!(s.recent, std::slice::from_ref(&root));
    assert_eq!(s.open, std::slice::from_ref(&root));
    // The first snapshot also reports that its notices, none here, were delivered.
    let delivered = WorkspaceEvent::Delivered {
        root: root.clone(),
        acked: 0,
        generation: 0,
    };
    assert_eq!(*seen.borrow(), [WorkspaceEvent::Opened(root), delivered]);
}

#[gpui_kit::test]
fn opening_an_open_root_again_changes_nothing(cx: &mut TestAppContext) {
    let files = tempfile::tempdir().unwrap();
    let col = collection();
    let root = col.path().to_path_buf();
    let session = session_at(cx, &files, Session::default());
    let ws = workspace(cx, &session);
    let seen = events(cx, &ws);
    ws.update(cx, |w, cx| w.open(root.clone(), cx));
    cx.run_until_parked();
    ws.update(cx, |w, cx| w.open(root.clone(), cx));
    cx.run_until_parked();
    assert_eq!(ws.read_with(cx, |w, _| w.collections().len()), 1);
    // The first snapshot also reports that its notices, none here, were delivered.
    let delivered = WorkspaceEvent::Delivered {
        root: root.clone(),
        acked: 0,
        generation: 0,
    };
    assert_eq!(*seen.borrow(), [WorkspaceEvent::Opened(root), delivered]);
}

#[gpui_kit::test]
fn a_failed_open_is_reported_and_keeps_recents(cx: &mut TestAppContext) {
    let files = tempfile::tempdir().unwrap();
    let missing = files.path().join("gone");
    let start = Session {
        recent: vec![missing.clone()],
        open: vec![missing.clone()],
        ..Session::default()
    };
    let session = session_at(cx, &files, start);
    let ws = workspace(cx, &session);
    let seen = events(cx, &ws);
    ws.update(cx, |w, cx| w.open(missing.clone(), cx));
    cx.run_until_parked();
    assert!(ws.read_with(cx, |w, _| w.collections().is_empty()));
    let events = seen.borrow().clone();
    assert!(
        matches!(events.as_slice(), [WorkspaceEvent::OpenFailed { root, message }] if *root == missing && !message.is_empty()),
        "{events:?}"
    );
    let s = session_value(cx, &session);
    assert_eq!(s.recent, [missing]);
    assert!(s.open.is_empty());
}

#[gpui_kit::test]
fn close_forgets_the_open_root_but_not_the_recent_one(cx: &mut TestAppContext) {
    let files = tempfile::tempdir().unwrap();
    let col = collection();
    let root = col.path().to_path_buf();
    let session = session_at(cx, &files, Session::default());
    let ws = workspace(cx, &session);
    ws.update(cx, |w, cx| w.open(root.clone(), cx));
    cx.run_until_parked();
    ws.update(cx, |w, cx| w.close(&root, cx));
    cx.run_until_parked();
    assert!(ws.read_with(cx, |w, _| w.collections().is_empty()));
    let s = session_value(cx, &session);
    assert!(s.open.is_empty());
    assert_eq!(s.recent, [root]);
}

#[gpui_kit::test]
fn restore_opens_the_saved_roots(cx: &mut TestAppContext) {
    let files = tempfile::tempdir().unwrap();
    let col = collection();
    let root = col.path().to_path_buf();
    let missing = files.path().join("gone");
    let start = Session {
        open: vec![root.clone(), missing.clone()],
        ..Session::default()
    };
    let session = session_at(cx, &files, start);
    let ws = workspace(cx, &session);
    let seen = events(cx, &ws);
    ws.update(cx, |w, cx| w.restore(cx));
    cx.run_until_parked();
    let roots: Vec<_> = ws.read_with(cx, |w, _| {
        w.collections()
            .iter()
            .map(|c| c.root().to_path_buf())
            .collect()
    });
    assert_eq!(roots, std::slice::from_ref(&root));
    assert_eq!(
        session_value(cx, &session).open,
        std::slice::from_ref(&root)
    );
    let events = seen.borrow().clone();
    assert!(events.contains(&WorkspaceEvent::Opened(root)));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, WorkspaceEvent::OpenFailed { root, .. } if *root == missing))
    );
}

#[gpui_kit::test]
fn actor_notices_are_forwarded_once(cx: &mut TestAppContext) {
    let files = tempfile::tempdir().unwrap();
    let col = collection();
    let root = col.path().to_path_buf();
    let session = session_at(cx, &files, Session::default());
    let ws = workspace(cx, &session);
    let seen = events(cx, &ws);
    ws.update(cx, |w, cx| w.open(root.clone(), cx));
    cx.run_until_parked();
    ws.update(cx, |w, cx| {
        w.send(&root, edit_url("nope.yml", "http://x/1"), cx)
    });
    cx.run_until_parked();
    // A later command's snapshot must not carry the earlier notice again.
    ws.update(cx, |w, cx| {
        w.send(&root, edit_url("a.yml", "http://x/2"), cx)
    });
    cx.run_until_parked();
    let kinds: Vec<NoticeKind> = seen
        .borrow()
        .iter()
        .filter_map(|e| match e {
            WorkspaceEvent::Notices {
                root: r, notices, ..
            } if *r == root => Some(notices.iter().map(|n| n.kind.clone())),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(
        kinds,
        [NoticeKind::NotFound(Target::Request(ip("nope.yml")))]
    );
}

#[gpui_kit::test]
fn send_reaches_the_actor_of_an_open_root(cx: &mut TestAppContext) {
    let files = tempfile::tempdir().unwrap();
    let col = collection();
    let root = col.path().to_path_buf();
    let session = session_at(cx, &files, Session::default());
    let ws = workspace(cx, &session);
    ws.update(cx, |w, cx| w.open(root.clone(), cx));
    cx.run_until_parked();
    let id = ws.update(cx, |w, cx| {
        w.send(&root, edit_url("a.yml", "http://x/9"), cx)
    });
    assert_eq!(id, Some(1));
    let none = ws.update(cx, |w, cx| {
        w.send(
            std::path::Path::new("/nowhere"),
            edit_url("a.yml", "http://x/1"),
            cx,
        )
    });
    assert_eq!(none, None);
    cx.run_until_parked();
    let s = ws
        .read_with(cx, |w, cx| w.snapshot(&root, cx))
        .expect("opened");
    assert_eq!(url(&s, "a.yml"), "http://x/9");
}

#[gpui_kit::test]
fn a_responsive_actor_never_shows_busy(cx: &mut TestAppContext) {
    let files = tempfile::tempdir().unwrap();
    let col = collection();
    let root = col.path().to_path_buf();
    let session = session_at(cx, &files, Session::default());
    let ws = workspace(cx, &session);
    ws.update(cx, |w, cx| w.open(root.clone(), cx));
    cx.run_until_parked();
    for u in ["http://x/1", "http://x/2", "http://x/3"] {
        ws.update(cx, |w, cx| w.send(&root, edit_url("a.yml", u), cx));
    }
    assert!(!ws.read_with(cx, |w, _| w.is_busy(&root)));
    cx.run_until_parked();
    cx.executor()
        .advance_clock(apinata_app::busy::BUSY_AFTER * 2);
    cx.run_until_parked();
    assert!(!ws.read_with(cx, |w, _| w.is_busy(&root)));
    assert!(!ws.read_with(cx, |w, _| w.is_busy(std::path::Path::new("/nowhere"))));
}
