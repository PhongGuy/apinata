//! The title bar's controls give way in a fixed order as the window narrows, and never
//! overlap or leave the window: branch label, then the collection name down to its floor, then
//! (below the cramped width) the global environment select, then the project one. The app menu
//! and the manage-environments button never give way.

use crate::git_support;
use crate::support;

use apinata_app::app::AppState;
use apinata_app::style::{ENV_CONTROL_MIN, SPACE_1, TITLE_NAME_FLOOR, at_zoom};
use git_support::{Sandbox, write};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext, px, size};
use support::*;

const NAME: &str = "A collection with a rather long descriptive name";
const ORDER: [&str; 6] = [
    "collection-switcher",
    "git-label",
    "env-control",
    "global-env-selector",
    "environments",
    "app-menu",
];

/// Left and right edge of each ORDER control at a window `w` wide and zoom `zoom`.
fn edges(cx: &mut TestAppContext, s: &Started, w: f32, zoom: f32) -> Vec<(f32, f32)> {
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(w), px(600.)));
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| p.update(cx, |st| st.zoom = zoom));
    });
    // The branch label reports its room a frame late, so let layout settle.
    for _ in 0..4 {
        frame(cx, s.win);
    }
    cx.update_window(s.win, |_, win, cx| {
        win.render_frame(cx);
        let bar = win.within("title-bar");
        // A hidden label has left its id; it takes no edge of its own.
        let mut out: Vec<(f32, f32)> = Vec::new();
        for id in ORDER {
            let at = out.last().map_or(0., |e| e.1);
            out.push(match bar.try_find(id) {
                Some(e) => (e.bounds().left().as_f32(), e.bounds().right().as_f32()),
                None => (at, at),
            });
        }
        out
    })
    .unwrap()
}

fn long_named_changed(cx: &mut TestAppContext, sb: &Sandbox) -> (tempfile::TempDir, Started) {
    let root = format!("opencollection: 1.0.0\ninfo:\n  name: {NAME}\n");
    let col = write_tree(&[
        ("opencollection.yml", &root),
        ("a.yml", &request("a", "http://x/a")),
    ]);
    sb.git(col.path(), &["init", "-q", "-b", "main"]);
    sb.commit_all(col.path(), "init");
    write(col.path(), "a.yml", &request("a", "http://x/changed"));
    let s = started(cx, &col);
    let options = sb.options();
    s.ws.update(cx, |w, cx| w.set_git(Some(options), cx));
    frame(cx, s.win);
    (col, s)
}

// Sweeps the window width from 1500 px down to `lowest` at `zoom`, checking every stage.
// `reached` says which stages the sweep must hit: the selects give way only in a narrow bar.
fn sweep(cx: &mut TestAppContext, zoom: f32, lowest: u32, reached: [bool; 3]) {
    let sb = Sandbox::new();
    let (_col, s) = long_named_changed(cx, &sb);
    let wide = edges(cx, &s, 1600., zoom);
    let width = |e: &[(f32, f32)], ix: usize| e[ix].1 - e[ix].0;
    let natural_name = width(&wide, 0);
    let min = at_zoom(ENV_CONTROL_MIN, zoom).as_f32();
    let floor = at_zoom(TITLE_NAME_FLOOR, zoom).as_f32() + 2. * at_zoom(SPACE_1, zoom).as_f32();
    let sliver = 6.;
    let mut seen = [false; 3];
    for w in (lowest..=1500).rev().step_by(20) {
        let w = w as f32;
        let e = edges(cx, &s, w, zoom);
        let at = |ix: usize| width(&e, ix);
        let (name, branch, env, global) = (at(0), at(1), at(2), at(3));
        // Nothing leaves the window, and the menu keeps room for its ring.
        for (id, (l, r)) in ORDER.iter().zip(&e) {
            assert!(
                *l >= 0. && *r <= w,
                "{w}: {id} [{l}, {r}] leaves the window in {e:?}"
            );
        }
        assert!(
            e[5].1 <= w - at_zoom(SPACE_1, zoom).as_f32(),
            "{w}: no ring room at the menu"
        );
        // Nothing overlaps its neighbour (a collapsed control takes no room to overlap).
        let shown: Vec<_> = e.iter().filter(|(l, r)| r - l >= 1.).collect();
        for pair in shown.windows(2) {
            assert!(
                pair[0].1 <= pair[1].0 + 0.5,
                "{w}: {pair:?} overlap in {e:?}"
            );
        }
        // The two buttons never give way.
        assert_eq!(at(4), width(&wide, 4), "{w}: manage-environments shrank");
        assert_eq!(at(5), width(&wide, 5), "{w}: the app menu shrank");
        // The order: branch, name to its floor, global select, project select.
        if branch > sliver {
            assert!(
                name >= natural_name - 1.,
                "{w}: name cut before the branch: {e:?}"
            );
        }
        if global < min - 1. {
            assert!(
                branch <= sliver,
                "{w}: selects shrank before the branch: {e:?}"
            );
            assert!(
                name <= floor + 1.,
                "{w}: selects shrank before the name: {e:?}"
            );
            seen[0] = true;
        }
        if branch > sliver {
            seen[1] = true;
        }
        if env < min - 1. {
            assert!(global <= sliver, "{w}: project select before global: {e:?}");
            seen[2] = true;
        }
    }
    assert_eq!(seen, reached, "the sweep reached the wrong stages");
}

