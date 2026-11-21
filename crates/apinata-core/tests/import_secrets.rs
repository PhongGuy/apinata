//! Imported credentials never land in collection files: each literal becomes a secret variable
//! whose value goes to the secret store, and the report names the variable but not the value.

use apinata_core::import::{
    CollectionOptions, ImportReport, NoteKind, add_request, parse_collections, parse_request,
    place_items, store_request_secrets, write_collection,
};
use apinata_core::{
    Collection, EnvVariable, ItemPath, MemoryStore, SecretError, SecretKey, SecretStore, VarValue,
};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

const V21: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";

/// Every file under `root`, concatenated.
pub(crate) fn all_text(root: &Path) -> String {
    let mut out = String::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("read dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push_str(&fs::read_to_string(&path).unwrap_or_default());
            }
        }
    }
    out
}

/// The value stored for `var` in any environment of `c`.
pub(crate) fn stored(c: &Collection, store: &MemoryStore, var: &str) -> Option<String> {
    c.environments().iter().find_map(|env| {
        store
            .get(&c.secret_key(env.name(), var))
            .expect("get")
            .filter(|_| {
                env.variables()
                    .iter()
                    .any(|v| v.name == var && v.secret && v.value == VarValue::Text(String::new()))
            })
    })
}

/// Import `input` as a new collection; returns the collection, the report and the store.
pub(crate) fn import(input: &str) -> (TempDir, Collection, ImportReport, MemoryStore) {
    let parent = TempDir::new().expect("tempdir");
    let bundles =
        parse_collections(None, input.as_bytes(), &CollectionOptions::default()).expect("parses");
    let store = MemoryStore::new();
    let (c, report) = write_collection(
        parent.path(),
        &bundles[0].collection,
        &bundles[0].environments,
        &store,
    )
    .expect("writes");
    (parent, c, report, store)
}

/// Each `(variable, value)`: no file holds the value, the store does, and a note names the
/// variable without its value.
pub(crate) fn assert_protected(
    c: &Collection,
    report: &ImportReport,
    store: &MemoryStore,
    pairs: &[(&str, &str)],
) {
    let text = all_text(c.root());
    for (var, value) in pairs {
        assert!(
            !text.contains(value),
            "`{value}` written to a file:\n{text}"
        );
        assert!(
            text.contains(&format!("{{{{{var}}}}}")),
            "no reference to `{var}`:\n{text}"
        );
        assert_eq!(stored(c, store, var).as_deref(), Some(*value), "{var}");
        assert!(
            report
                .notes
                .iter()
                .any(|n| n.detail.contains(&format!("`{var}`"))),
            "no note names `{var}`: {:?}",
            report.notes
        );
    }
    for note in &report.notes {
        for (_, value) in pairs {
            assert!(!note.detail.contains(value), "{note:?}");
        }
    }
}

pub(crate) fn postman(items: &str) -> String {
    format!(r#"{{"info":{{"name":"Shop","schema":"{V21}"}},"item":[{items}]}}"#)
}

#[test]
fn a_postman_basic_password_is_kept_out_of_the_file() {
    let input = postman(
        r#"{"name":"Create user","request":{"method":"POST","url":"https://x.test/u",
        "auth":{"type":"basic","basic":[{"key":"username","value":"bob"},{"key":"password","value":"pw-s3cret"}]}}}"#,
    );
    let (_d, c, report, store) = import(&input);
    assert_protected(
        &c,
        &report,
        &store,
        &[("create_user_password", "pw-s3cret")],
    );
    assert!(all_text(c.root()).contains("bob"), "the user name stays");
}

#[test]
fn postman_tokens_keys_and_oauth2_secrets_are_kept_out_of_the_files() {
    let input = format!(
        r#"{{"info":{{"name":"Shop","schema":"{V21}"}},
        "auth":{{"type":"bearer","bearer":[{{"key":"token","value":"root-tok-1"}}]}},
        "item":[
          {{"name":"Keys","auth":{{"type":"apikey","apikey":[{{"key":"key","value":"X-Key"}},{{"key":"value","value":"key-val-2"}}]}},
            "item":[{{"name":"Inner","request":{{"method":"GET","url":"https://x.test/i"}}}}]}},
          {{"name":"Login","request":{{"method":"GET","url":"https://x.test/l",
            "auth":{{"type":"oauth2","oauth2":[{{"key":"grant_type","value":"password_credentials"}},
              {{"key":"clientSecret","value":"client-sec-3"}},{{"key":"username","value":"ann"}},
              {{"key":"password","value":"oauth-pw-4"}}]}}}}}}
        ]}}"#
    );
    let (_d, c, report, store) = import(&input);
    assert_protected(
        &c,
        &report,
        &store,
        &[
            ("shop_token", "root-tok-1"),
            ("keys_api_key", "key-val-2"),
            ("login_client_secret", "client-sec-3"),
            ("login_password", "oauth-pw-4"),
        ],
    );
}

