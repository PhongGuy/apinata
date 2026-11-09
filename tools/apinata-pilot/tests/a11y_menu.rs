//! A sidebar row's context menu opens while accessibility is active, in a debug build.
//!
//! The MCP pilot turns accessibility on only around its own polls, so a right-click there never
//! draws the menu's first frame with it on. This test keeps it on throughout, as a screen reader
//! would, so a menu that reports focus twice in one frame panics the debug build here.

use std::{rc::Rc, sync::mpsc, time::Duration};

use apinata_pilot::{Apinata, open_app};
use gpui_kit::base::test_support::snapshots;
use gpui_kit::{
    AppContext as _, Application, AsyncApp, Bounds, ElementId, MouseButton, Pixels, px, size,
};
use gpui_pilot_host::{PilotApp as _, PilotPlatform, PilotWindow};

const ROW: &str = "0:health.yml";

fn collection() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let files = [
        (
            "opencollection.yml",
            "opencollection: 1.0.0\ninfo:\n  name: Fixture\n",
        ),
        (
            "health.yml",
            "info:\n  name: Health\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://127.0.0.1:9/health\"\n",
        ),
    ];
    for (rel, text) in files {
        std::fs::write(dir.path().join(rel), text).unwrap();
    }
    dir
}

/// The bounds of every drawn element whose id is `id`.
fn found(window: &PilotWindow, cx: &mut AsyncApp, id: &str) -> Vec<Bounds<Pixels>> {
    let wanted = ElementId::Name(id.to_owned().into());
    cx.update_window(window.handle(), |_, window, _| {
        snapshots(window)
            .into_iter()
            .filter(|s| s.path().last() == Some(&wanted))
            .map(|s| s.bounds())
            .collect()
    })
    .unwrap()
}

/// Settles, then waits up to 5 s for an element `id` to be drawn.
async fn wait_for(
    platform: &PilotPlatform,
    window: &PilotWindow,
    cx: &mut AsyncApp,
    id: &str,
) -> Bounds<Pixels> {
    for _ in 0..50 {
        gpui_pilot_host::settle(platform, window, cx, Duration::from_millis(100)).await;
        if let [bounds] = found(window, cx, id)[..] {
            return bounds;
        }
    }
    panic!("{id} was never drawn");
}

async fn right_click_row(platform: Rc<PilotPlatform>, mut cx: AsyncApp) -> (bool, bool) {
    let window = platform.main_window().expect("the app opened a window");
    let row = wait_for(&platform, &window, &mut cx, ROW).await;
    window.set_a11y_active(true);
    let events =
        gpui_pilot_host::click_events(row.center(), MouseButton::Right, 1, Default::default());
    cx.update_window(window.handle(), |_, w, cx| {
        gpui_pilot_host::dispatch_events(w, cx, events)
    })
    .unwrap();
    wait_for(&platform, &window, &mut cx, "popup-menu").await;
    let active = cx
        .update_window(window.handle(), |_, w, _| w.is_a11y_active())
        .unwrap();
    let menu = window.a11y_tree().is_some_and(|tree| {
        (tree.nodes.iter()).any(|(_, node)| node.role() == gpui_kit::accesskit::Role::Menu)
    });
    (active, menu)
}

#[test]
fn a_rows_context_menu_opens_with_accessibility_active() {
    let (data, roots) = (tempfile::tempdir().unwrap(), collection());
    let root = roots.path().to_str().unwrap().to_owned();
    let platform = PilotPlatform::new(size(px(1280.), px(800.)), 1.0, 60);
    let (done, result) = mpsc::channel();
    let data_dir = data.path().to_owned();
    let app =
        Application::with_platform(platform.clone()).with_assets(apinata_app::icons::AppAssets);
    app.run(move |cx| {
        Apinata.init(cx);
        open_app(&data_dir, &[root], false, cx).expect("the app opens");
        let body_cx = cx.to_async();
        cx.spawn(async move |cx| {
            done.send(right_click_row(platform, body_cx).await).ok();
            cx.update(|cx| cx.quit());
        })
        .detach();
    });
    let (active, menu) = result.recv().expect("the app ran the test to its end");
    assert!(active, "accessibility was off when the menu was drawn");
    assert!(menu, "the accessibility tree sent no menu");
}
