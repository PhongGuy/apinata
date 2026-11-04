//! `import_collection`: text and URL sources, where the items land, confinement of `into`, and
//! the approval a fetch of a non-local host needs.

use crate::common;

use apinata_core::Trash;
use common::{FakeTrash, ROOT, TestServer, files_under};
use serde_json::{Value, json};
use std::fs;
use std::sync::Arc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SPEC: &str = r#"{"openapi":"3.0.0","info":{"title":"Petstore","version":"1"},
"paths":{"/pets":{"get":{"summary":"List pets","tags":["pets"]}},
"/users":{"get":{"summary":"List users","tags":["users"]}}}}"#;

const POSTMAN: &str = r#"{"info":{"name":"Col","schema":"https://schema.getpostman.com/json/collection/v2.1.0/collection.json"},
"item":[{"name":"Get","request":{"method":"GET","url":"http://127.0.0.1:1/x"}}],
"variable":[{"key":"a","value":"1"}],
"auth":{"type":"bearer","bearer":[{"key":"token","value":"t"}]}}"#;

const EXISTING: &str = "info:\n  name: api\n  type: folder\n  seq: 1\n";

async fn shop() -> TestServer {
    TestServer::start(&[("opencollection.yml", ROOT), ("api/folder.yml", EXISTING)]).await
}

async fn serve_spec() -> MockServer {
    let upstream = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/petstore.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(SPEC))
        .mount(&upstream)
        .await;
    upstream
}

fn imported(server: &TestServer) -> Vec<(String, String)> {
    files_under(server.root())
        .into_iter()
        .filter(|(rel, _)| rel.starts_with("imports/"))
        .collect()
}

#[tokio::test]
async fn a_text_import_writes_the_items_under_the_default_folder_and_they_list() {
    let server = shop().await;
    let out = server.ok("import_collection", json!({"text": SPEC})).await;
    assert_eq!(out["collections"][0]["path"], "imports/Petstore");
    assert_eq!(out["collections"][0]["requests"], 2);
    assert_eq!(out["collections"][0]["folders"], 2);
    let files = imported(&server);
    let names: Vec<&str> = files.iter().map(|(rel, _)| rel.as_str()).collect();
    assert!(
        names.contains(&"imports/Petstore/pets/List pets.yml"),
        "{names:?}"
    );
    assert!(
        names.contains(&"imports/Petstore/users/List users.yml"),
        "{names:?}"
    );
    assert!(
        !names.iter().any(|n| n.ends_with("opencollection.yml")),
        "{names:?}"
    );
    // The wrapper folders sit after the existing `api` folder.
    let seq = |rel: &str| files.iter().find(|(r, _)| r == rel).expect(rel).1.clone();
    assert!(
        seq("imports/folder.yml").contains("seq: 2"),
        "{}",
        seq("imports/folder.yml")
    );
    let list = server
        .ok("list_items", json!({"path": "imports/Petstore/pets"}))
        .await;
    assert_eq!(
        list["items"][0]["path"],
        "imports/Petstore/pets/List pets.yml"
    );
}

#[tokio::test]
async fn a_second_import_of_the_same_collection_gets_a_unique_folder() {
    let server = shop().await;
    server.ok("import_collection", json!({"text": SPEC})).await;
    let out = server.ok("import_collection", json!({"text": SPEC})).await;
    assert_eq!(out["collections"][0]["path"], "imports/Petstore 1");
}

#[tokio::test]
async fn into_names_another_relative_folder() {
    let server = shop().await;
    let out = server
        .ok(
            "import_collection",
            json!({"text": SPEC, "into": "vendor/apis"}),
        )
        .await;
    assert_eq!(out["collections"][0]["path"], "vendor/apis/Petstore");
    assert!(server.root().join("vendor/apis/Petstore/pets").is_dir());
}

#[tokio::test]
async fn root_headers_vars_and_auth_are_reported_as_skipped() {
    let server = shop().await;
    let out = server
        .ok("import_collection", json!({"text": POSTMAN}))
        .await;
    let skipped = out["skipped"].to_string();
    assert!(skipped.contains("variables"), "{skipped}");
    assert!(skipped.contains("auth"), "{skipped}");
    assert!(server.root().join("imports/Col/Get.yml").is_file());
}

