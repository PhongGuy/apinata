//! Headers, auth, variables and docs on `folder.yml` and `opencollection.yml`: Bruno's bytes
//! on normalised files, minimal edits elsewhere.

use crate::config_parity;

use apinata_core::{AuthSpec, Collection, EditError, ItemPath, KeyValue, VarValue, Variable};
use config_parity::check;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn item(path: &str) -> ItemPath {
    ItemPath::new(path).expect("valid item path")
}

fn disabled(name: &str, value: &str) -> KeyValue {
    let mut kv = KeyValue::new(name, value);
    kv.enabled = false;
    kv
}

/// A collection whose `opencollection.yml` is `root` and whose folder `f` has `folder`.
fn collection_with(root: &str, folder: &str) -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    fs::write(dir.path().join("opencollection.yml"), root).expect("root");
    fs::create_dir(dir.path().join("f")).expect("mkdir");
    fs::write(dir.path().join("f/folder.yml"), folder).expect("folder.yml");
    dir
}

#[test]
fn folder_headers_vars_and_auth_match_bruno() {
    check("folder-lists-", |id, c| {
        let file = c.folder_file_mut(&item("folder_1")).expect("folder file");
        match id {
            "folder-lists-headers" => file
                .set_headers(vec![KeyValue::new("x-a", "1"), disabled("x-b", "{{b}}")])
                .expect("valid"),
            "folder-lists-vars" => {
                file.set_vars(vec![Variable::new("v", VarValue::Text("1".into()))])
            }
            "folder-lists-auth-bearer" => file.set_auth(AuthSpec::Bearer {
                token: "t0k".into(),
            }),
            "folder-lists-auth-inherit" => file.set_auth(AuthSpec::Inherit),
            other => panic!("unexpected case {other}"),
        }
        file.save().expect("saves");
    });
}

#[test]
fn collection_headers_vars_and_auth_match_bruno() {
    check("collection-lists-", |id, c| {
        let file = c.file_mut();
        match id {
            "collection-lists-headers" => file
                .set_headers(vec![KeyValue::new("x-c", "c")])
                .expect("valid"),
            "collection-lists-vars-edit" => {
                let mut vars = file.vars();
                vars[0].value = VarValue::Text("changed".into());
                file.set_vars(vars);
            }
            "collection-lists-auth-basic" => file.set_auth(AuthSpec::Basic {
                username: "u".into(),
                password: "{{pw}}".into(),
            }),
            other => panic!("unexpected case {other}"),
        }
        file.save().expect("saves");
    });
}

#[test]
fn docs_match_bruno() {
    check("folder-docs", |_, c| {
        let file = c.folder_file_mut(&item("folder_1")).expect("folder file");
        file.set_docs("# Folder\nnotes");
        file.save().expect("saves");
    });
    check("collection-docs", |_, c| {
        c.file_mut().set_docs("# Collection");
        c.file_mut().save().expect("saves");
    });
}

#[test]
fn docs_read_text_or_content_and_blank_text_removes_them() {
    let root = "opencollection: 1.0.0\ninfo:\n  name: c\ndocs:\n  content: mapped text\n  type: text/markdown\n";
    let dir = collection_with(root, "info:\n  name: f\ndocs: plain text\n");
    let mut c = Collection::open(dir.path()).expect("opens");
    assert_eq!(
        c.folder(&item("f")).unwrap().file().unwrap().docs(),
        "plain text"
    );
    assert_eq!(c.file().docs(), "mapped text");

    let folder = c.folder_file_mut(&item("f")).unwrap();
    folder.set_docs("now mapped");
    folder.save().expect("saves");
    assert_eq!(
        read(&dir.path().join("f/folder.yml")),
        "info:\n  name: f\n\ndocs:\n  content: now mapped\n  type: text/markdown\n"
    );
    c.file_mut().set_docs(" \n\t");
    c.file_mut().save().expect("saves");
    assert_eq!(
        read(&dir.path().join("opencollection.yml")),
        "opencollection: 1.0.0\n\ninfo:\n  name: c\n"
    );
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).expect("read")
}