#[gpui_kit::test]
fn the_title_bar_gives_way_in_order_without_overlap_at_200_percent(cx: &mut TestAppContext) {
    sweep(cx, 2.0, 240, [true; 3]);
}

#[gpui_kit::test]
fn the_title_bar_gives_way_in_order_without_overlap_at_100_percent(cx: &mut TestAppContext) {
    sweep(cx, 1.0, 320, [false, true, false]);
}

fn unchanged_long_named(cx: &mut TestAppContext, sb: &Sandbox) -> (tempfile::TempDir, Started) {
    let root = format!("opencollection: 1.0.0\ninfo:\n  name: {NAME}\n");
    let col = write_tree(&[
        ("opencollection.yml", &root),
        ("a.yml", &request("a", "http://x/a")),
    ]);
    sb.git(col.path(), &["init", "-q", "-b", "main"]);
    sb.commit_all(col.path(), "init");
    let s = started(cx, &col);
    let options = sb.options();
    s.ws.update(cx, |w, cx| w.set_git(Some(options), cx));
    frame(cx, s.win);
    (col, s)
}

// Narrower than icon, gap and an ellipsis with a letter, the label shows nothing legible.
const LEGIBLE_LABEL: f32 = 40.;

/// The label's width, or `None` while it is hidden or collapsed; the chevron must always stay.
fn label_width(cx: &mut TestAppContext, s: &Started) -> Option<f32> {
    cx.update_window(s.win, |_, win, _| {
        let bar = win.within("title-bar");
        let chevron = bar.find("git-branch-chevron");
        assert_eq!(chevron.label(), Some("Switch branch"));
        let width = bar
            .try_find("git-label")
            .map(|e| e.bounds().size.width.as_f32());
        width.filter(|w| *w >= 1.)
    })
    .unwrap()
}

// A label squeezed to a sliver shows a stray glyph, so it is either legible or hidden.
fn label_is_legible_or_hidden(cx: &mut TestAppContext, changed: bool) {
    let sb = Sandbox::new();
    let (_col, s) = if changed {
        long_named_changed(cx, &sb)
    } else {
        unchanged_long_named(cx, &sb)
    };
    let mut hidden_once = false;
    for w in (380..=1000).step_by(6) {
        edges(cx, &s, w as f32, 1.0);
        match label_width(cx, &s) {
            Some(width) => assert!(
                width >= LEGIBLE_LABEL,
                "{w}: the label is a {width} px sliver"
            ),
            None => hidden_once = true,
        }
    }
    assert!(hidden_once, "the sweep never narrowed the label away");
    // Widening the window brings the label back.
    edges(cx, &s, 1600., 1.0);
    assert!(label_width(cx, &s).is_some_and(|w| w >= LEGIBLE_LABEL));
}

