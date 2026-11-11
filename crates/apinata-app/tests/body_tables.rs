use crate::support;

use apinata_app::body_section::{BodyMode, BodySection, collection_relative};
use apinata_app::command::Edit;
use apinata_app::kv_table::KvColumns;
use apinata_core::{BodySpec, FileEntry, KeyValue, MultipartField};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use std::path::{Path, PathBuf};
use support::*;

fn body(
    cx: &mut TestAppContext,
    spec: BodySpec,
) -> (AnyWindowHandle, Entity<SectionHost<BodySection>>) {
    let (win, host) = host_section(cx, |w, cx| BodySection::new(PathBuf::from("/c"), w, cx));
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| s.update(cx, |s, cx| s.load(&spec, w, cx)))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    (win, host)
}

fn last_body(cx: &mut TestAppContext, host: &Entity<SectionHost<BodySection>>) -> BodySpec {
    match host.read_with(cx, |h, _| h.edits.last().cloned()) {
        Some(Edit::Body(b)) => b,
        _ => panic!("no body edit"),
    }
}

fn click(cx: &mut TestAppContext, win: AnyWindowHandle, id: (&'static str, usize)) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.within("body-table").click(id, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

fn sel(path: &str, selected: bool) -> FileEntry {
    let mut e = FileEntry::new(path);
    e.selected = selected;
    e
}

#[test]
fn picked_paths_inside_the_collection_become_relative() {
    let root = Path::new("/c/col");
    let inside = Path::new("data").join("a.bin");
    assert_eq!(
        collection_relative(root, &root.join(&inside)),
        inside.to_string_lossy()
    );
    assert_eq!(
        collection_relative(root, Path::new("/elsewhere/b.txt")),
        "/elsewhere/b.txt"
    );
    assert_eq!(collection_relative(root, root), "/c/col");
    assert_eq!(
        collection_relative(root, Path::new("/c/col2/x")),
        "/c/col2/x"
    );
}

#[gpui_kit::test]
fn form_rows_edit_through_the_table(cx: &mut TestAppContext) {
    let (win, host) = body(cx, BodySpec::FormUrlEncoded(vec![KeyValue::new("a", "1")]));
    assert!(
        cx.update_window(win, |_, w, _| w.try_find("body-table").is_some()
            && w.try_find("body-code").is_none())
            .unwrap()
    );
    type_at_end(cx, win, Some("body-table"), ("kv-value", 0usize), "2");
    cx.run_until_parked();
    assert_eq!(
        last_body(cx, &host),
        BodySpec::FormUrlEncoded(vec![KeyValue::new("a", "12")])
    );
}

#[gpui_kit::test]
fn multipart_rows_pick_several_files(cx: &mut TestAppContext) {
    let (win, host) = body(cx, BodySpec::Json("{}".into()));
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.set_mode(BodyMode::Multipart, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    let table = s.read_with(cx, |s, _| s.table().clone());
    assert_eq!(
        table.read_with(cx, |t, _| t.columns()),
        KvColumns::MULTIPART
    );
    assert_eq!(last_body(cx, &host), BodySpec::Multipart(vec![]));
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| {
            s.load(
                &BodySpec::Multipart(vec![MultipartField::text("f", "")]),
                w,
                cx,
            )
        })
    })
    .unwrap();
    click(cx, win, ("kv-pick", 0));
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|o| {
        assert!(o.multiple && o.files && !o.directories);
        Some(vec![PathBuf::from("/c/a.png"), PathBuf::from("/x/b.txt")])
    });
    cx.run_until_parked();
    let want = MultipartField::files("f", vec!["a.png".into(), "/x/b.txt".into()]);
    assert_eq!(last_body(cx, &host), BodySpec::Multipart(vec![want]));
}

#[gpui_kit::test]
fn a_cancelled_pick_sends_nothing(cx: &mut TestAppContext) {
    let (win, host) = body(cx, BodySpec::Multipart(vec![MultipartField::text("f", "")]));
    click(cx, win, ("kv-pick", 0));
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    assert!(host.read_with(cx, |h, _| h.edits.is_empty()));
}

#[gpui_kit::test]
fn a_file_body_picks_one_file_and_selects_one_entry(cx: &mut TestAppContext) {
    let (win, host) = body(
        cx,
        BodySpec::File(vec![sel("a.bin", true), FileEntry::new("")]),
    );
    click(cx, win, ("kv-pick", 1));
    cx.simulate_path_prompt_response(|o| {
        assert!(!o.multiple);
        Some(vec![PathBuf::from("/c/b.json")])
    });
    cx.run_until_parked();
    assert_eq!(
        last_body(cx, &host),
        BodySpec::File(vec![
            sel("a.bin", true),
            typed("b.json", "application/json", false)
        ])
    );
    click(cx, win, ("kv-enabled", 1));
    assert_eq!(
        last_body(cx, &host),
        BodySpec::File(vec![
            sel("a.bin", false),
            typed("b.json", "application/json", true)
        ])
    );
}

