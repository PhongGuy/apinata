use crate::support;

use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, point};
use support::*;

fn toasts(cx: &mut TestAppContext, s: &Started) -> usize {
    cx.update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

// Moving out of a read-only directory fails for that member while another member succeeds.
#[cfg(unix)]
#[gpui_kit::test]
fn a_partial_move_failure_leaves_one_toast_and_the_failed_row_selected(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("A", "http://x/a")),
        ("t/folder.yml", &folder("T")),
        ("ro/folder.yml", &folder("Ro")),
        ("ro/x.yml", &request("X", "http://x/x")),
    ]);
    let ro = col.path().join("ro");
    let _g = crate::support::ModeGuard::set(&ro, 0o555);
    if std::fs::File::create(ro.join("probe")).is_ok() {
        eprintln!("skipped: permissions are not enforced for this user");
        return;
    }
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| w.click(row("ro"), cx))
        .unwrap(); // expands ro
    cx.update_window(s.win, |_, w, cx| w.click(row("a.yml"), cx))
        .unwrap();
    secondary_click(cx, s.win, row("ro/x.yml"));
    let before = toasts(cx, &s);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let start = w.find(row("a.yml")).bounds().center();
        let b = w.find(row("t")).bounds();
        w.drag(start, point(b.center().x, b.center().y), cx);
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    assert!(
        col.path().join("t/a.yml").exists(),
        "the movable member moved"
    );
    assert!(
        col.path().join("ro/x.yml").exists(),
        "the failed member stayed"
    );
    assert_eq!(toasts(cx, &s), before + 1, "one summary toast");
    let sidebar = s.shell.read_with(cx, |sh, _| sh.sidebar().clone());
    let keys: Vec<String> = sidebar.read_with(cx, |sb, _| {
        sb.multi_keys()
            .into_iter()
            .map(|(_, p)| p.as_str().to_owned())
            .collect()
    });
    assert!(keys.contains(&"ro/x.yml".to_owned()), "{keys:?}");
}

#[gpui_kit::test]
fn a_clean_multi_move_pushes_no_toast(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("A", "http://x/a")),
        ("b.yml", &request("B", "http://x/b")),
        ("t/folder.yml", &folder("T")),
    ]);
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| w.click(row("a.yml"), cx))
        .unwrap();
    secondary_click(cx, s.win, row("b.yml"));
    let before = toasts(cx, &s);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let start = w.find(row("b.yml")).bounds().center();
        let b = w.find(row("t")).bounds();
        w.drag(start, point(b.center().x, b.center().y), cx);
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    assert!(col.path().join("t/a.yml").exists() && col.path().join("t/b.yml").exists());
    assert_eq!(toasts(cx, &s), before);
}
