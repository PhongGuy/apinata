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
// A lone file is changed through its own button, and a new file comes through "Add another".
fn pick_file(cx: &mut TestAppContext, files: Vec<FileEntry>, ix: usize, path: &str) -> BodySpec {
    let n = files.len();
    let (win, host) = body(cx, BodySpec::File(files));
    match (n, ix) {
        (_, ix) if ix == n => press(cx, win, "body-file-add"),
        (1, 0) => press(cx, win, "body-file-change"),
        _ => click(cx, win, ("kv-pick", ix)),
    }
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
        pick_file(cx, vec![sel("a.bin", true)], 1, "/c/photo.PNG"),
        BodySpec::File(vec![
            sel("a.bin", true),
            typed("photo.PNG", "image/png", false)
        ])
    );
}

fn press(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(id, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

// Which file body view is drawn: the empty first step, the lone file, or the table.
#[derive(Debug, PartialEq)]
enum FileView {
    Empty,
    One,
    Table,
}

fn file_view(cx: &mut TestAppContext, win: AnyWindowHandle) -> FileView {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let shown = [
            w.try_find("body-file-empty").is_some(),
            w.try_find("body-file-one").is_some(),
            w.try_find("body-table").is_some(),
        ];
        match shown {
            [true, false, false] => FileView::Empty,
            [false, true, false] => FileView::One,
            [false, false, true] => FileView::Table,
            other => panic!("file body views shown: {other:?}"),
        }
    })
    .unwrap()
}

fn file_empty_state(cx: &mut TestAppContext, win: AnyWindowHandle) -> (Option<String>, bool) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let note = w
            .try_find("body-file-note")
            .map(|e| e.label().map(str::to_owned));
        let choose = w
            .try_find("body-file-choose")
            .map(|e| e.label().map(str::to_owned));
        assert_eq!(choose.clone().flatten().is_some(), note.is_some());
        if let Some(label) = choose {
            assert_eq!(label.as_deref(), Some("Choose file\u{2026}"));
        }
        (note.flatten(), w.try_find("body-table").is_some())
    })
    .unwrap()
}

// A file body with no files offers one first step in place of an empty table, and says what
// it sends; the first pick brings the table back with that file in it.
#[gpui_kit::test]
fn an_empty_file_body_offers_choose_file_and_says_what_it_sends(cx: &mut TestAppContext) {
    let (win, host) = body(cx, BodySpec::File(vec![]));
    assert_eq!(
        file_empty_state(cx, win),
        (
            Some("Sends the contents of one file as the request body.".to_owned()),
            false
        )
    );
    cx.update_window(win, |_, w, cx| w.click("body-file-choose", cx))
        .unwrap();
    cx.run_until_parked();
    cx.simulate_path_prompt_response(|o| {
        assert!(!o.multiple && o.files && !o.directories);
        Some(vec![PathBuf::from("/c/photo.PNG")])
    });
    cx.run_until_parked();
    assert_eq!(
        last_body(cx, &host),
        BodySpec::File(vec![typed("photo.PNG", "image/png", true)])
    );
    assert_eq!(file_empty_state(cx, win), (None, false));
    assert_eq!(file_view(cx, win), FileView::One);
}

#[gpui_kit::test]
fn a_cancelled_first_pick_keeps_the_empty_file_body(cx: &mut TestAppContext) {
    let (win, host) = body(cx, BodySpec::File(vec![]));
    cx.update_window(win, |_, w, cx| w.click("body-file-choose", cx))
        .unwrap();
    cx.run_until_parked();
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    assert!(host.read_with(cx, |h, _| h.edits.is_empty()));
    assert!(file_empty_state(cx, win).0.is_some());
}

// One entry with no file yet still shows as the lone file, offering to choose one.
#[gpui_kit::test]
fn a_lone_entry_without_a_file_offers_to_choose_one(cx: &mut TestAppContext) {
    let (win, _) = body(cx, BodySpec::File(vec![FileEntry::new("")]));
    assert_eq!(file_view(cx, win), FileView::One);
    let (name, button) = cx
        .update_window(win, |_, w, _| {
            (
                w.find("body-file-name").label().map(str::to_owned),
                w.find("body-file-change").label().map(str::to_owned),
            )
        })
        .unwrap();
    assert_eq!(name.as_deref(), Some("No file chosen"));
    assert_eq!(button.as_deref(), Some("Choose file\u{2026}"));
}

// What the lone file view says, as (name, content type label and value, size, change,
// remove, add); `None` where an element is missing.
type OneFile = [Option<String>; 7];

fn one_file(cx: &mut TestAppContext, win: AnyWindowHandle) -> OneFile {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let label = |id: &'static str| w.try_find(id).and_then(|e| e.label().map(str::to_owned));
        [
            label("body-file-name"),
            label("body-file-type"),
            w.try_find("body-file-type")
                .and_then(|e| e.value().map(str::to_owned)),
            label("body-file-size"),
            label("body-file-change"),
            label("body-file-remove"),
            label("body-file-add"),
        ]
    })
    .unwrap()
}

fn some(s: &str) -> Option<String> {
    Some(s.to_owned())
}

#[gpui_kit::test]
fn one_file_shows_alone_with_no_blank_row(cx: &mut TestAppContext) {
    let (win, _) = body(
        cx,
        BodySpec::File(vec![typed("data/photo.png", "image/png", true)]),
    );
    assert_eq!(file_view(cx, win), FileView::One);
    assert_eq!(
        one_file(cx, win),
        [
            some("photo.png"),
            some("Content type"),
            some("image/png"),
            None,
            some("Change\u{2026}"),
            some("Remove"),
            some("Add another file\u{2026}"),
        ]
    );
    let rows = cx
        .update_window(win, |_, w, _| {
            [("kv-pick", 0usize), ("kv-pick", 1), ("kv-enabled", 1)]
                .map(|id| w.try_find(id).is_some())
        })
        .unwrap();
    assert_eq!(rows, [false; 3]);
}

