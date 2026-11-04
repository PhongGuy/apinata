//! The Inherited headers group under a request's headers: read-only rows for what the
//! collection and folders add, struck through when an inner level or the request sets the name.

use crate::support;

use apinata_app::inherited_headers_section::{InheritedHeadersSection, inherited_rows};
use apinata_core::{Collection, ItemPath, KeyValue};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use support::*;

const REQ: &str = "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: https://x.test/\n";

fn collection(root_rows: &str, folder_rows: &str) -> (tempfile::TempDir, Collection) {
    let dir = tempfile::TempDir::new().unwrap();
    let p = dir.path();
    std::fs::create_dir_all(p.join("f")).unwrap();
    let root =
        format!("opencollection: \"1.0.0\"\ninfo:\n  name: t\nrequest:\n  headers:\n{root_rows}");
    let folder =
        format!("info:\n  name: Folder F\n  type: folder\nrequest:\n  headers:\n{folder_rows}");
    std::fs::write(p.join("opencollection.yml"), root).unwrap();
    std::fs::write(p.join("f/folder.yml"), folder).unwrap();
    std::fs::write(p.join("f/r.yml"), REQ).unwrap();
    let c = Collection::open(p).unwrap();
    (dir, c)
}

fn section(
    cx: &mut TestAppContext,
    c: &Collection,
    own: &[&str],
) -> (
    AnyWindowHandle,
    Entity<SectionHost<InheritedHeadersSection>>,
) {
    let (win, host) = host_section(cx, InheritedHeadersSection::new);
    let s = host.read_with(cx, |h, _| h.view.clone());
    let rows = inherited_rows(c, &ItemPath::new("f/r.yml").unwrap());
    let own: Vec<KeyValue> = own.iter().map(|n| KeyValue::new(*n, "x")).collect();
    cx.update_window(win, |_, _, cx| {
        s.update(cx, |s, cx| {
            s.load(&rows, cx);
            s.set_own(&own, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    (win, host)
}

fn found(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> bool {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id).is_some()
    })
    .unwrap()
}

fn label(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> Option<String> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

#[gpui_kit::test]
fn rows_show_name_raw_value_and_source(cx: &mut TestAppContext) {
    let (_d, c) = collection(
        "    - name: X-A\n      value: a\n",
        "    - name: X-B\n      value: \"{{token}}\"\n",
    );
    let (win, _host) = section(cx, &c, &[]);
    assert!(found(cx, win, "inherited-headers"));
    assert!(found(cx, win, "inherited-headers-heading"));
    assert_eq!(
        label(cx, win, "inherited-header-0-name").as_deref(),
        Some("X-A")
    );
    assert_eq!(
        label(cx, win, "inherited-header-1-value").as_deref(),
        Some("{{token}}")
    );
    assert_eq!(
        label(cx, win, "inherited-header-0-source").as_deref(),
        Some("collection")
    );
    assert_eq!(
        label(cx, win, "inherited-header-1-source").as_deref(),
        Some("Folder F")
    );
    assert!(!found(cx, win, "inherited-header-0-overridden"));
}

#[gpui_kit::test]
fn a_header_set_by_the_request_reads_as_overridden_live(cx: &mut TestAppContext) {
    let (_d, c) = collection("    - name: X-A\n      value: a\n", "");
    let (win, host) = section(cx, &c, &["X-A"]);
    assert!(found(cx, win, "inherited-header-0-overridden"));
    // Dropping the request's own row restores the inherited one.
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, _, cx| s.update(cx, |s, cx| s.set_own(&[], cx)))
        .unwrap();
    cx.run_until_parked();
    assert!(!found(cx, win, "inherited-header-0-overridden"));
}

#[gpui_kit::test]
fn an_inner_folder_row_marks_the_outer_one_overridden(cx: &mut TestAppContext) {
    let (_d, c) = collection(
        "    - name: X-A\n      value: outer\n",
        "    - name: X-A\n      value: inner\n",
    );
    let (win, _host) = section(cx, &c, &[]);
    assert!(found(cx, win, "inherited-header-0-overridden"));
    assert!(!found(cx, win, "inherited-header-1-overridden"));
}

#[gpui_kit::test]
fn nothing_inherited_leaves_no_group(cx: &mut TestAppContext) {
    let (_d, c) = collection("", "");
    let (win, _host) = section(cx, &c, &[]);
    assert!(!found(cx, win, "inherited-headers"));
    assert!(!found(cx, win, "inherited-headers-heading"));
}
