//! Tab into a control below the fold of the request section body or the environment editor
//! and the area scrolls just far enough to show it.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::request_editor::Section;
use apinata_app::scroll_follow::{FocusFollow, Followed};
use apinata_app::settings::Settings;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Bounds, ElementId, Pixels, TestAppContext, px, size};
use support::*;

// Resizes the window and sets zoom, then draws so scroll handles report the new extent.
fn room(cx: &mut TestAppContext, s: &Started, w: f32, h: f32, zoom: f32) {
    gpui_kit::VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(w), px(h)));
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| p.update(cx, |s: &mut Settings| s.zoom = zoom));
    });
    for _ in 0..3 {
        frame(cx, s.win);
    }
}

// Anything that names the window under test: the started app or a bare window.
trait Win {
    fn win(&self) -> gpui_kit::AnyWindowHandle;
}
impl Win for Started {
    fn win(&self) -> gpui_kit::AnyWindowHandle {
        self.win
    }
}
impl Win for gpui_kit::AnyWindowHandle {
    fn win(&self) -> gpui_kit::AnyWindowHandle {
        *self
    }
}

fn find(cx: &mut TestAppContext, s: &impl Win, id: impl Into<ElementId>) -> Bounds<Pixels> {
    let id = id.into();
    cx.update_window(s.win(), |_, w, cx| {
        w.render_frame(cx);
        w.find(id).bounds()
    })
    .unwrap()
}

fn focused(cx: &mut TestAppContext, s: &impl Win, id: impl Into<ElementId>) -> bool {
    let id = id.into();
    cx.update_window(s.win(), |_, w, _| w.find(id).focused())
        .unwrap()
        == Some(true)
}

// Presses Tab until `id` has focus, drawing a few frames after each press. An assertion row
// has five stops, so eight rows take 40 presses.
fn tab_to(cx: &mut TestAppContext, s: &impl Win, id: impl Into<ElementId> + Clone) {
    for _ in 0..80 {
        if focused(cx, s, id.clone()) {
            return;
        }
        cx.update_window(s.win(), |_, w, cx| w.press("tab", cx))
            .unwrap();
        for _ in 0..4 {
            frame(cx, s.win());
        }
    }
    panic!("Tab never reached the control");
}

fn assert_in_view(inner: Bounds<Pixels>, area: Bounds<Pixels>, what: &str) {
    assert!(
        inner.top() >= area.top() - px(1.) && inner.bottom() <= area.bottom() + px(1.),
        "{what} {inner:?} is outside {area:?}"
    );
}

#[gpui_kit::test]
fn tabbing_to_a_settings_control_below_the_fold_scrolls_it_into_view(cx: &mut TestAppContext) {
    let a =
        "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://x/0\"\n";
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", a)]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    cx.update_window(s.win, |_, w, cx| {
        w.click(("section", Section::Settings as usize), cx);
        w.render_frame(cx)
    })
    .unwrap();
    room(cx, &s, 1280., 360., 2.0);
    let body = find(cx, &s, "section-body");
    let row = find(cx, &s, "set-row-timeout");
    assert!(row.bottom() > body.bottom(), "{row:?} fits {body:?}");
    tab_to(cx, &s, "set-timeout-mode");
    let body = find(cx, &s, "section-body");
    assert_in_view(find(cx, &s, "set-timeout-mode"), body, "the timeout select");
}

#[gpui_kit::test]
fn tabbing_through_the_variables_keeps_each_in_view(cx: &mut TestAppContext) {
    let mut vars = "name: dev\nvariables:\n".to_string();
    for i in 0..8 {
        vars += &format!("  - name: v{i}\n    value: x\n");
    }
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", &vars),
    ]);
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("environments", cx)
    })
    .unwrap();
    frame(cx, s.win);
    room(cx, &s, 1280., 300., 1.0);
    let editor = find(cx, &s, "env-editor");
    let last = find(cx, &s, ("env-name", 7usize));
    assert!(last.bottom() > editor.bottom(), "{last:?} fits {editor:?}");
    cx.update_window(s.win, |_, w, cx| w.click(("env-name", 0usize), cx))
        .unwrap();
    frame(cx, s.win);
    for i in 1..8usize {
        tab_to(cx, &s, ("env-name", i));
        let editor = find(cx, &s, "env-editor");
        assert_in_view(find(cx, &s, ("env-name", i)), editor, "the variable name");
    }
}