fn typed(path: &str, content_type: &str, selected: bool) -> FileEntry {
    let mut e = sel(path, selected);
    e.content_type = content_type.into();
    e
}

// Picks `path` for row `ix` of a file body that starts as `files`, and returns the body sent.
fn pick_file(cx: &mut TestAppContext, files: Vec<FileEntry>, ix: usize, path: &str) -> BodySpec {
    let (win, host) = body(cx, BodySpec::File(files));
    click(cx, win, ("kv-pick", ix));
    let path = PathBuf::from(path);
    cx.simulate_path_prompt_response(move |_| Some(vec![path]));
    cx.run_until_parked();
    last_body(cx, &host)
}

#[gpui_kit::test]
fn picking_a_file_fills_a_blank_content_type_from_its_extension(cx: &mut TestAppContext) {
    assert_eq!(
        pick_file(cx, vec![FileEntry::new("")], 0, "/c/data.json"),
        BodySpec::File(vec![typed("data.json", "application/json", false)])
    );
}

#[gpui_kit::test]
fn picking_a_file_on_the_empty_row_fills_its_content_type(cx: &mut TestAppContext) {
    assert_eq!(
        pick_file(cx, vec![], 0, "/c/photo.PNG"),
        BodySpec::File(vec![typed("photo.PNG", "image/png", true)])
    );
}

#[gpui_kit::test]
fn picking_a_file_of_unknown_kind_fills_the_generic_binary_type(cx: &mut TestAppContext) {
    assert_eq!(
        pick_file(cx, vec![FileEntry::new("")], 0, "/c/blob.zzzq"),
        BodySpec::File(vec![typed("blob.zzzq", "application/octet-stream", false)])
    );
}

#[gpui_kit::test]
fn picking_a_file_keeps_a_content_type_the_user_typed(cx: &mut TestAppContext) {
    assert_eq!(
        pick_file(
            cx,
            vec![typed("old.bin", "text/csv", true)],
            0,
            "/c/data.json"
        ),
        BodySpec::File(vec![typed("data.json", "text/csv", true)])
    );
}

#[gpui_kit::test]
fn the_file_body_content_type_cell_says_what_a_blank_sends(cx: &mut TestAppContext) {
    let (_, host) = body(cx, BodySpec::File(vec![FileEntry::new("")]));
    let s = host.read_with(cx, |h, _| h.view.clone());
    let table = s.read_with(cx, |s, _| s.table().clone());
    let hint = |cx: &mut TestAppContext, ix: usize| {
        let input = table.read_with(cx, |t, _| t.content_type_input(ix).cloned().unwrap());
        input.read_with(cx, |i, _| i.presentation().placeholder().to_string())
    };
    assert_eq!(hint(cx, 0), "Blank sends an empty header");
    assert_eq!(hint(cx, 1), "Blank sends an empty header");
}

#[gpui_kit::test]
fn a_multipart_content_type_cell_keeps_its_plain_placeholder(cx: &mut TestAppContext) {
    let (_, host) = body(cx, BodySpec::Multipart(vec![MultipartField::text("f", "")]));
    let s = host.read_with(cx, |h, _| h.view.clone());
    let table = s.read_with(cx, |s, _| s.table().clone());
    let input = table.read_with(cx, |t, _| t.content_type_input(0).cloned().unwrap());
    assert_eq!(
        input.read_with(cx, |i, _| i.presentation().placeholder().to_string()),
        "Content type"
    );
}

// The workspace keeps the root as opened, through a symlink, while the file picker hands
// back the resolved path; a file inside is still collection-relative, either way round.
#[cfg(unix)]
#[test]
fn picked_paths_inside_a_symlinked_collection_become_relative() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real");
    std::fs::create_dir_all(real.join("data")).unwrap();
    std::fs::write(real.join("data").join("a.bin"), "x").unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let inside = Path::new("data").join("a.bin");
    assert_eq!(
        collection_relative(&link, &real.join(&inside)),
        inside.to_string_lossy()
    );
    assert_eq!(
        collection_relative(&real, &link.join(&inside)),
        inside.to_string_lossy()
    );
    let elsewhere = dir.path().join("b.txt");
    std::fs::write(&elsewhere, "y").unwrap();
    assert_eq!(
        collection_relative(&link, &elsewhere),
        elsewhere.to_string_lossy()
    );
}

// When the picked file does not exist its canonical path cannot be read, so a resolved
// spelling falls back to the path as given; the link's own spelling still strips lexically.
#[cfg(unix)]
#[test]
fn a_missing_file_under_a_symlinked_collection_falls_back_to_the_given_path() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real");
    std::fs::create_dir_all(real.join("data")).unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let missing = Path::new("data").join("gone.bin");
    let resolved = real.join(&missing);
    assert_eq!(
        collection_relative(&link, &resolved),
        resolved.to_string_lossy()
    );
    assert_eq!(
        collection_relative(&link, &link.join(&missing)),
        missing.to_string_lossy()
    );
}