#[test]
fn credential_headers_and_cookies_are_kept_out_of_the_file() {
    let input = postman(
        r#"{"name":"Me","request":{"method":"GET","url":"https://x.test/me",
        "header":[{"key":"Authorization","value":"Bearer hdr-tok-5"},{"key":"Cookie","value":"sid=cookie-6"},
          {"key":"X-Api-Key","value":"api-key-7"},{"key":"Accept","value":"text/plain"}]}}"#,
    );
    let (_d, c, report, store) = import(&input);
    assert_protected(
        &c,
        &report,
        &store,
        &[
            ("me_authorization", "Bearer hdr-tok-5"),
            ("me_cookie", "sid=cookie-6"),
            ("me_x_api_key", "api-key-7"),
        ],
    );
    assert!(
        all_text(c.root()).contains("text/plain"),
        "other headers stay"
    );
}

#[test]
fn values_already_written_as_references_stay_and_add_no_environment() {
    let input = postman(
        r#"{"name":"R","request":{"method":"GET","url":"https://x.test/r",
        "auth":{"type":"basic","basic":[{"key":"username","value":"u"},{"key":"password","value":"{{pw}}"}]},
        "header":[{"key":"Authorization","value":"Bearer {{token}}"}]}}"#,
    );
    let (_d, c, report, _store) = import(&input);
    let text = all_text(c.root());
    assert!(
        text.contains("{{pw}}") && text.contains("Bearer {{token}}"),
        "{text}"
    );
    assert!(c.environments().is_empty());
    assert!(
        report
            .notes
            .iter()
            .all(|n| n.kind != NoteKind::Converted || !n.detail.contains("secret variable"))
    );
}

#[test]
fn an_insomnia_bearer_token_is_kept_out_of_the_file() {
    let input = r#"{"_type":"export","__export_format":4,"resources":[
      {"_id":"wrk_1","_type":"workspace","parentId":null,"name":"Pets"},
      {"_id":"req_1","_type":"request","parentId":"wrk_1","name":"Get pet","method":"GET",
       "url":"https://x.test/pets/1","headers":[],"body":{},
       "authentication":{"type":"bearer","token":"pet-token-8"}}]}"#;
    let (_d, c, report, store) = import(input);
    assert_protected(&c, &report, &store, &[("get_pet_token", "pet-token-8")]);
}

#[test]
fn hoppscotch_basic_and_bearer_secrets_are_kept_out_of_the_files() {
    let input = r#"{"v":2,"name":"Hopp","folders":[
      {"v":2,"name":"Orders","folders":[],"requests":[
        {"v":"1","name":"Get order","method":"GET","endpoint":"https://x.test/o/7","params":[],"headers":[],
         "preRequestScript":"","testScript":"",
         "auth":{"authType":"bearer","authActive":true,"token":"hopp-tok-9"},"body":{"contentType":null,"body":null}}],
       "auth":{"authType":"inherit","authActive":true},"headers":[]}],
     "requests":[
        {"v":"1","name":"Create order","method":"POST","endpoint":"https://x.test/o","params":[],"headers":[],
         "preRequestScript":"","testScript":"",
         "auth":{"authType":"basic","authActive":true,"username":"u","password":"hopp-pw-10"},
         "body":{"contentType":null,"body":null}}],
     "auth":{"authType":"none","authActive":true},"headers":[]}"#;
    let (_d, c, report, store) = import(input);
    assert_protected(
        &c,
        &report,
        &store,
        &[
            ("get_order_token", "hopp-tok-9"),
            ("create_order_password", "hopp-pw-10"),
        ],
    );
}

#[test]
fn a_secret_goes_to_every_imported_environment() {
    let parent = TempDir::new().expect("tempdir");
    let input = postman(
        r#"{"name":"R","request":{"method":"GET","url":"https://x.test/r",
        "auth":{"type":"bearer","bearer":[{"key":"token","value":"tok-11"}]}}}"#,
    );
    let bundles =
        parse_collections(None, input.as_bytes(), &CollectionOptions::default()).expect("parses");
    let envs = apinata_core::import::parse_environments(
        None,
        br#"{"name":"dev","values":[{"key":"r_token","value":"plain","enabled":true}]}"#,
    )
    .expect("env");
    let mut both = envs.clone();
    let mut prod = envs[0].clone();
    prod.name = "prod".to_owned();
    both.push(prod);
    let store = MemoryStore::new();
    let (c, report) =
        write_collection(parent.path(), &bundles[0].collection, &both, &store).expect("writes");
    // `r_token` is taken by a plain variable, so the secret gets the next free name.
    assert_protected(&c, &report, &store, &[("r_token_2", "tok-11")]);
    for env in c.environments() {
        let key = c.secret_key(env.name(), "r_token_2");
        assert_eq!(store.get(&key).expect("get").as_deref(), Some("tok-11"));
        assert!(
            env.variables()
                .iter()
                .any(|v: &EnvVariable| v.name == "r_token" && !v.secret)
        );
    }
}

