//! `extensions.bruno.presets` on `opencollection.yml`: read as Bruno's reader does, written with
//! Bruno's bytes, and a file that is not edited keeps its bytes.

use crate::config_parity;

use apinata_core::{Collection, CollectionPresets, RequestKind};
use config_parity::check_corpus;
use std::fs;
use tempfile::TempDir;

const CORPUS: &str = "2026-10-19-presets-parity";

fn open(root: &str) -> (TempDir, Collection) {
    let dir = TempDir::new().expect("tempdir");
    fs::write(dir.path().join("opencollection.yml"), root).expect("root");
    let c = Collection::open(dir.path()).expect("opens");
    (dir, c)
}

fn saved(dir: &TempDir) -> String {
    fs::read_to_string(dir.path().join("opencollection.yml")).expect("read")
}

const HEAD: &str = "opencollection: 1.0.0\ninfo:\n  name: t\n";
/// `HEAD` as the writer lays it out, with a blank line after the version.
const WRITTEN: &str = "opencollection: 1.0.0\n\ninfo:\n  name: t\n";

#[test]
fn presets_read_as_bruno_reads_them() {
    let (_d, c) = open(&format!(
        "{HEAD}extensions:\n  bruno:\n    presets:\n      request:\n        type: graphql\n        url: https://x.test\n      defaultEnvironment: Local\n"
    ));
    let p = c.file().presets();
    assert_eq!(p.request_type, "graphql");
    assert_eq!(p.request_url, "https://x.test");
    assert_eq!(p.default_environment(), "Local");
    assert_eq!(p.kind(), RequestKind::Graphql);

    let (_d, c) = open(HEAD);
    assert_eq!(c.file().presets(), CollectionPresets::default());
    assert_eq!(c.file().presets().kind(), RequestKind::Http);

    // The reference maps an absent key to an empty string, and an unknown type means HTTP.
    let (_d, c) = open(&format!(
        "{HEAD}extensions:\n  bruno:\n    presets:\n      request:\n        type: grpc\n"
    ));
    assert_eq!(c.file().presets().request_type, "grpc");
    assert_eq!(c.file().presets().request_url, "");
    assert_eq!(c.file().presets().kind(), RequestKind::Http);
}

#[test]
fn preset_edits_match_bruno() {
    check_corpus(CORPUS, "presets-", |id, c| {
        let file = c.file_mut();
        let mut p = file.presets();
        (p.request_type, p.request_url) = match id {
            "presets-set-both" => ("graphql", "https://api.example.test"),
            "presets-set-url" => ("", "https://api.example.test"),
            "presets-set-type" => ("http", ""),
            "presets-set-unknown-type" => ("soap", "https://api.example.test"),
            "presets-edit-keeps-neighbours" => ("graphql", "https://other.example.test"),
            "presets-clear-keeps-environment" | "presets-clear-all" | "presets-clear-last" => {
                ("", "")
            }
            other => panic!("unexpected case {other}"),
        }
        .into_pair();
        file.set_presets(&p);
        file.save().expect("saves");
    });
}

/// `(&str, &str)` into owned strings, so the match above stays one table.
trait IntoPair {
    fn into_pair(self) -> (String, String);
}

impl IntoPair for (&str, &str) {
    fn into_pair(self) -> (String, String) {
        (self.0.to_owned(), self.1.to_owned())
    }
}

#[test]
fn writing_the_presets_just_read_keeps_the_bytes() {
    for name in ["plain", "full", "request-only", "only-presets"] {
        let path = format!("normalized/{name}/opencollection.yml");
        let text = fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/reference")
                .join(CORPUS)
                .join(path),
        )
        .expect("input");
        let (dir, mut c) = open(&text);
        let p = c.file().presets();
        c.file_mut().set_presets(&p);
        assert!(!c.file().is_dirty(), "{name} is clean");
        c.file_mut().save().expect("saves");
        assert_eq!(saved(&dir), text, "{name}");
    }
}

