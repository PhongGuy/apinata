//! The title bar's branch label: the branch name that opens the branch dialog, the
//! changed-file count that opens the Changes dialog, and a commit made elsewhere picked up on
//! activation.

use crate::git_support;
use crate::support;

use std::path::PathBuf;

use apinata_app::branch_dialog;
use apinata_app::changes_dialog::current;
use apinata_app::git_status::{GitChange, GitChanges, branch_text};
use git_support::{Sandbox, write};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext};
use support::*;

fn committed(sb: &Sandbox, files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = write_tree(files);
    sb.git(dir.path(), &["init", "-q", "-b", "main"]);
    sb.commit_all(dir.path(), "init");
    dir
}

/// The real app over a repository whose `a.yml` changed, with git on.
fn one_changed(cx: &mut TestAppContext, sb: &Sandbox) -> (tempfile::TempDir, Started) {
    let col = committed(
        sb,
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &request("a", "http://x/a")),
        ],
    );
    write(col.path(), "a.yml", &request("a", "http://x/changed"));
    let s = started(cx, &col);
    let options = sb.options();
    s.ws.update(cx, |w, cx| w.set_git(Some(options), cx));
    frame(cx, s.win);
    (col, s)
}

fn branch(cx: &mut TestAppContext, s: &Started) -> Option<String> {
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        w.within("title-bar")
            .try_find("git-label")
            .and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

#[test]
fn the_text_joins_the_branch_and_the_count() {
    let mut g = GitChanges::default();
    assert_eq!(branch_text(Some("main"), Some(&g)).as_deref(), Some("main"));
    assert_eq!(branch_text(Some("main"), None).as_deref(), Some("main"));
    assert_eq!(branch_text(None, None), None);
    g.files.insert(PathBuf::from("a.yml"), GitChange::Modified);
    assert_eq!(
        branch_text(Some("main"), Some(&g)).as_deref(),
        Some("main, 1 changed")
    );
    assert_eq!(branch_text(None, Some(&g)).as_deref(), Some("1 changed"));
}

/// Whether the branch dialog is up, after letting its listing settle.
fn branch_dialog_open(cx: &mut TestAppContext, s: &Started) -> bool {
    frame(cx, s.win);
    cx.update(|cx| branch_dialog::current(cx)).is_some() && present(cx, s.win, "branch-dialog")
}

#[gpui_kit::test]
fn clicking_the_count_opens_the_changes(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_col, s) = one_changed(cx, &sb);
    assert_eq!(branch(cx, &s).as_deref(), Some("main, 1 changed"));
    let count = cx
        .update_window(s.win, |_, w, _| {
            w.within("title-bar")
                .try_find("git-changes")
                .and_then(|e| e.label().map(str::to_owned))
        })
        .unwrap();
    assert_eq!(count.as_deref(), Some("1 changed"));
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("git-changes", cx)
    })
    .unwrap();
    frame(cx, s.win);
    assert!(cx.update(|cx| branch_dialog::current(cx)).is_none());
    let selected = cx.update(|cx| {
        current(cx).and_then(|v| {
            v.read(cx)
                .selected()
                .map(|p| p.to_string_lossy().into_owned())
        })
    });
    assert_eq!(selected.as_deref(), Some("a.yml"));
}

#[gpui_kit::test]
fn a_commit_made_elsewhere_shows_on_activation(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (col, s) = one_changed(cx, &sb);
    assert_eq!(branch(cx, &s).as_deref(), Some("main, 1 changed"));
    sb.commit_all(col.path(), "outside");
    let mut vcx = VisualTestContext::from_window(s.win, cx);
    vcx.deactivate_window();
    cx.update_window(s.win, |_, w, _| w.activate_window())
        .unwrap();
    cx.run_until_parked();
    assert_eq!(branch(cx, &s).as_deref(), Some("main"));
}