struct Refusing;

impl SecretStore for Refusing {
    fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
        Ok(None)
    }
    fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
        Err(SecretError::Unavailable {
            message: "locked".to_owned(),
        })
    }
    fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
        Ok(())
    }
}

#[test]
fn a_refusing_store_leaves_the_secret_to_enter_and_still_writes_no_value() {
    let parent = TempDir::new().expect("tempdir");
    let input = postman(
        r#"{"name":"R","request":{"method":"GET","url":"https://x.test/r",
        "auth":{"type":"bearer","bearer":[{"key":"token","value":"tok-12"}]}}}"#,
    );
    let bundles =
        parse_collections(None, input.as_bytes(), &CollectionOptions::default()).expect("parses");
    let (c, report) =
        write_collection(parent.path(), &bundles[0].collection, &[], &Refusing).expect("writes");
    assert!(!all_text(c.root()).contains("tok-12"));
    assert!(
        report
            .secrets_to_enter
            .iter()
            .any(|(_, var)| var == "r_token"),
        "{:?}",
        report.secrets_to_enter
    );
}

#[test]
fn items_placed_in_a_folder_keep_their_secrets_in_the_only_environment() {
    let dir = TempDir::new().expect("tempdir");
    let store = MemoryStore::new();
    let mut c = Collection::create(&dir.path().join("c"), "c").expect("create");
    c.create_environment("dev", &store).expect("env");
    let input = postman(
        r#"{"name":"R","request":{"method":"GET","url":"https://x.test/r",
        "auth":{"type":"basic","basic":[{"key":"username","value":"u"},{"key":"password","value":"placed-pw-13"}]}}}"#,
    );
    let bundles =
        parse_collections(None, input.as_bytes(), &CollectionOptions::default()).expect("parses");
    let placed = place_items(&mut c, &ItemPath::root(), &bundles[0], &store).expect("places");
    assert_protected(
        &c,
        &placed.report,
        &store,
        &[("r_password", "placed-pw-13")],
    );
}

#[test]
fn curl_credentials_are_kept_out_of_the_request_file() {
    let dir = TempDir::new().expect("tempdir");
    let store = MemoryStore::new();
    let mut c = Collection::create(&dir.path().join("c"), "c").expect("create");
    c.create_environment("dev", &store).expect("env");
    let command = "curl https://api.example.com/x -u alice:curl-pw-14 \
        -H 'X-Api-Key: curl-key-15' -b 'sid=curl-cookie-16'";
    let mut parsed = parse_request(None, command).expect("parses");
    add_request(&mut c, &ItemPath::root(), "X", &parsed.request).expect("adds");
    store_request_secrets(&mut c, &parsed.request, None, &store, &mut parsed.notes);
    let mut report = ImportReport::default();
    report.notes = parsed.notes;
    assert_protected(
        &c,
        &report,
        &store,
        &[
            ("api_example_com_password", "curl-pw-14"),
            ("api_example_com_x_api_key", "curl-key-15"),
            ("api_example_com_cookie", "sid=curl-cookie-16"),
        ],
    );
    assert!(all_text(c.root()).contains("alice"), "the user name stays");
}

#[test]
fn a_curl_bearer_header_and_oauth2_bearer_are_kept_out_of_the_request_file() {
    let dir = TempDir::new().expect("tempdir");
    let store = MemoryStore::new();
    let mut c = Collection::create(&dir.path().join("c"), "c").expect("create");
    c.create_environment("dev", &store).expect("env");
    let command = "curl https://h.test/ -H 'Authorization: Bearer curl-hdr-17' \
        --oauth2-bearer curl-tok-18";
    let mut parsed = parse_request(None, command).expect("parses");
    add_request(&mut c, &ItemPath::root(), "X", &parsed.request).expect("adds");
    store_request_secrets(&mut c, &parsed.request, None, &store, &mut parsed.notes);
    let text = all_text(c.root());
    for value in ["curl-hdr-17", "curl-tok-18"] {
        assert!(!text.contains(value), "{text}");
    }
    assert_eq!(
        stored(&c, &store, "h_test_token").as_deref(),
        Some("curl-tok-18")
    );
}