// A request with `n` rows of the given YAML list under `http:` or `runtime:`.
fn long_request(list: &str, n: usize, row: impl Fn(usize) -> String) -> String {
    let head =
        "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://x/0\"\n";
    let mut text = head.to_string();
    text += list;
    for i in 0..n {
        text += &row(i);
    }
    text
}

// Opens `a.yml`, shows `section`, and shrinks the window so the last of `n` rows is below the
// fold of `section-body`.
fn open_long_section(cx: &mut TestAppContext, yaml: &str, section: Section) -> Started {
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", yaml)]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    cx.update_window(s.win, |_, w, cx| {
        w.click(("section", section as usize), cx);
        w.render_frame(cx)
    })
    .unwrap();
    room(cx, &s, 1280., 420., 1.0);
    s
}

#[gpui_kit::test]
fn tabbing_through_a_long_headers_table_keeps_each_row_in_view(cx: &mut TestAppContext) {
    let yaml = long_request("  headers:\n", 14, |i| {
        format!("    - name: h{i}\n      value: x\n")
    });
    let s = open_long_section(cx, &yaml, Section::Headers);
    let body = find(cx, &s, "section-body");
    let last = find(cx, &s, ("kv-name", 13usize));
    assert!(last.bottom() > body.bottom(), "{last:?} fits {body:?}");
    cx.update_window(s.win, |_, w, cx| w.click(("kv-name", 0usize), cx))
        .unwrap();
    frame(cx, s.win);
    for i in [4usize, 9] {
        tab_to(cx, &s, ("kv-name", i));
        let body = find(cx, &s, "section-body");
        assert_in_view(find(cx, &s, ("kv-name", i)), body, "the header name");
    }
}

#[gpui_kit::test]
fn tabbing_through_a_long_assertions_table_keeps_each_row_in_view(cx: &mut TestAppContext) {
    let yaml = long_request("runtime:\n  assertions:\n", 14, |i| {
        format!("    - expression: res.body.f{i}\n      operator: eq\n      value: \"1\"\n")
    });
    let s = open_long_section(cx, &yaml, Section::Assertions);
    let body = find(cx, &s, "section-body");
    let last = find(cx, &s, ("as-expr", 13usize));
    assert!(last.bottom() > body.bottom(), "{last:?} fits {body:?}");
    cx.update_window(s.win, |_, w, cx| w.click(("as-expr", 0usize), cx))
        .unwrap();
    frame(cx, s.win);
    for i in [5usize, 13] {
        tab_to(cx, &s, ("as-expr", i));
        let body = find(cx, &s, "section-body");
        assert_in_view(
            find(cx, &s, ("as-expr", i)),
            body,
            "the assertion expression",
        );
    }
}

struct Blank;
impl gpui_kit::Render for Blank {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        gpui_kit::div()
    }
}

#[gpui_kit::test]
fn tabbing_through_the_global_environment_editor_keeps_each_variable_in_view(
    cx: &mut TestAppContext,
) {
    use apinata_app::global_env_editor::open_global_env_editor;
    let mut vars = "name: g\nvariables:\n".to_string();
    for i in 0..14 {
        vars += &format!("  - name: v{i}\n    value: x\n");
    }
    support::init(cx);
    let col = collection();
    let globals = write_tree(&[("g.yml", &vars)]);
    let (_state, ws, _root) = open_dir(cx, &col);
    let dir = globals.path().to_path_buf();
    ws.update(cx, |w, cx| w.set_global_dir(dir, cx));
    cx.run_until_parked();
    let (win, _) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |_, cx| {
                cx.new(|_| Blank)
            })
        })
        .unwrap();
    gpui_kit::VisualTestContext::from_window(win, cx).simulate_resize(size(px(1280.), px(420.)));
    let editor = cx
        .update_window(win, |_, w, cx| open_global_env_editor(ws.clone(), w, cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        editor.update(cx, |e, cx| e.select("g", w, cx))
    })
    .unwrap();
    for _ in 0..3 {
        frame(cx, win);
    }
    let editor_box = find(cx, &win, "global-env-scroll");
    let last = find(cx, &win, ("env-name", 13usize));
    assert!(
        last.bottom() > editor_box.bottom(),
        "{last:?} fits {editor_box:?}"
    );
    cx.update_window(win, |_, w, cx| w.click(("env-name", 0usize), cx))
        .unwrap();
    frame(cx, win);
    for i in [5usize, 13] {
        tab_to(cx, &win, ("env-name", i));
        let scroll = find(cx, &win, "global-env-scroll");
        assert_in_view(find(cx, &win, ("env-name", i)), scroll, "the variable name");
    }
}

