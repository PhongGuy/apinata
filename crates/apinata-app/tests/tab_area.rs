use crate::support;

use apinata_app::command::Command;
use gpui_kit::AppContext as _;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt;
use support::*;

fn active(cx: &mut TestAppContext, a: &Area) -> Option<String> {
    a.ws.read_with(cx, |w, _| {
        w.tabs()
            .active()
            .and_then(|e| e.item())
            .map(|p| p.as_str().to_owned())
    })
}

fn close_at(cx: &mut TestAppContext, a: &Area, ix: usize) {
    cx.update_window(a.win, |_, w, cx| w.click(("tab-close", ix), cx))
        .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn clicking_a_tab_activates_it_and_shows_its_editor(cx: &mut TestAppContext) {
    let a = open_area(cx);
    assert!(present(cx, a.win, "tab-empty"));
    open_tab(cx, &a.ws, &a.root, "a.yml");
    open_tab(cx, &a.ws, &a.root, "b.yml");
    cx.run_until_parked();
    cx.update_window(a.win, |_, w, cx| w.click(("tab", 0usize), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(active(cx, &a).as_deref(), Some("a.yml"));
    assert!(present(cx, a.win, "url"));
}

#[gpui_kit::test]
fn preview_and_dirty_markers_show(cx: &mut TestAppContext) {
    let a = open_area(cx);
    a.ws.update(cx, |w, cx| w.open_item(&a.root, ip("a.yml"), true, cx));
    cx.run_until_parked();
    assert!(present(cx, a.win, ("preview", 0usize)));
    assert!(!present(cx, a.win, ("dirty", 0usize)));
    a.ws.update(cx, |w, cx| {
        w.send(&a.root, edit_url("a.yml", "http://x/1"), cx)
    });
    cx.run_until_parked();
    assert!(present(cx, a.win, ("dirty", 0usize)));
}

#[gpui_kit::test]
fn closing_a_dirty_tab_asks_first(cx: &mut TestAppContext) {
    let a = open_area(cx);
    open_tab(cx, &a.ws, &a.root, "a.yml");
    open_tab(cx, &a.ws, &a.root, "b.yml");
    cx.run_until_parked();
    close_at(cx, &a, 1);
    assert!(!confirm_open(cx));
    assert_eq!(tab_names(cx, &a.ws), ["a.yml"]);
    a.ws.update(cx, |w, cx| {
        w.send(&a.root, edit_url("a.yml", "http://x/1"), cx)
    });
    cx.run_until_parked();
    close_at(cx, &a, 0);
    answer_confirm(cx, "confirm-cancel");
    cx.run_until_parked();
    assert_eq!(tab_names(cx, &a.ws), ["a.yml"]);
    close_at(cx, &a, 0);
    answer_confirm(cx, "confirm-save");
    cx.run_until_parked();
    assert!(tab_names(cx, &a.ws).is_empty());
    let disk = std::fs::read_to_string(a.col.path().join("a.yml")).unwrap();
    assert!(disk.contains("http://x/1"), "{disk}");
    open_tab(cx, &a.ws, &a.root, "b.yml");
    a.ws.update(cx, |w, cx| {
        w.send(&a.root, edit_url("b.yml", "http://x/2"), cx)
    });
    cx.run_until_parked();
    close_at(cx, &a, 0);
    answer_confirm(cx, "confirm-discard");
    cx.run_until_parked();
    assert!(tab_names(cx, &a.ws).is_empty());
    let b = view_of(cx, &a.ws, &a.root, "b.yml");
    assert_eq!((b.url.as_str(), b.dirty), ("http://x/b", false));
}

#[gpui_kit::test]
fn a_failed_save_keeps_the_tab_open(cx: &mut TestAppContext) {
    let a = open_area(cx);
    open_tab(cx, &a.ws, &a.root, "a.yml");
    a.ws.update(cx, |w, cx| {
        w.send(&a.root, edit_url("a.yml", "http://x/1"), cx)
    });
    cx.run_until_parked();
    std::fs::write(a.col.path().join("a.yml"), request("a", "http://x/disk")).unwrap();
    close_at(cx, &a, 0);
    answer_confirm(cx, "confirm-save");
    cx.run_until_parked();
    assert_eq!(tab_names(cx, &a.ws), ["a.yml"]);
    assert!(view_of(cx, &a.ws, &a.root, "a.yml").dirty);
}

#[gpui_kit::test]
fn the_tab_menu_closes_others_and_saved(cx: &mut TestAppContext) {
    let a = open_area(cx);
    open_tab(cx, &a.ws, &a.root, "a.yml");
    open_tab(cx, &a.ws, &a.root, "b.yml");
    cx.run_until_parked();
    cx.update_window(a.win, |_, w, cx| w.right_click(("tab", 1usize), cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(a.win, |_, w, cx| w.within("popup-menu").click(1usize, cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(tab_names(cx, &a.ws), ["b.yml"]);
    a.ws.update(cx, |w, cx| {
        w.send(&a.root, edit_url("b.yml", "http://x/2"), cx)
    });
    open_tab(cx, &a.ws, &a.root, "a.yml");
    cx.run_until_parked();
    cx.update_window(a.win, |_, w, cx| w.right_click(("tab", 0usize), cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(a.win, |_, w, cx| w.within("popup-menu").click(3usize, cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(tab_names(cx, &a.ws), ["b.yml"]);
    assert!(!confirm_open(cx));
}

#[gpui_kit::test]
fn a_tab_removed_by_the_workspace_drops_its_editor(cx: &mut TestAppContext) {
    let a = open_area(cx);
    let id = open_tab(cx, &a.ws, &a.root, "a.yml");
    cx.run_until_parked();
    assert!(present(cx, a.win, "url"));
    assert!(a.view.read_with(cx, |v, _| v.editor(id).is_some()));
    std::fs::remove_file(a.col.path().join("a.yml")).unwrap();
    a.ws.update(cx, |w, cx| {
        w.send(
            &a.root,
            Command::Reload(vec![a.col.path().join("a.yml")]),
            cx,
        )
    });
    cx.run_until_parked();
    assert!(present(cx, a.win, "tab-empty"));
    assert!(a.view.read_with(cx, |v, _| v.editor(id).is_none()));
}

#[gpui_kit::test]
fn closing_another_tab_keeps_the_active_one(cx: &mut TestAppContext) {
    let a = open_area(cx);
    let c = a.col.path().join("c.yml");
    std::fs::write(&c, request("c", "http://x/c")).unwrap();
    a.ws.update(cx, |w, cx| w.send(&a.root, Command::Reload(vec![c]), cx));
    cx.run_until_parked();
    for rel in ["a.yml", "b.yml", "c.yml"] {
        open_tab(cx, &a.ws, &a.root, rel);
    }
    cx.run_until_parked();
    assert!(present(cx, a.win, "url"));
    close_at(cx, &a, 0);
    assert_eq!(tab_names(cx, &a.ws), ["b.yml", "c.yml"]);
    assert_eq!(active(cx, &a).as_deref(), Some("c.yml"));
}

#[gpui_kit::test]
fn escape_on_the_save_dialog_keeps_the_tab_and_the_file(cx: &mut TestAppContext) {
    let a = open_area(cx);
    open_tab(cx, &a.ws, &a.root, "a.yml");
    let file = a.col.path().join("a.yml");
    let before = std::fs::read(&file).unwrap();
    a.ws.update(cx, |w, cx| {
        w.send(&a.root, edit_url("a.yml", "http://x/1"), cx)
    });
    cx.run_until_parked();
    close_at(cx, &a, 0);
    let win = confirm_window(cx).expect("the save dialog");
    cx.update_window(win, |_, w, _| {
        for id in ["confirm-cancel", "confirm-discard", "confirm-save"] {
            assert!(w.try_find(id).is_some(), "{id} missing");
        }
    })
    .unwrap();
    cx.simulate_keystrokes(win, "escape");
    cx.run_until_parked();
    assert!(!confirm_open(cx));
    assert_eq!(tab_names(cx, &a.ws), ["a.yml"]);
    assert_eq!(std::fs::read(&file).unwrap(), before);
    let v = view_of(cx, &a.ws, &a.root, "a.yml");
    assert_eq!((v.url.as_str(), v.dirty), ("http://x/1", true));
}
