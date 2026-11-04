//! Saved examples written as Bruno writes them, from the
//! examples parity corpus, plus the draft, secret and error rules.

use apinata_core::{
    Collection, ContextOptions, ExampleBody, ExampleBodyKind, ExampleResponse, ItemPath,
    MemoryStore, SecretStore, TreeError, VarContext, resolve,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: c\n";

fn research() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/reference")
}

#[derive(serde::Deserialize)]
struct Case {
    id: String,
    input: String,
    file: String,
}

fn cases() -> Vec<Case> {
    let path = research().join("2026-10-11-examples-parity/cases.json");
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn expected(id: &str, file: &str) -> String {
    let path = research()
        .join("2026-10-11-examples-parity/expected")
        .join(id)
        .join(file);
    fs::read_to_string(path).unwrap()
}

fn ip(rel: &str) -> ItemPath {
    ItemPath::new(rel).expect("valid item path")
}

/// A collection holding `files` (relative path, text) besides its root file.
fn open(files: &[(&str, &str)]) -> (TempDir, Collection) {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("opencollection.yml"), ROOT).unwrap();
    for (rel, text) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let c = Collection::open(dir.path()).unwrap();
    assert!(c.load_errors().is_empty(), "{:?}", c.load_errors());
    (dir, c)
}

fn read(dir: &TempDir, rel: &str) -> String {
    fs::read_to_string(dir.path().join(rel)).unwrap()
}

fn response(
    status: u16,
    text: &str,
    headers: &[(&str, &str)],
    kind: ExampleBodyKind,
    data: &str,
) -> ExampleResponse {
    let mut r = ExampleResponse::new(status);
    if !text.is_empty() {
        r.status_text = Some(text.to_owned());
    }
    r.headers = headers
        .iter()
        .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
        .collect();
    r.body = Some(ExampleBody::new(kind, data));
    r
}

/// The Apinata edit matching each corpus case in `edits.mjs`.
fn apply(c: &mut Collection, id: &str, item: &ItemPath) -> Result<(), TreeError> {
    use ExampleBodyKind as K;
    let json = "{\n  \"id\": 8,\n  \"name\": \"{{name}}\"\n}";
    match id {
        "ex-add" => {
            let headers = [
                ("content-type", "application/json"),
                ("x-request-id", "abc"),
            ];
            let r = response(200, "OK", &headers, K::Json, json);
            c.add_example(item, "example (1)", "", &r)
                .map(|ix| assert_eq!(ix, 2))
        }
        "ex-add-first" => {
            let r = response(
                200,
                "OK",
                &[("content-type", "text/plain")],
                K::Text,
                "pong",
            );
            c.add_example(item, "example", "", &r)
                .map(|ix| assert_eq!(ix, 0))
        }
        "ex-add-description" => {
            let r = response(201, "Created", &[], K::Json, "{}");
            c.add_example(item, "happy", "the happy path\nwith two lines", &r)
                .map(drop)
        }
        "ex-add-binary" => {
            let headers = [("content-type", "image/png")];
            let r = response(200, "OK", &headers, K::Binary, "iVBORw0KGgoAAAANSUhEUg==");
            c.add_example(item, "logo", "", &r).map(drop)
        }
        "ex-add-quoted" => {
            let headers = [("content-type", "text/html")];
            let r = response(404, "Not Found", &headers, K::Html, "<p>gone</p>\n");
            c.add_example(item, "#1: missing", "", &r).map(drop)
        }
        "ex-add-empty" => {
            let r = response(299, "", &[], K::Text, "");
            c.add_example(item, "empty", "", &r).map(drop)
        }
        "ex-rename" => c.rename_example(item, 1, "bad request"),
        "ex-delete-first" | "ex-delete-only" => c.delete_example(item, 0),
        "ex-untouched" => c.rename(item, "renamed", false).map(drop),
        other => panic!("no Apinata edit for {other}"),
    }
}