/// Two wrappers far apart on one line of a 200 px scroll area, the second wider than the area.
struct Sideways {
    scroll: gpui_kit::ScrollHandle,
    follow: FocusFollow,
    near: gpui_kit::FocusHandle,
    far: gpui_kit::FocusHandle,
    wide: gpui_kit::FocusHandle,
    inside: gpui_kit::FocusHandle,
}

impl gpui_kit::Render for Sideways {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        use gpui_kit::{InteractiveElement as _, ParentElement as _, Styled as _, div};
        use gpui_kit::{StatefulInteractiveElement as _, TestSupportExt as _};
        let cell = |id: &'static str, handle: &gpui_kit::FocusHandle, width: f32| {
            div()
                .id(id)
                .test_support()
                .track_focus(handle)
                .w(px(width))
                .h(px(20.))
        };
        let followed = |id: &'static str, inner| Followed::new((id, 0usize), &self.follow, inner);
        div()
            .id("sideways")
            .test_support()
            .w(px(200.))
            .h(px(100.))
            .overflow_scroll()
            .track_scroll(&self.scroll)
            .child(
                div()
                    .flex()
                    .w(px(2000.))
                    .gap(px(300.))
                    .child(followed("near", cell("near", &self.near, 50.)))
                    .child(followed("far", cell("far", &self.far, 50.)))
                    .child(followed(
                        "wide",
                        cell("wide", &self.wide, 500.).child(cell("inside", &self.inside, 10.)),
                    )),
            )
            .child(self.follow.watch(&self.scroll))
    }
}

#[gpui_kit::test]
fn focus_moves_the_area_sideways_just_enough(cx: &mut TestAppContext) {
    support::init(cx);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |_, cx| {
                cx.new(|cx| Sideways {
                    scroll: gpui_kit::ScrollHandle::new(),
                    follow: FocusFollow::default(),
                    near: cx.focus_handle(),
                    far: cx.focus_handle(),
                    wide: cx.focus_handle(),
                    inside: cx.focus_handle(),
                })
            })
        })
        .unwrap();
    let focus = |cx: &mut TestAppContext, pick: fn(&Sideways) -> &gpui_kit::FocusHandle| {
        let handle = view.read_with(cx, |v, _| pick(v).clone());
        cx.update_window(win, |_, w, cx| handle.focus(w, cx))
            .unwrap();
        for _ in 0..4 {
            frame(cx, win);
        }
    };
    frame(cx, win);
    let area = find(cx, &win, "sideways");

    focus(cx, |v| &v.far);
    let far = find(cx, &win, "far");
    assert_eq!(far.right(), area.right(), "no further than it takes");

    focus(cx, |v| &v.near);
    let near = find(cx, &win, "near");
    assert_eq!(near.left(), area.left(), "back the other way");

    focus(cx, |v| &v.wide);
    let wide = find(cx, &win, "wide");
    assert_eq!(
        wide.left(),
        area.left(),
        "a wide wrapper keeps its left edge"
    );

    // Scrolled into the middle of the wrapper, it spans the area and stays put.
    let delta = gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(-100.), px(0.)));
    cx.update_window(win, |_, w, cx| w.scroll("sideways", delta, cx))
        .unwrap();
    frame(cx, win);
    let spanning = find(cx, &win, "wide");
    assert!(spanning.left() < area.left() && spanning.right() > area.right());
    focus(cx, |v| &v.inside);
    assert_eq!(find(cx, &win, "wide").left(), spanning.left(), "no jump");
}

#[gpui_kit::test]
fn tabbing_through_a_narrow_environment_editor_moves_sideways(cx: &mut TestAppContext) {
    let vars = "name: dev\nvariables:\n  - name: host\n    value: x\n";
    let col = write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", vars)]);
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("environments", cx)
    })
    .unwrap();
    frame(cx, s.win);
    room(cx, &s, 320., 700., 1.0);
    let editor = find(cx, &s, "env-editor");
    let row = find(cx, &s, ("env-row", 0usize));
    assert!(row.right() > editor.right(), "{row:?} fits {editor:?}");
    tab_to(cx, &s, ("env-name", 0usize));
    let row = find(cx, &s, ("env-row", 0usize));
    assert!(row.right() <= editor.right() + px(1.), "{row:?} {editor:?}");
}
