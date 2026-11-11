//! The title bar's change count comes back whenever it fits: after a narrow bar hid it, a
//! wide bar shows it again on the next frame even when its text changed meanwhile, a shown
//! count never blinks out when its text changes, and a hidden count leaves no gap.

use crate::git_support;
use crate::support;

use apinata_app::app::AppState;
use apinata_app::style::{SPACE_1, at_zoom};
use git_support::{Sandbox, write};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext, px, size};
use support::*;

const FILES: usize = 10;

fn file(ix: usize) -> String {
    format!("r{ix}.yml")
}

/// The real app over a repository of `FILES` requests, the first `changed` of them edited.
fn with_changed(
    cx: &mut TestAppContext,
    sb: &Sandbox,
    changed: usize,
) -> (tempfile::TempDir, Started) {
    repo_with_changed(cx, sb, changed, &[])
}

/// `with_changed` over a repository made with extra `git init` flags.
fn repo_with_changed(
    cx: &mut TestAppContext,
    sb: &Sandbox,
    changed: usize,
    init: &[&str],
) -> (tempfile::TempDir, Started) {
    let mut files = vec![("opencollection.yml".to_owned(), ROOT.to_owned())];
    files.extend((0..FILES).map(|ix| (file(ix), request(&format!("r{ix}"), "http://x/a"))));
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(p, t)| (p.as_str(), t.as_str()))
        .collect();
    let col = write_tree(&refs);
    let mut args = vec!["init", "-q", "-b", "main"];
    args.extend_from_slice(init);
    sb.git(col.path(), &args);
    sb.commit_all(col.path(), "init");
    for ix in 0..changed {
        write(
            col.path(),
            &file(ix),
            &request(&format!("r{ix}"), "http://x/changed"),
        );
    }
    let s = started(cx, &col);
    let options = sb.options();
    s.ws.update(cx, |w, cx| w.set_git(Some(options), cx));
    frame(cx, s.win);
    (col, s)
}

/// Edits or restores the last request so the count reads `changed`, and lets git report it.
fn set_changed(cx: &mut TestAppContext, col: &tempfile::TempDir, s: &Started, changed: usize) {
    let url = if changed == FILES {
        "http://x/changed"
    } else {
        "http://x/a"
    };
    let ix = FILES - 1;
    write(col.path(), &file(ix), &request(&format!("r{ix}"), url));
    let root = s.root.clone();
    s.ws.update(cx, |w, cx| w.refresh_git(&root, cx));
    cx.run_until_parked();
}

fn zoom_to(cx: &mut TestAppContext, zoom: f32) {
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| p.update(cx, |st| st.zoom = zoom));
    });
}

fn resize(cx: &mut TestAppContext, s: &Started, w: f32) {
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(w), px(700.)));
}