#[tokio::test]
async fn a_local_url_import_equals_the_text_import() {
    let upstream = serve_spec().await;
    let from_text = shop().await;
    from_text
        .ok("import_collection", json!({"text": SPEC}))
        .await;
    let from_url = shop().await;
    from_url
        .ok(
            "import_collection",
            json!({"url": format!("{}/petstore.json", upstream.uri())}),
        )
        .await;
    assert_eq!(imported(&from_text), imported(&from_url));
    assert!(!imported(&from_url).is_empty());
}

#[tokio::test]
async fn a_non_local_url_without_an_inbox_is_refused_before_any_network_io() {
    let server = TestServer::start_without_inbox(&[("opencollection.yml", ROOT)]).await;
    // `.invalid` cannot resolve, so a refusal that names trust proves the policy ran first.
    let message = server
        .err(
            "import_collection",
            json!({"url": "http://specs.example.invalid/api.json"}),
        )
        .await;
    assert!(message.contains("not trusted"), "{message}");
    assert!(message.contains("no approval directory"), "{message}");
    assert!(imported(&server).is_empty());
}

#[tokio::test]
async fn a_non_local_url_files_an_approval_and_writes_nothing() {
    let server = shop().await;
    let message = server
        .err(
            "import_collection",
            json!({"url": "http://specs.example.invalid/api.json"}),
        )
        .await;
    assert!(message.contains("Approval requested"), "{message}");
    assert!(message.contains("send_request"), "{message}");
    let pending = server.inbox().pending().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].host, "specs.example.invalid");
    assert_eq!(pending[0].request, "import_collection");
    assert!(imported(&server).is_empty());
}

#[tokio::test]
async fn a_denied_host_is_refused_without_filing() {
    let server = shop().await;
    server.deny("specs.example.invalid");
    let message = server
        .err(
            "import_collection",
            json!({"url": "http://specs.example.invalid/api.json"}),
        )
        .await;
    assert!(message.contains("denied"), "{message}");
    assert!(server.inbox().pending().unwrap().is_empty());
}

#[tokio::test]
async fn a_trusted_non_local_host_passes_the_policy_and_is_fetched() {
    // 0.0.0.0 is not a loopback name to the policy, yet connecting to it reaches the
    // 127.0.0.1 listener on this machine, so the whole path runs without leaving it.
    let upstream = serve_spec().await;
    let port = common::port_of(&upstream.uri());
    let server = shop().await;
    let url = format!("http://0.0.0.0:{port}/petstore.json");
    let refused = server.err("import_collection", json!({"url": url})).await;
    assert!(refused.contains("not trusted"), "{refused}");
    server.trust("0.0.0.0");
    let out = server.ok("import_collection", json!({"url": url})).await;
    assert_eq!(out["collections"][0]["path"], "imports/Petstore");
}

/// A local hop that redirects to `0.0.0.0:<port>`, a name the policy treats as non-local that
/// still dials the second local listener. Returns the first hop's URL and the second server.
async fn redirect_to_unapproved() -> (MockServer, MockServer, String) {
    let upstream = serve_spec().await;
    let port = common::port_of(&upstream.uri());
    let front = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/start"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", format!("http://0.0.0.0:{port}/petstore.json")),
        )
        .mount(&front)
        .await;
    let url = format!("{}/start", front.uri());
    (front, upstream, url)
}

