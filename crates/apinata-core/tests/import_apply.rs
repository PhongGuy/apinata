//! `ImportedRequest::apply_to` against Bruno's import bytes for
//! the import-parity corpus's `apinata-edge-cases`.

use apinata_core::import::ImportedRequest;
use apinata_core::{
    AuthSpec, BodySpec, Collection, FileEntry, ItemPath, KeyValue, Param, ParamKind,
    RequestSettings,
};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn expected(rel: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reference/2026-10-09-import-parity/expected/apinata-edge-cases");
    fs::read_to_string(root.join(rel)).expect("expected file")
}

/// Bruno's settings for a Postman request without protocolProfileBehavior.
fn postman_settings() -> RequestSettings {
    let mut settings = RequestSettings::new();
    settings.forward_authorization_header = false;
    settings
}

fn with<T>(mut entry: T, set: impl FnOnce(&mut T)) -> T {
    set(&mut entry);
    entry
}

/// Create `name` in a fresh collection, apply `request`, set `seq`, save, and return the bytes.
fn written(name: &str, seq: u64, request: &ImportedRequest) -> String {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path().join("c");
    let mut collection = Collection::create(&root, "c").expect("create");
    let path = collection
        .create_request(&ItemPath::root(), name)
        .expect("request");
    let file = collection.request_mut(&path).expect("loaded");
    request.apply_to(file).expect("applies");
    file.set_seq(seq);
    file.save().expect("saves");
    fs::read_to_string(path.to_path(&root)).expect("read back")
}

#[test]
fn url_and_params_are_written_verbatim_with_descriptions() {
    let mut r = ImportedRequest::new("GET", "https://{{host}}/users/:id?expand=true&debug=1");
    r.headers = vec![
        with(KeyValue::new("Accept", "application/json"), |h| {
            h.description = Some("What we take".into())
        }),
        with(KeyValue::new("X-Off", "1"), |h| h.enabled = false),
    ];
    r.params = vec![
        with(Param::new("expand", "true", ParamKind::Query), |p| {
            p.description = Some("Expand relations".into())
        }),
        with(Param::new("debug", "1", ParamKind::Query), |p| {
            p.enabled = false
        }),
        with(Param::new("id", "42", ParamKind::Path), |p| {
            p.description = Some("User id".into())
        }),
    ];
    r.docs = "Fetches one user".into();
    r.settings = Some(postman_settings());
    assert_eq!(written("Get user", 1, &r), expected("Users/Get user.yml"));
}

#[test]
fn bodies_auth_and_settings_match_bruno() {
    let mut form = ImportedRequest::new("POST", "https://api.example.com:8443/form?q=x");
    form.headers = vec![
        KeyValue::new("Content-Type", "application/x-www-form-urlencoded"),
        KeyValue::new("X-Trace", "on"),
    ];
    form.params = vec![Param::new("q", "x", ParamKind::Query)];
    form.body = BodySpec::FormUrlEncoded(vec![
        with(KeyValue::new("a", "1"), |f| {
            f.description = Some("First".into())
        }),
        with(KeyValue::new("b", "2"), |f| f.enabled = false),
    ]);
    form.settings = Some(postman_settings());
    assert_eq!(written("Form", 1, &form), expected("Bodies/Form.yml"));

    let mut binary = ImportedRequest::new("POST", "https://example.com/bin");
    binary.body = BodySpec::File(vec![with(FileEntry::new("/tmp/data.bin"), |f| {
        f.content_type = "application/octet-stream".into();
        f.selected = true;
    })]);
    binary.settings = Some(postman_settings());
    assert_eq!(written("Binary", 5, &binary), expected("Bodies/Binary.yml"));

    let mut basic = ImportedRequest::new("GET", "https://example.com/settings");
    basic.auth = AuthSpec::Basic {
        username: "u".into(),
        password: "p".into(),
    };
    basic.settings = Some(with(RequestSettings::new(), |s| {
        s.encode_url = false;
        s.follow_redirects = false;
        s.max_redirects = 3;
    }));
    assert_eq!(written("Settings", 3, &basic), expected("Settings.yml"));

    let mut none = ImportedRequest::new("GET", "https://example.com/none");
    none.auth = AuthSpec::None;
    none.settings = Some(postman_settings());
    assert_eq!(written("No auth", 5, &none), expected("No auth.yml"));
}

#[test]
fn empty_docs_leave_the_file_docs_and_given_docs_replace_them() {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path().join("c");
    let mut collection = Collection::create(&root, "c").expect("create");
    let path = collection
        .create_request(&ItemPath::root(), "r")
        .expect("request");
    let file = collection.request_mut(&path).expect("loaded");
    file.set_docs("Keep me");
    ImportedRequest::new("GET", "https://x.test")
        .apply_to(file)
        .expect("applies");
    assert_eq!(file.docs(), "Keep me");
    let mut documented = ImportedRequest::new("GET", "https://x.test");
    documented.docs = "From the import".into();
    documented.apply_to(file).expect("applies");
    assert_eq!(file.docs(), "From the import");
}

#[test]
fn omitted_headers_are_written_and_an_empty_list_keeps_the_files_own() {
    let dir = TempDir::new().unwrap();
    let mut collection = Collection::create(&dir.path().join("c"), "c").expect("create");
    let path = collection
        .create_request(&ItemPath::root(), "r")
        .expect("request");
    let file = collection.request_mut(&path).expect("loaded");
    let mut omitting = ImportedRequest::new("GET", "https://x.test");
    omitting.omitted_headers = vec!["user-agent".into()];
    omitting.apply_to(file).expect("applies");
    assert_eq!(file.omitted_headers(), ["user-agent"]);
    ImportedRequest::new("GET", "https://x.test")
        .apply_to(file)
        .expect("applies");
    assert_eq!(file.omitted_headers(), ["user-agent"]);
}