#[gpui_kit::test]
fn clicking_the_counted_branch_name_opens_the_branch_dialog(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    cx.executor().allow_parking();
    let (_col, s) = one_changed(cx, &sb);
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("git-branch", cx)
    })
    .unwrap();
    assert!(branch_dialog_open(cx, &s));
    assert!(cx.update(|cx| current(cx)).is_none(), "Changes stays shut");
}

#[gpui_kit::test]
fn a_clean_label_opens_the_branch_dialog(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    cx.executor().allow_parking();
    let col = committed(
        &sb,
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &request("a", "http://x/a")),
        ],
    );
    let s = started(cx, &col);
    let options = sb.options();
    s.ws.update(cx, |w, cx| w.set_git(Some(options), cx));
    assert_eq!(branch(cx, &s).as_deref(), Some("main"));
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("git-branch", cx)
    })
    .unwrap();
    assert!(branch_dialog_open(cx, &s));
    assert!(cx.update(|cx| current(cx)).is_none(), "Changes stays shut");
}

// In a narrow or zoomed window the branch label keeps a gap before the environment control,
// and the environment select shrinks inside its control, so its own text ellipsis shows
// instead of the control clipping it.
#[gpui_kit::test]
fn the_branch_label_and_environment_select_fit_a_narrow_bar(cx: &mut TestAppContext) {
    use apinata_app::style::{SPACE_2, at_zoom};
    use gpui_kit::{px, size};
    let sb = Sandbox::new();
    let (_col, s) = one_changed(cx, &sb);
    for (w, h, zoom) in [(320., 700., 1.0), (1280., 800., 2.0), (640., 700., 1.0)] {
        VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(w), px(h)));
        cx.update(|cx| {
            let p = cx.global::<apinata_app::app::AppState>().settings.clone();
            p.update(cx, |p, cx| p.update(cx, |s| s.zoom = zoom));
        });
        frame(cx, s.win);
        frame(cx, s.win);
        let (branch, env, select) = cx
            .update_window(s.win, |_, win, cx| {
                win.render_frame(cx);
                let bar = win.within("title-bar");
                (
                    bar.find("git-label").bounds(),
                    bar.find("env-control").bounds(),
                    bar.find("env-select").bounds(),
                )
            })
            .unwrap();
        assert!(
            branch.right() + at_zoom(SPACE_2, zoom) <= env.left(),
            "{w}x{h} at {zoom}: branch {branch:?} touches env {env:?}"
        );
        assert!(
            select.right() <= env.right() + px(0.5),
            "{w}x{h} at {zoom}: select {select:?} overflows env {env:?}"
        );
    }
}

fn opens_by(cx: &mut TestAppContext, key: &str) {
    let sb = Sandbox::new();
    cx.executor().allow_parking();
    let (_col, s) = one_changed(cx, &sb);
    assert_eq!(branch(cx, &s).as_deref(), Some("main, 1 changed"));
    tab_to(cx, s.win, "git-branch");
    cx.update_window(s.win, |_, w, cx| w.press(key, cx))
        .unwrap();
    assert!(branch_dialog_open(cx, &s), "{key} opens the branch dialog");
}

#[gpui_kit::test]
fn enter_on_the_counted_label_opens_the_branch_dialog(cx: &mut TestAppContext) {
    opens_by(cx, "enter");
}

#[gpui_kit::test]
fn space_on_the_counted_label_opens_the_branch_dialog(cx: &mut TestAppContext) {
    opens_by(cx, "space");
}

fn count_opens_by(cx: &mut TestAppContext, key: &str) {
    let sb = Sandbox::new();
    let (_col, s) = one_changed(cx, &sb);
    tab_to(cx, s.win, "git-changes");
    cx.update_window(s.win, |_, w, cx| w.press(key, cx))
        .unwrap();
    frame(cx, s.win);
    assert!(cx.update(|cx| current(cx)).is_some(), "{key} opens Changes");
}