#[test]
fn getters_read_the_request_block() {
    let folder = "info:\n  name: f\nrequest:\n  headers:\n    - name: A\n      value: 1\n      disabled: true\n  auth:\n    type: apikey\n    key: k\n    value: v\n    placement: query\n  variables:\n    - name: n\n      value:\n        type: number\n        data: \"7\"\n";
    let dir = collection_with("opencollection: 1.0.0\ninfo:\n  name: c\n", folder);
    let c = Collection::open(dir.path()).expect("opens");
    let file = c.folder(&item("f")).unwrap().file().unwrap();
    assert_eq!(file.headers(), vec![disabled("A", "1")]);
    assert_eq!(
        file.auth(),
        AuthSpec::ApiKey {
            key: "k".into(),
            value: "v".into(),
            placement: apinata_core::ApiKeyPlacement::Query
        }
    );
    assert_eq!(
        file.vars(),
        vec![Variable::new(
            "n",
            VarValue::Typed {
                kind: "number".into(),
                data: "7".into()
            }
        )]
    );
    assert_eq!(c.file().headers(), vec![]);
    assert_eq!(c.file().auth(), AuthSpec::None);
}

#[test]
fn clearing_the_last_default_removes_the_request_block() {
    let folder =
        "info:\n  name: f\nrequest:\n  headers:\n    - name: A\n      value: a\n  auth: inherit\n";
    let dir = collection_with("opencollection: 1.0.0\ninfo:\n  name: c\n", folder);
    let mut c = Collection::open(dir.path()).expect("opens");
    let file = c.folder_file_mut(&item("f")).unwrap();
    file.set_headers(vec![]).expect("valid");
    file.set_auth(AuthSpec::None);
    file.save().expect("saves");
    assert_eq!(
        fs::read_to_string(dir.path().join("f/folder.yml")).unwrap(),
        "info:\n  name: f\n"
    );
}

#[test]
fn a_same_mode_auth_edit_keeps_unknown_keys() {
    let root = "opencollection: 1.0.0\ninfo:\n  name: c\nrequest:\n  auth:\n    type: bearer\n    x-note: keep\n    token: old\n";
    let dir = collection_with(root, "info:\n  name: f\n");
    let mut c = Collection::open(dir.path()).expect("opens");
    c.file_mut().set_auth(AuthSpec::Bearer {
        token: "new".into(),
    });
    c.file_mut().save().expect("saves");
    assert_eq!(
        fs::read_to_string(dir.path().join("opencollection.yml")).unwrap(),
        "opencollection: 1.0.0\n\ninfo:\n  name: c\n\nrequest:\n  auth:\n    type: bearer\n    x-note: keep\n    token: new\n"
    );
}

#[test]
fn writing_back_what_was_read_leaves_the_files_clean() {
    let folder = "info:\n  name: f\nrequest:\n  headers:\n    - name: A\n      x-note: n\n      value: 1\n  variables:\n    - name: v\n      value: 2\n";
    let dir = collection_with("opencollection: 1.0.0\ninfo:\n  name: c\n", folder);
    let mut c = Collection::open(dir.path()).expect("opens");
    let file = c.folder_file_mut(&item("f")).unwrap();
    let (headers, vars, auth) = (file.headers(), file.vars(), file.auth());
    file.set_headers(headers).expect("valid");
    file.set_vars(vars);
    file.set_auth(auth);
    assert!(!file.is_dirty());
}

#[test]
fn a_header_name_with_a_line_break_is_refused_and_changes_nothing() {
    let dir = collection_with(
        "opencollection: 1.0.0\ninfo:\n  name: c\n",
        "info:\n  name: f\n",
    );
    let mut c = Collection::open(dir.path()).expect("opens");
    let err = c
        .file_mut()
        .set_headers(vec![
            KeyValue::new("ok", "1"),
            KeyValue::new("bad\r\nX", "2"),
        ])
        .expect_err("refused");
    assert_eq!(err, EditError::InvalidHeaderName { row: 1 });
    assert!(!c.file().is_dirty());
}