#[tokio::test]
async fn a_redirect_hop_to_an_unapproved_host_is_refused_and_never_fetched() {
    let (_front, upstream, url) = redirect_to_unapproved().await;
    let server = shop().await;
    let message = server.err("import_collection", json!({"url": url})).await;
    assert!(message.contains("not trusted"), "{message}");
    assert!(message.contains("hop 1"), "{message}");
    let pending = server.inbox().pending().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].host, "0.0.0.0");
    assert_eq!(pending[0].request, "import_collection");
    assert!(imported(&server).is_empty());
    assert!(upstream.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_redirect_hop_to_a_trusted_host_is_followed() {
    let (_front, upstream, url) = redirect_to_unapproved().await;
    let server = shop().await;
    server.trust("0.0.0.0");
    let out = server.ok("import_collection", json!({"url": url})).await;
    assert_eq!(out["collections"][0]["path"], "imports/Petstore");
    assert_eq!(upstream.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_redirect_hop_to_a_denied_host_is_refused_and_never_fetched() {
    let (_front, upstream, url) = redirect_to_unapproved().await;
    let server = shop().await;
    server.deny("0.0.0.0");
    let message = server.err("import_collection", json!({"url": url})).await;
    assert!(message.contains("denied"), "{message}");
    assert!(server.inbox().pending().unwrap().is_empty());
    assert!(imported(&server).is_empty());
    assert!(upstream.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn into_cannot_leave_the_served_root() {
    let server = shop().await;
    for into in [
        "..",
        "../out",
        "a/../../out",
        "/tmp/apinata-out",
        "a//b",
        ".",
        "",
    ] {
        let message = server
            .err("import_collection", json!({"text": SPEC, "into": into}))
            .await;
        assert!(message.contains("`into`"), "{into}: {message}");
    }
    assert!(imported(&server).is_empty());
    assert!(!server.root().join("imports").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn into_cannot_pass_through_a_symlink() {
    let outside = tempfile::tempdir().unwrap();
    let server = shop().await;
    std::os::unix::fs::symlink(outside.path(), server.root().join("link")).unwrap();
    for into in ["link", "link/deeper"] {
        let message = server
            .err("import_collection", json!({"text": SPEC, "into": into}))
            .await;
        assert!(message.contains("symlink"), "{into}: {message}");
    }
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn exactly_one_of_url_and_text_is_required() {
    let server = shop().await;
    let both = server
        .err(
            "import_collection",
            json!({"text": SPEC, "url": "http://127.0.0.1:1/x"}),
        )
        .await;
    assert!(both.contains("exactly one of `url` and `text`"), "{both}");
    let neither = server.err("import_collection", json!({})).await;
    assert!(
        neither.contains("exactly one of `url` and `text`"),
        "{neither}"
    );
}

#[tokio::test]
async fn options_are_validated_like_the_cli() {
    let server = shop().await;
    let cases: [(Value, &str); 4] = [
        (json!({"text": SPEC, "format": "nope"}), "unknown format"),
        (json!({"text": SPEC, "group_by": "folders"}), "group_by"),
        (json!({"text": POSTMAN, "group_by": "path"}), "OpenAPI"),
        (json!({"text": SPEC, "name": "x"}), "HAR"),
    ];
    for (args, expect) in cases {
        let message = server.err("import_collection", args.clone()).await;
        assert!(message.contains(expect), "{args}: {message}");
    }
    assert!(imported(&server).is_empty());
}

#[tokio::test]
async fn the_description_says_a_non_local_fetch_needs_approval() {
    let server = shop().await;
    let tools = server.tools().await;
    let tool = tools
        .iter()
        .find(|t| t.name == "import_collection")
        .unwrap();
    let description = tool.description.as_deref().unwrap();
    assert!(description.contains("approval"), "{description}");
}

// ---- update from a spec ----

const V1: &str = include_str!("../../apinata-core/tests/fixtures/openapi/update-v1.json");
const V2: &str = include_str!("../../apinata-core/tests/fixtures/openapi/update-v2.json");
const TEST: &str = "runtime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n";

/// A served collection that is the v1 spec's own import, so its root file holds the baseline.
async fn from_v1() -> TestServer {
    use apinata_core::MemoryStore;
    use apinata_core::import::{CollectionOptions, parse_collections, write_collection};
    let parent = tempfile::tempdir().unwrap();
    let bundles = parse_collections(None, V1.as_bytes(), &CollectionOptions::default()).unwrap();
    write_collection(
        parent.path(),
        &bundles[0].collection,
        &[],
        &MemoryStore::new(),
    )
    .unwrap();
    let files: Vec<(String, String)> = files_under(&parent.path().join("Shop"));
    TestServer::start_owned(&files).await
}

fn all(server: &TestServer) -> Vec<(String, String)> {
    let mut files = files_under(server.root());
    files.sort();
    files
}

fn add_test(server: &TestServer, rel: &str) {
    let path = server.root().join(rel);
    let text = fs::read_to_string(&path).unwrap();
    assert!(!text.contains("runtime:"), "{text}");
    fs::write(path, format!("{text}{TEST}")).unwrap();
}

#[tokio::test]
async fn update_refreshes_adds_and_orphans_and_keeps_local_tests() {
    let server = from_v1().await;
    add_test(&server, "people/getUser.yml");
    let out = server
        .ok("import_collection", json!({"text": V2, "update": true}))
        .await;
    let mut keys: Vec<&str> = out
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    for k in [
        "collection",
        "dry_run",
        "added",
        "changed",
        "kept",
        "edited",
        "edited_fields",
        "orphaned",
        "pruned",
        "notes",
    ] {
        assert!(keys.contains(&k), "{k} in {keys:?}");
    }
    keys.retain(|k| !["collections", "skipped"].contains(k));
    assert_eq!(keys.len(), 10, "{keys:?}");
    assert_eq!(out["collection"], server.root().display().to_string());
    assert_eq!(out["dry_run"], false);
    assert_eq!(out["added"], json!(["people/listUsers.yml"]));
    assert_eq!(out["orphaned"], json!(["legacy/removeLegacy.yml"]));
    assert_eq!(out["changed"][0]["path"], "people/getUser.yml");
    assert!(out["changed"][0]["fields"].is_array(), "{out}");
    assert_eq!(out["pruned"], json!([]));
    assert_eq!(out["collections"], json!([]));
    assert!(server.root().join("people/listUsers.yml").is_file());
    let kept = fs::read_to_string(server.root().join("people/getUser.yml")).unwrap();
    assert!(
        kept.contains("res.status"),
        "the local test survives: {kept}"
    );
    assert!(server.root().join("legacy/removeLegacy.yml").is_file());
    let list = server.ok("list_items", json!({"path": "people"})).await;
    assert!(list.to_string().contains("listUsers"), "{list}");
}

#[tokio::test]
async fn a_dry_run_writes_nothing_and_matches_the_applied_summary() {
    let server = from_v1().await;
    let before = all(&server);
    let dry = server
        .ok(
            "import_collection",
            json!({"text": V2, "update": true, "dry_run": true}),
        )
        .await;
    assert_eq!(dry["dry_run"], true);
    assert_eq!(all(&server), before);
    let mut applied = server
        .ok("import_collection", json!({"text": V2, "update": true}))
        .await;
    assert_ne!(all(&server), before);
    applied["dry_run"] = json!(true);
    assert_eq!(dry, applied);
}

#[tokio::test]
async fn prune_removes_an_unedited_orphan_and_keeps_one_with_a_test() {
    let server = from_v1().await;
    let out = server
        .ok(
            "import_collection",
            json!({"text": V2, "update": true, "prune": true}),
        )
        .await;
    assert_eq!(out["pruned"], json!(["legacy/removeLegacy.yml"]));
    assert_eq!(out["pruned_destination"], "trash");
    assert!(!server.root().join("legacy/removeLegacy.yml").exists());

    let edited = from_v1().await;
    add_test(&edited, "legacy/removeLegacy.yml");
    let out = edited
        .ok(
            "import_collection",
            json!({"text": V2, "update": true, "prune": true}),
        )
        .await;
    assert_eq!(out["pruned"], json!([]));
    assert!(out.get("pruned_destination").is_none_or(Value::is_null));
    assert_eq!(out["orphaned"], json!(["legacy/removeLegacy.yml"]));
    assert!(edited.root().join("legacy/removeLegacy.yml").is_file());
}

/// A served v1 import whose trash is `trash`.
async fn from_v1_with_trash(trash: Arc<dyn Trash>) -> TestServer {
    let source = from_v1().await;
    let files = all(&source);
    TestServer::start_with(
        &files
            .iter()
            .map(|(p, t)| (p.as_str(), t.as_str()))
            .collect::<Vec<_>>(),
        move |o| o.trash(trash),
    )
    .await
}

#[tokio::test]
async fn a_prune_files_the_orphan_in_the_configured_trash() {
    let trash = Arc::new(FakeTrash::new(true));
    let server = from_v1_with_trash(trash.clone()).await;
    server
        .ok(
            "import_collection",
            json!({"text": V2, "update": true, "prune": true}),
        )
        .await;
    assert_eq!(trash.taken(), 1);
}

#[tokio::test]
async fn a_dry_run_prune_claims_no_move() {
    let trash = Arc::new(FakeTrash::new(true));
    let server = from_v1_with_trash(trash.clone()).await;
    let out = server
        .ok(
            "import_collection",
            json!({"text": V2, "update": true, "prune": true, "dry_run": true}),
        )
        .await;
    assert_eq!(out["pruned"], json!(["legacy/removeLegacy.yml"]));
    assert!(out.get("pruned_destination").is_none_or(Value::is_null));
    assert_eq!(trash.taken(), 0);
    assert!(server.root().join("legacy/removeLegacy.yml").is_file());
}

#[tokio::test]
async fn a_prune_without_a_usable_trash_is_refused_and_changes_nothing() {
    let server = from_v1_with_trash(Arc::new(FakeTrash::new(false))).await;
    let before = all(&server);
    let err = server
        .err(
            "import_collection",
            json!({"text": V2, "update": true, "prune": true}),
        )
        .await;
    assert!(err.contains("trash"), "{err}");
    assert_eq!(all(&server), before);
}

#[tokio::test]
async fn a_collection_without_a_baseline_is_reported_and_left_alone() {
    let server = shop().await;
    let before = all(&server);
    let out = server
        .ok("import_collection", json!({"text": V2, "update": true}))
        .await;
    for k in ["added", "changed", "kept", "edited", "orphaned", "pruned"] {
        assert_eq!(out[k], json!([]), "{k}");
    }
    let notes = out["notes"].to_string();
    assert!(notes.contains("no update baseline"), "{notes}");
    assert!(notes.contains("Nothing was changed"), "{notes}");
    // Like the CLI, every update note carries `file`, null when it concerns no file.
    assert_eq!(out["notes"][0]["file"], Value::Null);
    assert!(out["notes"][0].as_object().unwrap().contains_key("file"));
    assert_eq!(all(&server), before);
}

#[tokio::test]
async fn update_option_combinations_are_validated() {
    let server = from_v1().await;
    let before = all(&server);
    let cases: [(Value, &str); 5] = [
        (json!({"text": V2, "prune": true}), "require `update`"),
        (json!({"text": V2, "dry_run": true}), "require `update`"),
        (
            json!({"text": POSTMAN, "update": true}),
            "update needs an OpenAPI description",
        ),
        (json!({"text": V2, "update": true, "name": "x"}), "HAR"),
        (
            json!({"text": r#"{"log":{"version":"1.2","entries":[]}}"#, "update": true}),
            "update needs an OpenAPI description",
        ),
    ];
    for (args, expect) in cases {
        let message = server.err("import_collection", args.clone()).await;
        assert!(message.contains(expect), "{args}: {message}");
    }
    assert_eq!(all(&server), before);
}

#[tokio::test]
async fn an_update_from_a_non_local_url_still_needs_approval() {
    let server = from_v1().await;
    let before = all(&server);
    let message = server
        .err(
            "import_collection",
            json!({"url": "http://specs.example.invalid/api.json", "update": true}),
        )
        .await;
    assert!(message.contains("Approval requested"), "{message}");
    assert_eq!(server.inbox().pending().unwrap().len(), 1);
    assert_eq!(all(&server), before);
}

#[tokio::test]
async fn an_edited_path_is_followed_through_the_recorded_file() {
    let server = from_v1().await;
    let path = server.root().join("people/getUser.yml");
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("/users/:id"), "{text}");
    fs::write(&path, text.replace("/users/:id", "/v9/users/:id")).unwrap();
    let out = server
        .ok("import_collection", json!({"text": V2, "update": true}))
        .await;
    assert_eq!(out["edited"], json!(["people/getUser.yml"]));
    assert_eq!(out["added"], json!(["people/listUsers.yml"]));
    assert!(!server.root().join("people/getUser 1.yml").exists());
}

#[tokio::test]
async fn a_changed_method_is_reported_as_edited_naming_the_method() {
    let server = from_v1().await;
    let path = server.root().join("people/getUser.yml");
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("method: GET"), "{text}");
    fs::write(&path, text.replace("method: GET", "method: POST")).unwrap();
    let out = server
        .ok("import_collection", json!({"text": V2, "update": true}))
        .await;
    assert_eq!(out["edited"], json!(["people/getUser.yml"]));
    assert_eq!(
        out["edited_fields"],
        json!([{"path": "people/getUser.yml", "fields": ["method"]}])
    );
    assert_eq!(out["changed"][0]["path"], "people/getUser.yml");
    assert!(!server.root().join("people/getUser 1.yml").exists());
}

#[tokio::test]
async fn a_second_update_changes_nothing() {
    let server = from_v1().await;
    server
        .ok("import_collection", json!({"text": V2, "update": true}))
        .await;
    let after = all(&server);
    let again = server
        .ok("import_collection", json!({"text": V2, "update": true}))
        .await;
    assert_eq!(again["added"], json!([]));
    assert_eq!(again["changed"], json!([]));
    assert_eq!(all(&server), after);
}

#[tokio::test]
async fn the_description_documents_update() {
    let server = shop().await;
    let tools = server.tools().await;
    let tool = tools
        .iter()
        .find(|t| t.name == "import_collection")
        .unwrap();
    let description = tool.description.as_deref().unwrap();
    for word in ["update", "prune", "dry_run"] {
        assert!(description.contains(word), "{word}: {description}");
    }
}

// ---- folder-scoped baselines ----

fn pets_spec(title: &str, extra: &str) -> String {
    format!(
        r#"{{"openapi":"3.0.0","info":{{"title":"{title}","version":"1"}},
"paths":{{"/pets":{{"get":{{"summary":"List pets","tags":["pets"]}}}}{extra}}}}}"#
    )
}

fn folder_files(server: &TestServer, folder: &str) -> Vec<(String, String)> {
    let prefix = format!("{folder}/");
    all(server)
        .into_iter()
        .filter(|(rel, _)| rel.starts_with(&prefix))
        .collect()
}

/// Two specs with the same operation key imported into two subfolders.
async fn two_imports() -> TestServer {
    let server = shop().await;
    for (title, into) in [("Alpha", "specs/a"), ("Beta", "specs/b")] {
        server
            .ok(
                "import_collection",
                json!({"text": pets_spec(title, ""), "into": into}),
            )
            .await;
    }
    server
}

#[tokio::test]
async fn an_import_records_a_baseline_entry_per_placed_folder() {
    let server = two_imports().await;
    let root = fs::read_to_string(server.root().join("opencollection.yml")).unwrap();
    for folder in ["specs/a/Alpha", "specs/b/Beta"] {
        assert!(root.contains(folder), "{folder} in {root}");
    }
    assert!(
        root.contains("specs/a/Alpha/pets/List pets.yml"),
        "the recorded file path carries the folder: {root}"
    );
    // The collection root itself has no entry of its own.
    let out = server
        .ok("import_collection", json!({"text": V2, "update": true}))
        .await;
    assert!(out["notes"].to_string().contains("no update baseline"));
}

#[tokio::test]
async fn an_update_with_into_changes_only_that_folder() {
    let server = two_imports().await;
    let v2 = pets_spec(
        "Alpha",
        r#","/cats":{"get":{"summary":"List cats","tags":["cats"]}}"#,
    );
    let b_before = folder_files(&server, "specs/b/Beta");
    let out = server
        .ok(
            "import_collection",
            json!({"text": v2, "update": true, "into": "specs/a/Alpha"}),
        )
        .await;
    assert_eq!(out["added"], json!(["specs/a/Alpha/cats/List cats.yml"]));
    assert_eq!(folder_files(&server, "specs/b/Beta"), b_before);
    // The other folder updates on its own entry.
    let v2b = pets_spec(
        "Beta",
        r#","/dogs":{"get":{"summary":"List dogs","tags":["dogs"]}}"#,
    );
    let a_before = folder_files(&server, "specs/a/Alpha");
    let out = server
        .ok(
            "import_collection",
            json!({"text": v2b, "update": true, "into": "specs/b/Beta"}),
        )
        .await;
    assert_eq!(out["added"], json!(["specs/b/Beta/dogs/List dogs.yml"]));
    assert_eq!(folder_files(&server, "specs/a/Alpha"), a_before);
}

#[tokio::test]
async fn an_update_without_into_has_no_root_baseline_after_folder_imports() {
    let server = two_imports().await;
    let before = all(&server);
    let out = server
        .ok("import_collection", json!({"text": V2, "update": true}))
        .await;
    assert!(
        out["notes"].to_string().contains("no update baseline"),
        "{out}"
    );
    assert_eq!(all(&server), before);
}

#[tokio::test]
async fn an_update_into_a_missing_folder_writes_nothing() {
    let server = two_imports().await;
    let before = all(&server);
    for into in ["specs/zzz", "specs/a/Alpha/pets/List pets.yml"] {
        let out = server
            .ok(
                "import_collection",
                json!({"text": V2, "update": true, "into": into}),
            )
            .await;
        assert!(
            out["notes"].to_string().contains("no update baseline"),
            "{into}: {out}"
        );
        assert_eq!(out["added"], json!([]));
    }
    assert_eq!(all(&server), before, "no folder was created");
}

#[tokio::test]
async fn the_description_documents_into_for_update() {
    let server = shop().await;
    let tools = server.tools().await;
    let tool = tools
        .iter()
        .find(|t| t.name == "import_collection")
        .unwrap();
    let description = tool.description.as_deref().unwrap();
    assert!(
        !description.contains("`into` is not accepted"),
        "{description}"
    );
    assert!(description.contains("`into`") && description.contains("baseline"));
    let schema = serde_json::to_string(&tool.input_schema).unwrap();
    assert!(schema.contains("exact folder"), "{schema}");
}

/// A stand-in proxy on 127.0.0.1 that answers every request with `SPEC` and counts them.
fn spec_proxy() -> (u16, Arc<std::sync::atomic::AtomicUsize>) {
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let hits = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&hits);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                match stream.read(&mut byte) {
                    Ok(1) => head.push(byte[0]),
                    _ => break,
                }
            }
            count.fetch_add(1, Ordering::SeqCst);
            let reply = format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{SPEC}",
                SPEC.len()
            );
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    (port, hits)
}

#[tokio::test]
async fn a_url_import_goes_through_the_configured_proxy() {
    use apinata_core::{GlobalProxy, NetworkSettings, ResolvedProxy};
    let (port, hits) = spec_proxy();
    let proxy = ResolvedProxy::new("http", "127.0.0.1", &port.to_string()).expect("proxy");
    let server = TestServer::start_with(
        &[("opencollection.yml", ROOT), ("api/folder.yml", EXISTING)],
        |options| {
            options.network(NetworkSettings::new().proxy(GlobalProxy::Manual {
                proxy,
                bypass: String::new(),
            }))
        },
    )
    .await;
    // Nothing listens on port 9, so only the proxy can have served the document.
    server
        .ok(
            "import_collection",
            json!({"url": "http://127.0.0.1:9/petstore.json"}),
        )
        .await;
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(!imported(&server).is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn an_update_into_a_symlink_writes_nothing() {
    let server = two_imports().await;
    std::os::unix::fs::symlink(
        server.root().join("specs/a/Alpha"),
        server.root().join("link"),
    )
    .unwrap();
    let before = all(&server);
    let out = server
        .ok(
            "import_collection",
            json!({"text": V2, "update": true, "into": "link"}),
        )
        .await;
    assert!(
        out["notes"].to_string().contains("no update baseline"),
        "{out}"
    );
    assert_eq!(all(&server), before);
}

#[tokio::test]
async fn a_pruned_request_comes_back_through_restore_item() {
    let trash = Arc::new(FakeTrash::new(true));
    let server = from_v1_with_trash(trash.clone()).await;
    let out = server
        .ok(
            "import_collection",
            json!({"text": V2, "update": true, "prune": true}),
        )
        .await;
    assert_eq!(out["pruned"], json!(["legacy/removeLegacy.yml"]));
    let back = server.ok("restore_item", json!({})).await;
    assert_eq!(
        back,
        json!({"restored": "legacy/removeLegacy.yml", "warnings": []})
    );
    assert!(server.root().join("legacy/removeLegacy.yml").is_file());
}

#[tokio::test]
async fn restore_pruned_brings_back_what_the_last_prune_removed() {
    let trash = Arc::new(FakeTrash::new(true));
    let server = from_v1_with_trash(trash.clone()).await;
    server
        .ok(
            "import_collection",
            json!({"text": V2, "update": true, "prune": true}),
        )
        .await;
    assert_eq!(
        server.ok("restore_pruned", json!({})).await,
        json!({"restored": ["legacy/removeLegacy.yml"], "warnings": []})
    );
    assert!(server.root().join("legacy/removeLegacy.yml").is_file());
    assert_eq!(
        server.err("restore_pruned", json!({})).await,
        "nothing deleted from this collection is left to restore"
    );
}

#[tokio::test]
async fn restore_pruned_refuses_when_the_newest_delete_was_not_a_prune() {
    let trash = Arc::new(FakeTrash::new(true));
    let server = from_v1_with_trash(trash.clone()).await;
    server
        .ok(
            "import_collection",
            json!({"text": V2, "update": true, "prune": true}),
        )
        .await;
    server
        .ok("delete_item", json!({"path": "people/getUser.yml"}))
        .await;
    let err = server.err("restore_pruned", json!({})).await;
    assert!(
        err.starts_with("the newest delete was not an import prune"),
        "{err}"
    );
    assert!(!server.root().join("legacy/removeLegacy.yml").exists());
    assert!(!server.root().join("people/getUser.yml").exists());
}

/// A prune with a later single delete on top of it, and the batch id that delete's refusal names.
async fn pruned_under_a_delete() -> (TestServer, String) {
    let server = from_v1_with_trash(Arc::new(FakeTrash::new(true))).await;
    server
        .ok(
            "import_collection",
            json!({"text": V2, "update": true, "prune": true}),
        )
        .await;
    server
        .ok("delete_item", json!({"path": "people/getUser.yml"}))
        .await;
    let err = server.err("restore_pruned", json!({})).await;
    let (_, rest) = err.split_once("newest first: ").expect(&err);
    let (batch, _) = rest.split_once(';').expect(&err);
    let batch = batch.to_owned();
    (server, batch)
}

#[tokio::test]
async fn restore_pruned_takes_a_batch_past_a_later_delete() {
    let (server, batch) = pruned_under_a_delete().await;
    assert_eq!(
        server.ok("restore_pruned", json!({"batch": batch})).await,
        json!({"restored": ["legacy/removeLegacy.yml"], "warnings": []})
    );
    assert!(server.root().join("legacy/removeLegacy.yml").is_file());
    assert!(!server.root().join("people/getUser.yml").exists());
}

#[tokio::test]
async fn restore_pruned_refuses_an_unknown_batch_and_lists_the_recorded_ones() {
    let (server, batch) = pruned_under_a_delete().await;
    assert_eq!(
        server.err("restore_pruned", json!({"batch": "nope"})).await,
        format!("no prune nope in the trash record; recorded prunes, newest first: {batch}")
    );
    assert!(!server.root().join("legacy/removeLegacy.yml").exists());
}

#[tokio::test]
async fn restore_pruned_works_with_no_arguments_at_all() {
    let server = from_v1_with_trash(Arc::new(FakeTrash::new(true))).await;
    server
        .ok(
            "import_collection",
            json!({"text": V2, "update": true, "prune": true}),
        )
        .await;
    let result = server.call_bare("restore_pruned").await;
    assert_ne!(result.is_error, Some(true), "{result:?}");
    assert!(server.root().join("legacy/removeLegacy.yml").is_file());
}

#[tokio::test]
async fn restore_pruned_offers_the_batch_as_optional() {
    let server = from_v1_with_trash(Arc::new(FakeTrash::new(true))).await;
    let tools = server.tools().await;
    let tool = tools.iter().find(|t| t.name == "restore_pruned").unwrap();
    let schema = tool.input_schema.as_ref();
    assert!(schema["properties"].get("batch").is_some(), "{schema:?}");
    assert!(schema.get("required").is_none(), "{schema:?}");
}

#[tokio::test]
async fn restore_pruned_and_scrub_are_listed_as_local_writes() {
    let server = from_v1().await;
    let tools = server.tools().await;
    let hints = |name: &str| {
        let tool = tools.iter().find(|t| t.name == name).expect("listed");
        assert!(tool.output_schema.is_some(), "{name}");
        let a = tool.annotations.as_ref().expect("annotations");
        (
            a.read_only_hint,
            a.destructive_hint,
            a.idempotent_hint,
            a.open_world_hint,
        )
    };
    assert_eq!(
        hints("restore_pruned"),
        (None, Some(false), Some(false), Some(false))
    );
    assert_eq!(
        hints("scrub_example_userinfo"),
        (None, Some(false), Some(true), Some(false))
    );
}
