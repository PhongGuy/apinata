//! Removing a client certificate row keeps focus in the Network tab, so the save key works.

use crate::support;

use apinata_app::config_editor::{ConfigEditor, ConfigSection, ConfigTarget};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use support::*;

fn cert(domain: &str) -> String {
    format!(
        "    - domain: {domain}\n      type: pem\n      certificateFilePath: c.pem\n      privateKeyFilePath: k.pem\n"
    )
}

// Opens the Network tab of a collection holding one certificate per domain.
fn open_network(
    cx: &mut TestAppContext,
    domains: &[&str],
) -> (tempfile::TempDir, AnyWindowHandle, tempfile::TempDir) {
    let certs: String = domains.iter().map(|d| cert(d)).collect();
    let root = format!("{ROOT}config:\n  clientCertificates:\n{certs}");
    let col = write_tree(&[("opencollection.yml", &root)]);
    let (state, ws, path) = open_dir(cx, &col);
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let (win, ed): (_, Entity<ConfigEditor>) = {
        let ws = ws.clone();
        cx.update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ConfigEditor::new(ws, path, ConfigTarget::Collection, w, cx))
            })
        })
        .unwrap()
    };
    ed.update(cx, |e, cx| e.set_section(ConfigSection::Network, cx));
    frame(cx, win);
    (col, win, state)
}

// Clicks the real remove button of row `ix`, then checks which row's domain field has focus
// (`None`: no row is left, so any focus inside the window will do) and that the save key
// writes the remaining certificates.
fn remove_then_save(
    cx: &mut TestAppContext,
    domains: &[&str],
    ix: usize,
    target: Option<usize>,
    kept: &[&str],
) {
    let (col, win, _state) = open_network(cx, domains);
    type_at_end(cx, win, None, ("cert-domain", ix), "");
    cx.update_window(win, |_, w, cx| w.click(("cert-remove", ix), cx))
        .unwrap();
    frame(cx, win);
    let focus = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            match target {
                Some(t) => w.find(("cert-domain", t)).focused() == Some(true),
                None => w.focused(cx).is_some(),
            }
        })
        .unwrap();
    assert!(focus, "removing row {ix} of {} moves focus", domains.len());
    cx.update_window(win, |_, w, cx| w.press(&key("s"), cx))
        .unwrap();
    frame(cx, win);
    let saved = std::fs::read_to_string(col.path().join("opencollection.yml")).unwrap();
    for d in domains {
        assert_eq!(saved.contains(d), kept.contains(d), "{d} in {saved}");
    }
}

#[gpui_kit::test]
fn removing_a_middle_certificate_focuses_the_next_row_and_saves(cx: &mut TestAppContext) {
    remove_then_save(
        cx,
        &["a.test", "b.test", "c.test"],
        1,
        Some(1),
        &["a.test", "c.test"],
    );
}

#[gpui_kit::test]
fn removing_the_last_certificate_focuses_the_previous_row_and_saves(cx: &mut TestAppContext) {
    remove_then_save(cx, &["a.test", "b.test"], 1, Some(0), &["a.test"]);
}

// With no row left, focus falls back to the section itself, which still routes the save key.
#[gpui_kit::test]
fn removing_the_only_certificate_keeps_focus_and_saves(cx: &mut TestAppContext) {
    remove_then_save(cx, &["a.test"], 0, None, &[]);
}