#[test]
fn example_edits_match_bruno() {
    let all = cases();
    assert_eq!(all.len(), 10, "every case in edits.mjs has a test");
    let mut failures = Vec::new();
    for case in &all {
        let input = fs::read_to_string(research().join(&case.input)).unwrap();
        let (dir, mut c) = open(&[(case.file.as_str(), input.as_str())]);
        let item = ip(&case.file);
        apply(&mut c, &case.id, &item).unwrap_or_else(|e| panic!("{}: {e}", case.id));
        let actual = read(&dir, &case.file);
        let want = expected(&case.id, &case.file);
        if actual != want {
            failures.push(format!(
                "{}\n  expected {want:?}\n  actual   {actual:?}",
                case.id
            ));
        }
        let request = c.request(&item).unwrap();
        assert!(
            !request.is_dirty(),
            "{}: a write-through leaves a clean request clean",
            case.id
        );
    }
    assert!(
        failures.is_empty(),
        "{} of 10 differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

const PLAIN: &str = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: https://x.test/{{path}}\n  headers:\n    - name: Authorization\n      value: Bearer {{token}}\n";

#[test]
fn an_example_keeps_unsaved_edits_unsaved_and_copies_the_draft_request() {
    let (dir, mut c) = open(&[("a.yml", PLAIN)]);
    let item = ip("a.yml");
    c.request_mut(&item).unwrap().set_method("POST");
    let ix = c
        .add_example(&item, "  first  ", "  ", &ExampleResponse::new(200))
        .unwrap();
    assert_eq!(ix, 0);
    let disk = read(&dir, "a.yml");
    assert!(disk.contains("http:\n  method: GET\n"), "{disk}");
    assert!(
        disk.contains(
            "examples:\n  - name: first\n    request:\n      url: https://x.test/{{path}}\n      method: POST\n"
        ),
        "{disk}"
    );
    assert!(
        disk.contains("    response:\n      status: 200\n"),
        "{disk}"
    );
    assert!(
        !disk.contains("description"),
        "a blank description is left out: {disk}"
    );
    let request = c.request(&item).unwrap();
    assert!(request.is_dirty(), "the method edit is still unsaved");
    assert_eq!(request.method(), "POST");
    c.request_mut(&item).unwrap().save().unwrap();
    let saved = read(&dir, "a.yml");
    assert!(saved.contains("http:\n  method: POST\n"), "{saved}");
    assert!(
        saved.contains("  - name: first\n"),
        "a later save keeps the example: {saved}"
    );
}

#[test]
fn the_example_request_keeps_placeholders_never_resolved_secrets() {
    let env = "name: dev\nvariables:\n  - secret: true\n    name: token\n  - name: path\n    value: users\n";
    let (dir, mut c) = open(&[("a.yml", PLAIN), ("environments/dev.yml", env)]);
    let item = ip("a.yml");
    let store = MemoryStore::new();
    store
        .set(&c.secret_key("dev", "token"), "s3cr3t-value")
        .unwrap();
    let ctx = VarContext::new(
        &c,
        ContextOptions::new().env("dev").secrets(Arc::new(store)),
    )
    .expect("context");
    let resolved = resolve(c.request(&item).unwrap(), &ctx).expect("resolves");
    assert!(
        resolved
            .headers()
            .iter()
            .any(|(_, v)| v == "Bearer s3cr3t-value"),
        "the send would carry the secret"
    );
    let mut r = ExampleResponse::new(200);
    r.body = Some(ExampleBody::new(ExampleBodyKind::Text, "ok"));
    c.add_example(&item, "with secret", "", &r).unwrap();
    let disk = read(&dir, "a.yml");
    assert_eq!(disk.matches("value: Bearer {{token}}").count(), 2, "{disk}");
    assert_eq!(
        disk.matches("url: https://x.test/{{path}}").count(),
        2,
        "{disk}"
    );
    assert!(!disk.contains("s3cr3t-value"), "{disk}");
    assert!(!disk.contains("x.test/users"), "{disk}");
}

#[test]
fn bad_names_and_indexes_are_refused_without_writing() {
    let (dir, mut c) = open(&[("a.yml", PLAIN)]);
    let item = ip("a.yml");
    let before = read(&dir, "a.yml");
    let ok = ExampleResponse::new(200);
    assert!(matches!(
        c.add_example(&item, "  ", "", &ok),
        Err(TreeError::InvalidName { .. })
    ));
    assert!(matches!(
        c.add_example(&ip("gone.yml"), "x", "", &ok),
        Err(TreeError::NotFound { .. })
    ));
    assert!(matches!(
        c.rename_example(&item, 0, "x"),
        Err(TreeError::NotFound { .. })
    ));
    assert!(matches!(
        c.delete_example(&item, 0),
        Err(TreeError::NotFound { .. })
    ));
    assert_eq!(read(&dir, "a.yml"), before);
    c.add_example(&item, "one", "", &ok).unwrap();
    let one = read(&dir, "a.yml");
    assert!(matches!(
        c.rename_example(&item, 0, " \t"),
        Err(TreeError::InvalidName { .. })
    ));
    assert!(matches!(
        c.delete_example(&item, 1),
        Err(TreeError::NotFound { .. })
    ));
    assert_eq!(read(&dir, "a.yml"), one);
}

#[test]
fn a_new_example_name_is_trimmed_and_checked_against_the_request() {
    let (dir, mut c) = open(&[("a.yml", PLAIN)]);
    let item = ip("a.yml");
    assert_eq!(c.new_example_name(&item, "  dup \u{feff}").unwrap(), "dup");
    c.add_example(&item, "dup", "", &ExampleResponse::new(200))
        .unwrap();
    let before = read(&dir, "a.yml");
    match c.new_example_name(&item, "  dup  ") {
        Err(TreeError::ExampleNameTaken { item: at, name }) => {
            assert_eq!((at, name.as_str()), (item.clone(), "dup"));
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        c.new_example_name(&item, "\u{feff}"),
        Err(TreeError::InvalidName { .. })
    ));
    assert!(matches!(
        c.new_example_name(&ip("gone.yml"), "x"),
        Err(TreeError::NotFound { .. })
    ));
    assert_eq!(read(&dir, "a.yml"), before);
}

#[test]
fn a_file_changed_on_disk_is_a_conflict() {
    let (dir, mut c) = open(&[("a.yml", PLAIN)]);
    fs::write(dir.path().join("a.yml"), PLAIN.replace("GET", "PUT")).unwrap();
    let err = c.add_example(&ip("a.yml"), "x", "", &ExampleResponse::new(200));
    assert!(matches!(err, Err(TreeError::Conflict { .. })), "{err:?}");
    assert!(!read(&dir, "a.yml").contains("examples"));
}

/// The url the example saved from a request typed as `url` holds, and the request's own url
/// after the save.
fn saved_example_url(url: &str) -> (String, String) {
    let (_dir, mut c) = open(&[("a.yml", PLAIN)]);
    let item = ip("a.yml");
    c.request_mut(&item).unwrap().set_url(url);
    c.add_example(&item, "e", "", &ExampleResponse::new(200))
        .unwrap();
    let request = c.request(&item).unwrap();
    let example = request.examples().remove(0);
    (example.request.unwrap().url, request.url().to_owned())
}

#[test]
fn an_example_url_loses_its_username_and_password() {
    let typed = "http://acct-7781:pw-55@127.0.0.1:1/x?q=a@b";
    let (saved, request) = saved_example_url(typed);
    assert_eq!(saved, "http://127.0.0.1:1/x?q=a@b");
    assert_eq!(request, typed, "the request keeps what it was typed with");
}

#[test]
fn an_example_url_loses_a_lone_username() {
    let (saved, _) = saved_example_url("http://acct-7781@127.0.0.1:1/x");
    assert_eq!(saved, "http://127.0.0.1:1/x");
}

#[test]
fn a_schemeless_example_url_loses_its_userinfo_and_stays_schemeless() {
    let (saved, _) = saved_example_url("acct-7781:pw-55@127.0.0.1:1/x");
    assert_eq!(saved, "127.0.0.1:1/x");
}

#[test]
fn placeholder_userinfo_is_dropped_from_an_example_url() {
    let (saved, _) = saved_example_url("http://{{u}}:{{p}}@{{host}}/x");
    assert_eq!(saved, "http://{{host}}/x");
}

#[test]
fn a_schemeless_url_is_read_as_http_when_dropping_userinfo() {
    let (saved, _) = saved_example_url("mailto:someone@x.test");
    assert_eq!(saved, "x.test", "sent as http://mailto:someone@x.test");
}

#[test]
fn an_example_url_without_userinfo_is_kept_byte_for_byte() {
    for url in [
        "https://x.test/a@b?c=d@e#f@g",
        "{{baseUrl}}/a",
        "x.test/p@q",
        "{{baseUrl}}?to=a@b",
        "http://[::1]:8080/a@b",
    ] {
        assert_eq!(saved_example_url(url).0, url);
    }
}

#[test]
fn a_placeholder_scheme_still_loses_the_userinfo() {
    for (typed, want) in [
        (
            "{{proto}}://acct-7781:pw-55@127.0.0.1:1/x",
            "{{proto}}://127.0.0.1:1/x",
        ),
        (
            "http{{s}}://acct-7781:pw-55@127.0.0.1:1/x",
            "http{{s}}://127.0.0.1:1/x",
        ),
    ] {
        assert_eq!(saved_example_url(typed).0, want, "{typed}");
    }
}

#[test]
fn userinfo_starting_with_a_bracket_is_dropped() {
    let (saved, _) = saved_example_url("http://[acct-7781:pw-55@127.0.0.1:1/x");
    assert_eq!(saved, "http://127.0.0.1:1/x");
}

#[test]
fn an_empty_userinfo_is_dropped() {
    let (saved, _) = saved_example_url("http://@127.0.0.1:1/x");
    assert_eq!(saved, "http://127.0.0.1:1/x");
}

#[test]
fn bracketed_hosts_and_schemeless_placeholders_keep_their_at_signs() {
    for url in ["http://[::1]:1/x", "{{baseUrl}}/a@b"] {
        assert_eq!(saved_example_url(url).0, url);
    }
}

#[test]
fn an_example_already_on_disk_keeps_its_userinfo() {
    let existing = "examples:\n  - name: old\n    request:\n      url: http://acct-7781:pw-55@127.0.0.1:1/x\n      method: GET\n    response:\n      status: 200\n";
    let file = format!("{PLAIN}{existing}");
    let (dir, mut c) = open(&[("a.yml", &file)]);
    let item = ip("a.yml");
    c.add_example(&item, "new", "", &ExampleResponse::new(200))
        .unwrap();
    let disk = read(&dir, "a.yml");
    assert!(disk.contains(existing), "{disk}");
    assert!(disk.contains("  - name: new\n"), "{disk}");
}