#[gpui_kit::test]
fn enter_on_the_count_opens_the_changes(cx: &mut TestAppContext) {
    count_opens_by(cx, "enter");
}

#[gpui_kit::test]
fn space_on_the_count_opens_the_changes(cx: &mut TestAppContext) {
    count_opens_by(cx, "space");
}

#[gpui_kit::test]
fn a_clean_label_is_a_tab_stop(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = committed(
        &sb,
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &request("a", "http://x/a")),
        ],
    );
    let s = started(cx, &col);
    let options = sb.options();
    s.ws.update(cx, |w, cx| w.set_git(Some(options), cx));
    assert_eq!(branch(cx, &s).as_deref(), Some("main"));
    tab_to(cx, s.win, "git-branch");
}

// The counted label's focus ring draws whole: it sits on the button's own edge, so no
// ancestor's clip can cut it, and nothing paints it narrower than the button.
#[gpui_kit::test]
fn the_counted_label_draws_its_whole_focus_ring(cx: &mut TestAppContext) {
    for id in ["git-branch", "git-changes"] {
        let sb = Sandbox::new();
        let (_col, s) = one_changed(cx, &sb);
        tab_to(cx, s.win, id);
        assert_whole_ring(cx, &s, id);
    }
}

fn assert_whole_ring(cx: &mut TestAppContext, s: &Started, id: &'static str) {
    cx.update(|cx| apinata_app::focus_mode::set(apinata_app::focus_mode::Modality::Keyboard, cx));
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let scale = w.scale_factor();
        let b = w.within("title-bar").find(id).bounds();
        let (left, top) = (b.left().as_f32() * scale, b.top().as_f32() * scale);
        let (right, bottom) = (b.right().as_f32() * scale, b.bottom().as_f32() * scale);
        let rings: Vec<_> = w
            .painted_quads()
            .into_iter()
            .filter(|q| {
                let qb = q.bounds;
                q.border_widths.left.0 > 0.
                    && (qb.origin.x.0 - left).abs() < 1.
                    && (qb.origin.y.0 - top).abs() < 1.
                    && ((qb.origin.x + qb.size.width).0 - right).abs() < 1.
                    && ((qb.origin.y + qb.size.height).0 - bottom).abs() < 1.
            })
            .collect();
        assert!(!rings.is_empty(), "{id}: a ring paints on {b:?}");
        // The band paints as one masked quad per side; together the masks span the ring.
        let ring = rings[0].bounds;
        let mask = rings
            .iter()
            .filter(|q| q.bounds == ring)
            .map(|q| q.content_mask.bounds)
            .reduce(|a, b| a.union(&b))
            .unwrap();
        assert!(
            mask.origin.x <= ring.origin.x
                && mask.origin.y <= ring.origin.y
                && (mask.origin.x + mask.size.width) >= (ring.origin.x + ring.size.width)
                && (mask.origin.y + mask.size.height) >= (ring.origin.y + ring.size.height),
            "{id}: the ring {ring:?} is clipped to {mask:?}"
        );
    })
    .unwrap();
}

// At 320 px the branch label truncates first; the environment selects, the Environments
// button and the app menu all stay inside the window.
#[gpui_kit::test]
fn a_320_px_bar_truncates_the_branch_and_keeps_its_controls_in_the_window(cx: &mut TestAppContext) {
    use gpui_kit::{px, size};
    let sb = Sandbox::new();
    let (_col, s) = one_changed(cx, &sb);
    let measure = |cx: &mut TestAppContext, w: f32| {
        VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(w), px(700.)));
        frame(cx, s.win);
        frame(cx, s.win);
        cx.update_window(s.win, |_, win, cx| {
            win.render_frame(cx);
            let whole = gpui_kit::Bounds::new(gpui_kit::Point::default(), win.viewport_size());
            let bar = win.within("title-bar");
            let ids = [
                "collection-switcher",
                "git-label",
                "env-control",
                "global-env-selector",
                "environments",
                "app-menu",
            ];
            (whole, ids.map(|id| bar.find(id).bounds()))
        })
        .unwrap()
    };
    let (_, wide) = measure(cx, 1280.);
    let (bar, narrow) = measure(cx, 320.);
    for (id, b) in [
        "env-control",
        "global-env-selector",
        "environments",
        "app-menu",
    ]
    .iter()
    .zip(&narrow[2..])
    {
        assert!(
            b.left() >= bar.left() && b.right() <= bar.right() + px(0.5),
            "{id} {b:?} lies outside the window {bar:?}"
        );
    }
    assert!(
        narrow[1].size.width < wide[1].size.width,
        "the branch label truncates: {:?} vs {:?}",
        narrow[1],
        wide[1]
    );
    assert_eq!(
        narrow[0].size.width, wide[0].size.width,
        "the collection name keeps its width while the branch label can still shrink"
    );
}