/// Left and right edge of each id in the last drawn frame, `None` for one absent or of no width.
fn drawn<const N: usize>(
    cx: &mut TestAppContext,
    s: &Started,
    ids: [&'static str; N],
) -> [Option<(f32, f32)>; N] {
    cx.update_window(s.win, |_, w, _| {
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

/// Draws one frame and returns the count's span in it.
fn next_frame_count(cx: &mut TestAppContext, s: &Started) -> Option<(f32, f32)> {
    cx.update_window(s.win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    drawn(cx, s, ["git-changes"])[0]
}

/// Narrows the bar until the count alone has hidden behind a shown name; returns that width.
fn hide_count(cx: &mut TestAppContext, s: &Started) -> f32 {
    for w in (200..=900).rev().step_by(4) {
        resize(cx, s, w as f32);
        for _ in 0..3 {
            frame(cx, s.win);
        }
        let [name, count] = drawn(cx, s, ["git-branch", "git-changes"]);
        if name.is_some() && count.is_none() {
            return w as f32;
        }
    }
    panic!("the sweep never hid the count alone");
}

/// The count's whole width in a wide bar at `zoom`, with `changed` files counted.
fn whole_width(cx: &mut TestAppContext, zoom: f32, changed: usize) -> f32 {
    let sb = Sandbox::new();
    let (_col, s) = with_changed(cx, &sb, changed);
    zoom_to(cx, zoom);
    resize(cx, &s, 1600.);
    for _ in 0..4 {
        frame(cx, s.win);
    }
    let (l, r) = drawn(cx, &s, ["git-changes"])[0].expect("a wide bar shows the count");
    r - l
}

// Hidden in a narrow bar, then widened to 1280 px while another file changes: the count is
// back, whole, within the frames the widening and the git report draw, and on every frame after.
fn returns_after_widening_and_text_change(
    cx: &mut TestAppContext,
    zoom: f32,
    from: usize,
    to: usize,
) {
    let whole = whole_width(cx, zoom, to);
    let sb = Sandbox::new();
    let (col, s) = with_changed(cx, &sb, from);
    zoom_to(cx, zoom);
    frame(cx, s.win);
    hide_count(cx, &s);
    resize(cx, &s, 1280.);
    set_changed(cx, &col, &s, to);
    assert!(
        drawn(cx, &s, ["git-changes"])[0].is_some(),
        "zoom {zoom}, {from} -> {to}: the frame git reported in hides the count"
    );
    let first = next_frame_count(cx, &s);
    for n in 0..4 {
        let now = next_frame_count(cx, &s);
        assert!(
            now.is_some(),
            "zoom {zoom}, {from} -> {to}: frame {n} after the change hides the count"
        );
    }
    let (l, r) = first.unwrap_or_else(|| {
        panic!("zoom {zoom}, {from} -> {to}: the count is hidden a frame after the report")
    });
    assert!(
        r - l >= whole - 0.5,
        "zoom {zoom}, {from} -> {to}: the count came back cut to {} of {whole} px",
        r - l
    );
}

#[gpui_kit::test]
fn a_hidden_count_returns_at_full_width_when_it_grows(cx: &mut TestAppContext) {
    returns_after_widening_and_text_change(cx, 1.0, 9, 10);
}

#[gpui_kit::test]
fn a_hidden_count_returns_at_full_width_when_it_shrinks(cx: &mut TestAppContext) {
    returns_after_widening_and_text_change(cx, 1.0, 10, 9);
}

// A zoom whose rem makes the probe's room fractional; the layout snaps it to a whole pixel.
#[gpui_kit::test]
fn a_hidden_count_returns_at_full_width_at_a_fractional_zoom(cx: &mut TestAppContext) {
    for zoom in [1.1, 0.9, 1.25] {
        returns_after_widening_and_text_change(cx, zoom, 9, 10);
        returns_after_widening_and_text_change(cx, zoom, 10, 9);
    }
}

// A shown count whose text shortens in a wide bar never hides, not even for one frame.
#[gpui_kit::test]
fn a_shown_count_stays_when_its_text_shortens(cx: &mut TestAppContext) {
    for zoom in [1.0, 1.1] {
        let sb = Sandbox::new();
        let (col, s) = with_changed(cx, &sb, FILES);
        zoom_to(cx, zoom);
        resize(cx, &s, 1280.);
        for _ in 0..4 {
            frame(cx, s.win);
        }
        assert!(
            drawn(cx, &s, ["git-changes"])[0].is_some(),
            "zoom {zoom}: no count"
        );
        set_changed(cx, &col, &s, FILES - 1);
        assert!(
            drawn(cx, &s, ["git-changes"])[0].is_some(),
            "zoom {zoom}: the frame git reported in hides the count"
        );
        for n in 0..4 {
            assert!(
                next_frame_count(cx, &s).is_some(),
                "zoom {zoom}: frame {n} after the text shortened hides the count"
            );
        }
    }
}

// A hidden count leaves no blank room: the chevron follows the name directly.
#[gpui_kit::test]
fn a_hidden_count_leaves_no_gap_before_the_chevron(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_col, s) = with_changed(cx, &sb, 1);
    let w = hide_count(cx, &s);
    let [name, chevron, probe] = drawn(
        cx,
        &s,
        ["git-branch", "git-branch-chevron", "git-count-probe"],
    );
    let (name, chevron) = (name.unwrap(), chevron.expect("the chevron stays"));
    let space = at_zoom(SPACE_1, 1.0).as_f32();
    assert!(
        chevron.0 - name.1 <= space,
        "{w}: a {} px gap between the name {name:?} and the chevron {chevron:?}",
        chevron.0 - name.1
    );
    if let Some(probe) = probe {
        assert!(
            probe.0 >= chevron.1 - 0.5,
            "{w}: the probe {probe:?} sits before the chevron"
        );
    }
}

/// The narrowest width at which a count hidden in a narrow bar returns, widening 1 px at a time.
fn return_width(cx: &mut TestAppContext, changed: usize) -> f32 {
    let sb = Sandbox::new();
    let (_col, s) = with_changed(cx, &sb, changed);
    let from = hide_count(cx, &s) as u32;
    (from..from + 200)
        .map(|w| w as f32)
        .find(|w| {
            resize(cx, &s, *w);
            for _ in 0..3 {
                frame(cx, s.win);
            }
            drawn(cx, &s, ["git-changes"])[0].is_some()
        })
        .expect("the count returns as the bar widens")
}

// A count gone with its last change keeps its old width. When more files change at a width that
// fits the old text with its slack but not the new one, the count stays hidden on every frame.
#[gpui_kit::test]
fn a_grown_count_that_does_not_fit_never_shows(cx: &mut TestAppContext) {
    let (back_one, back_all) = (return_width(cx, 1), return_width(cx, FILES));
    assert!(
        back_all > back_one + 1.,
        "{back_one} and {back_all} leave no band"
    );
    let sb = Sandbox::new();
    let (col, s) = with_changed(cx, &sb, 1);
    hide_count(cx, &s);
    sb.commit_all(col.path(), "all");
    let root = s.root.clone();
    s.ws.update(cx, |w, cx| w.refresh_git(&root, cx));
    cx.run_until_parked();
    resize(cx, &s, back_one);
    for _ in 0..3 {
        frame(cx, s.win);
    }
    assert_eq!(
        drawn(cx, &s, ["git-changes"])[0],
        None,
        "a clean tree shows a count"
    );
    // Every request differs from the commit, so the count reads all of them.
    for ix in 0..FILES {
        write(
            col.path(),
            &file(ix),
            &request(&format!("r{ix}"), "http://x/again"),
        );
    }
    s.ws.update(cx, |w, cx| w.refresh_git(&root, cx));
    cx.run_until_parked();
    assert_eq!(
        drawn(cx, &s, ["git-changes"])[0],
        None,
        "{back_one}: the frame git reported in shows a count that does not fit"
    );
    for n in 0..4 {
        assert_eq!(
            next_frame_count(cx, &s),
            None,
            "{back_one}: frame {n} after the report shows a count that does not fit"
        );
    }
}

// A wholly hidden label leaves no blank room: the chevron starts the branch group, with or
// without a branch name (a reftable repository's HEAD names no branch).
fn hidden_label_leaves_no_gap(cx: &mut TestAppContext, init: &[&str]) {
    let sb = Sandbox::new();
    let (_col, s) = repo_with_changed(cx, &sb, 1, init);
    let named = drawn(cx, &s, ["git-branch"])[0].is_some();
    assert_eq!(
        named,
        init.is_empty(),
        "the label's name is not as the repository's"
    );
    let w = (200..=900)
        .rev()
        .step_by(4)
        .map(|w| w as f32)
        .find(|w| {
            resize(cx, &s, *w);
            for _ in 0..3 {
                frame(cx, s.win);
            }
            drawn(cx, &s, ["git-label"])[0].is_none()
        })
        .expect("the sweep never hid the label");
    let [group, chevron] = drawn(cx, &s, ["git-group", "git-branch-chevron"]);
    let (group, chevron) = (
        group.expect("the group stays"),
        chevron.expect("the chevron stays"),
    );
    assert!(
        chevron.0 <= group.0 + 0.5,
        "{w}: a {} px gap before the chevron {chevron:?} in {group:?}",
        chevron.0 - group.0
    );
}

#[gpui_kit::test]
fn a_hidden_named_label_leaves_no_gap_before_the_chevron(cx: &mut TestAppContext) {
    hidden_label_leaves_no_gap(cx, &[]);
}

#[gpui_kit::test]
fn a_hidden_nameless_label_leaves_no_gap_before_the_chevron(cx: &mut TestAppContext) {
    hidden_label_leaves_no_gap(cx, &["--ref-format=reftable"]);
}

#[gpui_kit::test]
fn the_changed_file_count_says_it_shows_changes(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_col, s) = with_changed(cx, &sb, 1);
    cx.run_until_parked();
    frame(cx, s.win);
    let tip = cx
        .update_window(s.win, |_, w, _| {
            w.within("title-bar")
                .find("git-changes")
                .description()
                .map(str::to_owned)
        })
        .unwrap();
    assert_eq!(tip.as_deref(), Some("Show changes"));
}