#[gpui_kit::test]
fn a_changed_label_is_legible_or_hidden_never_a_sliver(cx: &mut TestAppContext) {
    label_is_legible_or_hidden(cx, true);
}

#[gpui_kit::test]
fn a_clean_label_is_legible_or_hidden_never_a_sliver(cx: &mut TestAppContext) {
    label_is_legible_or_hidden(cx, false);
}

// The change count truncates before the branch name and hides alone rather than shrink to a
// sliver, and it comes back when the bar widens.
#[gpui_kit::test]
fn the_change_count_is_legible_or_hidden_never_a_sliver(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_col, s) = long_named_changed(cx, &sb);
    let count_width = |cx: &mut TestAppContext| {
        cx.update_window(s.win, |_, win, _| {
            let bar = win.within("title-bar");
            bar.try_find("git-changes")
                .map(|e| e.bounds().size.width.as_f32())
                .filter(|w| *w >= 1.)
        })
        .unwrap()
    };
    let mut count_gone_name_shown = false;
    for w in (380..=1000).step_by(6) {
        edges(cx, &s, w as f32, 1.0);
        let count = count_width(cx);
        if let Some(width) = count {
            assert!(
                width >= LEGIBLE_LABEL,
                "{w}: the count is a {width} px sliver"
            );
        }
        if count.is_none() && label_width(cx, &s).is_some() {
            count_gone_name_shown = true;
        }
    }
    assert!(
        count_gone_name_shown,
        "the count never gave way before the branch name"
    );
    edges(cx, &s, 1600., 1.0);
    assert!(count_width(cx).is_some_and(|w| w >= LEGIBLE_LABEL));
}

/// The count button's width, or `None` while it is hidden.
fn count_width(cx: &mut TestAppContext, s: &Started) -> Option<f32> {
    cx.update_window(s.win, |_, win, _| {
        win.within("title-bar")
            .try_find("git-changes")
            .map(|e| e.bounds().size.width.as_f32())
            .filter(|w| *w >= 1.)
    })
    .unwrap()
}

// The count is whole or hidden at every width: an ellipsis in "1 chang..." reads as noise.
// Finds where the count gives way, then steps 1 px through 40 px either side, narrowing and
// widening, and draws extra frames at each width so a flicker between frames shows.
#[gpui_kit::test]
fn the_change_count_shows_whole_or_hides_never_cut(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_col, s) = long_named_changed(cx, &sb);
    edges(cx, &s, 1600., 1.0);
    let natural = count_width(cx, &s).expect("a wide bar shows the count");
    // The state at width `w` after layout settles, checked steady over six more frames.
    let settled = |cx: &mut TestAppContext, w: u32| {
        edges(cx, &s, w as f32, 1.0);
        let first = count_width(cx, &s);
        for _ in 0..6 {
            frame(cx, s.win);
            let now = count_width(cx, &s);
            assert_eq!(
                now.is_some(),
                first.is_some(),
                "{w}: the count toggles between frames ({first:?} then {now:?})"
            );
        }
        if let Some(width) = first {
            assert!(
                width >= natural - 0.5,
                "{w}: the count is cut to {width} of its {natural} px"
            );
        }
        first.is_some()
    };
    let hide = (380..=1100)
        .rev()
        .step_by(6)
        .find(|w| !settled(cx, *w))
        .expect("the count hides in a narrow bar");
    let band = (hide - 40)..=(hide + 40);
    for w in band.clone().rev() {
        settled(cx, w);
    }
    let mut back = None;
    for w in band {
        if settled(cx, w) {
            back.get_or_insert(w);
        } else {
            assert!(back.is_none(), "{w}: the count hid again after coming back");
        }
    }
    assert!(
        back.is_some(),
        "the count never came back within 40 px of {hide}"
    );
}