// A long collection name at 320 px: once the branch label has shrunk away the name
// truncates inside the bar instead of running under the environment control.
#[gpui_kit::test]
fn a_long_collection_name_truncates_after_the_branch(cx: &mut TestAppContext) {
    use gpui_kit::{px, size};
    let sb = Sandbox::new();
    let root = "opencollection: 1.0.0\ninfo:\n  name: A collection with a rather long name\n";
    let col = committed(
        &sb,
        &[
            ("opencollection.yml", root),
            ("a.yml", &request("a", "http://x/a")),
        ],
    );
    write(col.path(), "a.yml", &request("a", "http://x/changed"));
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| w.set_git(Some(sb.options()), cx));
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(320.), px(700.)));
    frame(cx, s.win);
    frame(cx, s.win);
    let (switcher, branch, env) = cx
        .update_window(s.win, |_, win, cx| {
            win.render_frame(cx);
            let bar = win.within("title-bar");
            (
                bar.find("collection-switcher").bounds(),
                bar.find("git-label").bounds(),
                bar.find("env-control").bounds(),
            )
        })
        .unwrap();
    assert!(
        switcher.right() <= env.left(),
        "the name {switcher:?} runs under the environment control {env:?}"
    );
    assert!(
        branch.size.width < px(1.) || branch.left() >= env.left(),
        "the branch label {branch:?} is gone before the name truncates"
    );
}

// The row laid over the kit's title bar takes no mouse hits of its own: a drag from an empty
// spot between the groups still reaches the bar's window-move handling. The test platform
// leaves the window move unimplemented, so reaching it panics with that message.
#[gpui_kit::test]
fn a_drag_on_an_empty_title_bar_spot_reaches_the_window_move(cx: &mut TestAppContext) {
    use gpui_kit::{point, px};
    let sb = Sandbox::new();
    let (_col, s) = one_changed(cx, &sb);
    let (branch, env) = cx
        .update_window(s.win, |_, win, cx| {
            win.render_frame(cx);
            let bar = win.within("title-bar");
            (
                bar.find("git-label").bounds(),
                bar.find("env-control").bounds(),
            )
        })
        .unwrap();
    assert!(
        env.left() - branch.right() > px(40.),
        "an empty gap lies between the groups"
    );
    let y = branch.center().y;
    let x = (branch.right() + env.left()) / 2.;
    let drag = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cx.update_window(s.win, |_, win, cx| {
            win.drag(point(x, y), point(x + px(20.), y), cx)
        })
        .unwrap();
    }));
    let message = drag
        .err()
        .and_then(|p| {
            p.downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
        })
        .unwrap_or_default();
    assert!(
        message.contains("not implemented"),
        "the drag never reached the kit bar's window move: {message:?}"
    );
}