#[gpui_kit::test]
fn one_file_shows_its_size_when_it_can_be_read(cx: &mut TestAppContext) {
    let path = std::env::temp_dir().join(format!("apinata-file-size-{}.bin", std::process::id()));
    std::fs::write(&path, vec![0u8; 1536]).unwrap();
    let entry = typed(&path.to_string_lossy(), "", true);
    let (win, _) = body(cx, BodySpec::File(vec![entry]));
    let size = one_file(cx, win)[3].clone();
    std::fs::remove_file(&path).unwrap();
    assert_eq!(size.as_deref(), Some("1.5 KB"));
}

#[gpui_kit::test]
fn typing_in_the_lone_files_content_type_edits_the_body(cx: &mut TestAppContext) {
    let (win, host) = body(cx, BodySpec::File(vec![typed("a.txt", "text/plain", true)]));
    type_at_end(cx, win, None, "body-file-type", ";x");
    cx.run_until_parked();
    assert_eq!(
        last_body(cx, &host),
        BodySpec::File(vec![typed("a.txt", "text/plain;x", true)])
    );
}

#[gpui_kit::test]
fn change_swaps_the_lone_file(cx: &mut TestAppContext) {
    let (win, host) = body(
        cx,
        BodySpec::File(vec![typed("a.json", "application/json", true)]),
    );
    press(cx, win, "body-file-change");
    cx.simulate_path_prompt_response(|o| {
        assert!(!o.multiple && o.files && !o.directories);
        Some(vec![PathBuf::from("/c/photo.png")])
    });
    cx.run_until_parked();
    assert_eq!(
        last_body(cx, &host),
        BodySpec::File(vec![typed("photo.png", "image/png", true)])
    );
    assert_eq!(file_view(cx, win), FileView::One);
    assert_eq!(one_file(cx, win)[0].as_deref(), Some("photo.png"));
}

#[gpui_kit::test]
fn remove_returns_the_lone_file_to_the_empty_state(cx: &mut TestAppContext) {
    let (win, host) = body(cx, BodySpec::File(vec![typed("a.bin", "", true)]));
    press(cx, win, "body-file-remove");
    assert_eq!(last_body(cx, &host), BodySpec::File(vec![]));
    assert_eq!(file_view(cx, win), FileView::Empty);
    assert!(file_empty_state(cx, win).0.is_some());
}

// Several files keep the table, whose radios pick the one sent, with no empty row under it;
// "Add another file" adds a row, and removing down to one file shows it alone.
#[gpui_kit::test]
fn two_files_keep_the_table_and_radios_without_a_blank_row(cx: &mut TestAppContext) {
    let (win, host) = body(
        cx,
        BodySpec::File(vec![sel("a.bin", true), sel("b.bin", false)]),
    );
    assert_eq!(file_view(cx, win), FileView::Table);
    let ids = cx
        .update_window(win, |_, w, _| {
            [
                ("kv-pick", 0usize),
                ("kv-pick", 1),
                ("kv-pick", 2),
                ("kv-enabled", 2),
            ]
            .map(|id| w.try_find(id).is_some())
        })
        .unwrap();
    assert_eq!(ids, [true, true, false, false]);
    click(cx, win, ("kv-enabled", 1));
    assert_eq!(
        last_body(cx, &host),
        BodySpec::File(vec![sel("a.bin", false), sel("b.bin", true)])
    );
    press(cx, win, "body-file-add");
    cx.simulate_path_prompt_response(|_| Some(vec![PathBuf::from("/c/c.txt")]));
    cx.run_until_parked();
    assert_eq!(
        last_body(cx, &host),
        BodySpec::File(vec![
            sel("a.bin", false),
            sel("b.bin", true),
            typed("c.txt", "text/plain", false)
        ])
    );
    for _ in 0..2 {
        click(cx, win, ("kv-delete", 0));
    }
    assert_eq!(
        last_body(cx, &host),
        BodySpec::File(vec![typed("c.txt", "text/plain", true)])
    );
    assert_eq!(file_view(cx, win), FileView::One);
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
fn re_picking_replaces_a_type_filled_from_the_old_file(cx: &mut TestAppContext) {
    assert_eq!(
        pick_file(
            cx,
            vec![typed("a.json", "application/json", true)],
            0,
            "/c/photo.png"
        ),
        BodySpec::File(vec![typed("photo.png", "image/png", true)])
    );
}

#[gpui_kit::test]
fn re_picking_keeps_a_typed_type_that_differs_from_the_old_files(cx: &mut TestAppContext) {
    assert_eq!(
        pick_file(
            cx,
            vec![typed("a.json", "text/plain", true)],
            0,
            "/c/photo.png"
        ),
        BodySpec::File(vec![typed("photo.png", "text/plain", true)])
    );
}

#[gpui_kit::test]
fn the_file_body_content_type_cell_says_a_pick_fills_it(cx: &mut TestAppContext) {
    let (_, host) = body(cx, BodySpec::File(vec![FileEntry::new("")]));
    let s = host.read_with(cx, |h, _| h.view.clone());
    let table = s.read_with(cx, |s, _| s.table().clone());
    let hint = |cx: &mut TestAppContext, ix: usize| {
        let input = table.read_with(cx, |t, _| t.content_type_input(ix).cloned().unwrap());
        input.read_with(cx, |i, _| i.presentation().placeholder().to_string())
    };
    assert_eq!(hint(cx, 0), "Filled in when you pick a file");
    assert_eq!(hint(cx, 1), "Filled in when you pick a file");
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