#[test]
fn unknown_siblings_and_keys_survive_an_edit() {
    let (dir, mut c) = open(&format!(
        "{HEAD}extensions:\n  other: 1\n  bruno:\n    scripts:\n      flow: sequential\n    presets:\n      mystery: 2\n      request:\n        type: http\n        extra: 3\n"
    ));
    let mut p = c.file().presets();
    p.request_url = "https://y.test".into();
    c.file_mut().set_presets(&p);
    c.file_mut().save().expect("saves");
    assert_eq!(
        saved(&dir),
        format!(
            "{WRITTEN}extensions:\n  other: 1\n  bruno:\n    scripts:\n      flow: sequential\n    presets:\n      mystery: 2\n      request:\n        type: http\n        extra: 3\n        url: https://y.test\n"
        )
    );
}

#[test]
fn presets_are_written_into_a_file_that_has_no_extensions() {
    let (dir, mut c) = open(HEAD);
    c.file_mut()
        .set_presets(&CollectionPresets::new("graphql", "https://z.test"));
    c.file_mut().save().expect("saves");
    assert_eq!(
        saved(&dir),
        format!(
            "{WRITTEN}extensions:\n  bruno:\n    presets:\n      request:\n        type: graphql\n        url: https://z.test\n"
        )
    );
}

fn new_draft(root: &str, kind_from_presets: bool) -> (RequestKind, String, bool) {
    let (_d, mut c) = open(root);
    let parent = apinata_core::ItemPath::root();
    let draft = if kind_from_presets {
        c.create_draft_from_presets(&parent)
    } else {
        c.create_draft(&parent)
    }
    .expect("draft");
    let r = c.request(&draft).expect("draft request");
    (r.kind(), r.url().to_owned(), r.is_dirty())
}

const GRAPHQL_PRESET: &str = "opencollection: 1.0.0\ninfo:\n  name: t\nextensions:\n  bruno:\n    presets:\n      request:\n        type: graphql\n        url: https://g.test\n";

#[test]
fn new_drafts_start_from_the_presets() {
    // A preset-aware draft takes both the kind and the URL, and is clean until edited.
    assert_eq!(
        new_draft(GRAPHQL_PRESET, true),
        (RequestKind::Graphql, "https://g.test".into(), false)
    );
    // A plain draft is HTTP but still starts with the preset URL.
    assert_eq!(
        new_draft(GRAPHQL_PRESET, false),
        (RequestKind::Http, "https://g.test".into(), false)
    );
    // Without presets nothing changes.
    assert_eq!(
        new_draft(HEAD, true),
        (RequestKind::Http, String::new(), false)
    );
}

fn new_saved(root: &str, kind: Option<RequestKind>) -> (RequestKind, String, String) {
    let (dir, mut c) = open(root);
    let item = c
        .create_request_from_presets(&apinata_core::ItemPath::root(), "made", kind)
        .expect("request");
    let r = c.request(&item).expect("request");
    let file = fs::read_to_string(dir.path().join("made.yml")).expect("file");
    (r.kind(), r.url().to_owned(), file)
}

#[test]
fn new_saved_requests_start_from_the_presets() {
    // The preset gives the kind and the URL, and the file holds the URL under the kind's key.
    let (kind, url, file) = new_saved(GRAPHQL_PRESET, None);
    assert_eq!(
        (kind, url.as_str()),
        (RequestKind::Graphql, "https://g.test")
    );
    assert!(file.contains("graphql:") && file.contains("url: https://g.test"));

    // An explicit kind wins over the preset kind; the URL still comes from the preset.
    let (kind, url, _) = new_saved(GRAPHQL_PRESET, Some(RequestKind::Http));
    assert_eq!((kind, url.as_str()), (RequestKind::Http, "https://g.test"));

    // Without presets this is the plain template, or the explicit kind.
    let (kind, url, _) = new_saved(HEAD, None);
    assert_eq!((kind, url.as_str()), (RequestKind::Http, ""));
    let (kind, url, _) = new_saved(HEAD, Some(RequestKind::Graphql));
    assert_eq!((kind, url.as_str()), (RequestKind::Graphql, ""));
}