// At 320 px the counted label shrinks to nothing; a Tab stop with no box would take focus
// and draw its ring as a sliver, so Tab skips both its parts while it has no room.
#[gpui_kit::test]
fn a_counted_label_with_no_room_is_no_tab_stop(cx: &mut TestAppContext) {
    use gpui_kit::{px, size};
    let sb = Sandbox::new();
    let (_col, s) = one_changed(cx, &sb);
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(320.), px(700.)));
    frame(cx, s.win);
    frame(cx, s.win);
    let width = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.within("title-bar").find("git-label").bounds().size.width
        })
        .unwrap();
    assert!(width < px(8.), "the label has no room: {width:?}");
    for _ in 0..120 {
        cx.update_window(s.win, |_, w, cx| w.focus_next(cx))
            .unwrap();
        for id in ["git-branch", "git-changes"] {
            let on = cx
                .update_window(s.win, |_, w, cx| {
                    w.render_frame(cx);
                    w.find(id).focused()
                })
                .unwrap();
            assert_ne!(on, Some(true), "{id}: a label with no room takes no focus");
        }
    }
}

/// Left and right edge of each id in the title bar, `None` for one not drawn or of no width.
fn spans<const N: usize>(
    cx: &mut TestAppContext,
    s: &Started,
    ids: [&'static str; N],
) -> [Option<(f32, f32)>; N] {
    frame(cx, s.win);
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let bar = w.within("title-bar");
        ids.map(|id| {
            bar.try_find(id)
                .map(|e| e.bounds())
                .filter(|b| b.size.width.as_f32() >= 1.)
                .map(|b| (b.left().as_f32(), b.right().as_f32()))
        })
    })
    .unwrap()
}

// The separator comma is its own text between the two buttons, so the name's focus ring hugs
// the bare name; the joined text stays the label's accessible name.
#[gpui_kit::test]
fn the_comma_sits_between_the_name_and_the_count(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_col, s) = one_changed(cx, &sb);
    assert_eq!(branch(cx, &s).as_deref(), Some("main, 1 changed"));
    let [name, text, comma, count] = spans(
        cx,
        &s,
        [
            "git-branch",
            "git-branch-text",
            "git-label-comma",
            "git-changes",
        ],
    );
    let (name, count) = (name.expect("the name"), count.expect("the count"));
    let text = text.expect("the name's text");
    let comma = comma.expect("a comma is drawn between the name and the count");
    // The comma follows the name as in running text, not a padding's width away.
    assert!(
        comma.0 - text.1 <= 4.,
        "the comma {comma:?} floats {} px after the name's text {text:?}",
        comma.0 - text.1
    );
    assert!(
        name.1 <= comma.0 + 0.5,
        "the name {name:?} covers the comma {comma:?}"
    );
    assert!(
        comma.1 <= count.0 + 0.5,
        "the comma {comma:?} runs into the count {count:?}"
    );
    let label = cx
        .update_window(s.win, |_, w, _| {
            w.within("title-bar")
                .find("git-branch")
                .label()
                .map(str::to_owned)
        })
        .unwrap();
    assert_eq!(label.as_deref(), Some("main"));
    // The name's text is as wide as a clean label's, so no comma sits inside its ring.
    let clean = committed(
        &sb,
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &request("a", "http://x/a")),
        ],
    );
    let c = started(cx, &clean);
    let options = sb.options();
    c.ws.update(cx, |w, cx| w.set_git(Some(options), cx));
    assert_eq!(branch(cx, &c).as_deref(), Some("main"));
    let [bare] = spans(cx, &c, ["git-branch-text"]);
    let bare = bare.expect("the clean name's text");
    assert!(
        (text.1 - text.0 - (bare.1 - bare.0)).abs() < 0.5,
        "the counted name's text {text:?} is wider than the bare name's {bare:?}"
    );
}

// Once the count hides in a narrow bar, no stray comma trails the name.
#[gpui_kit::test]
fn a_hidden_count_leaves_no_comma(cx: &mut TestAppContext) {
    use gpui_kit::{px, size};
    let sb = Sandbox::new();
    let (_col, s) = one_changed(cx, &sb);
    let mut seen = false;
    for w in (300..=700).rev().step_by(4) {
        VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(w as f32), px(700.)));
        let [name, comma, count] = spans(cx, &s, ["git-branch", "git-label-comma", "git-changes"]);
        if name.is_some() && count.is_none() {
            seen = true;
            assert_eq!(comma, None, "{w}: a comma trails the name without a count");
        }
    }
    assert!(seen, "the sweep never hid the count alone");
}

// Changes opened from the count hands focus back to the count when it closes, as any dialog
// does to its opener, whether it opened by click or by key.
fn focus_returns_to_the_count(cx: &mut TestAppContext, open: &str, close: &str) {
    let sb = Sandbox::new();
    let (_col, s) = one_changed(cx, &sb);
    match open {
        "click" | "click from the name" => {
            if open == "click from the name" {
                tab_to(cx, s.win, "git-branch");
            }
            cx.update_window(s.win, |_, w, cx| {
                w.render_frame(cx);
                w.within("title-bar").click("git-changes", cx)
            })
            .unwrap()
        }
        key => {
            tab_to(cx, s.win, "git-changes");
            cx.update_window(s.win, |_, w, cx| w.press(key, cx))
                .unwrap();
        }
    }
    frame(cx, s.win);
    assert!(
        cx.update(|cx| current(cx)).is_some(),
        "{open} opens Changes"
    );
    match close {
        "escape" => cx
            .update_window(s.win, |_, w, cx| w.press("escape", cx))
            .unwrap(),
        _ => cx
            .update_window(s.win, |_, w, cx| {
                w.render_frame(cx);
                w.click("changes-close", cx)
            })
            .unwrap(),
    }
    // Let the close animation and its deferred focus restore finish over drawn frames.
    for _ in 0..4 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(300));
        frame(cx, s.win);
    }
    assert!(
        cx.update(|cx| current(cx)).is_none(),
        "{close} closes Changes"
    );
    let [count_on, name_on] = ["git-changes", "git-branch"].map(|id| {
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.within("title-bar").find(id).focused()
        })
        .unwrap()
    });
    assert_eq!(
        (count_on, name_on),
        (Some(true), Some(false)),
        "{open} then {close}: focus is back on the count, not the name"
    );
}

#[gpui_kit::test]
fn escape_after_clicking_the_count_focuses_the_count(cx: &mut TestAppContext) {
    focus_returns_to_the_count(cx, "click", "escape");
}

// The QA case: the name held focus from an earlier Tab, which the click used to restore.
#[gpui_kit::test]
fn escape_after_clicking_the_count_past_the_focused_name_focuses_the_count(
    cx: &mut TestAppContext,
) {
    focus_returns_to_the_count(cx, "click from the name", "escape");
}

#[gpui_kit::test]
fn escape_after_enter_on_the_count_focuses_the_count(cx: &mut TestAppContext) {
    focus_returns_to_the_count(cx, "enter", "escape");
}

#[gpui_kit::test]
fn closing_after_space_on_the_count_focuses_the_count(cx: &mut TestAppContext) {
    focus_returns_to_the_count(cx, "space", "close");
}

// The name's return is unchanged: a press moves no focus, so a name that held it before the
// click gets it back on Escape (a click from elsewhere returns there; see focus_survives).
#[gpui_kit::test]
fn escape_after_clicking_the_focused_name_focuses_the_name(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    cx.executor().allow_parking();
    let (_col, s) = one_changed(cx, &sb);
    tab_to(cx, s.win, "git-branch");
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.within("title-bar").click("git-branch", cx)
    })
    .unwrap();
    assert!(branch_dialog_open(cx, &s));
    cx.update_window(s.win, |_, w, cx| w.press("escape", cx))
        .unwrap();
    for _ in 0..4 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(300));
        frame(cx, s.win);
    }
    let on = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.within("title-bar").find("git-branch").focused()
        })
        .unwrap();
    assert_eq!(on, Some(true), "focus is back on the name");
}
