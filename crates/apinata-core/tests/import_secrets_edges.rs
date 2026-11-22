//! Imported credentials, continued: a stored secret is never overwritten with another value,
//! partial references, the update path, the default environment, and the URL and form fields.

use crate::import_secrets::{all_text, assert_protected, import, postman, stored};
use apinata_core::import::{
    CollectionFormat, CollectionOptions, UpdateOptions, add_request, apply_update,
    claim_secret_names, parse_collections, parse_request, place_items, plan_update,
    store_request_secrets, write_collection,
};
use apinata_core::{
    Collection, ContextOptions, EnvironmentChoice, ItemPath, MemoryStore, SecretStore, VarContext,
    resolve,
};
use std::sync::Arc;
use tempfile::TempDir;

/// A collection `c` with the environment `dev`.
fn with_dev(store: &MemoryStore) -> (TempDir, Collection) {
    let dir = TempDir::new().expect("tempdir");
    let mut c = Collection::create(&dir.path().join("c"), "c").expect("create");
    c.create_environment("dev", store).expect("env");
    (dir, c)
}

/// Import `command` as request `name`, the way every shell does.
fn import_curl(c: &mut Collection, store: &MemoryStore, name: &str, command: &str) {
    let mut parsed = parse_request(None, command).expect("parses");
    claim_secret_names(c, &mut parsed.request, None, store);
    add_request(c, &ItemPath::root(), name, &parsed.request).expect("adds");
    store_request_secrets(c, &parsed.request, None, store, &mut parsed.notes);
}

fn text_of(c: &Collection, item: &str) -> String {
    std::fs::read_to_string(c.root().join(item)).expect("file")
}

#[test]
fn a_second_import_with_another_password_never_overwrites_the_first() {
    let store = MemoryStore::new();
    let (_d, mut c) = with_dev(&store);
    // Two ports share the host slug, so both start from the same name.
    import_curl(
        &mut c,
        &store,
        "A",
        "curl -u a:first-pw http://localhost:8080/x",
    );
    import_curl(
        &mut c,
        &store,
        "B",
        "curl -u a:second-pw http://localhost:9090/x",
    );
    import_curl(
        &mut c,
        &store,
        "C",
        "curl -u a:first-pw http://localhost:8080/y",
    );
    assert!(text_of(&c, "A.yml").contains("{{localhost_password}}"));
    assert!(text_of(&c, "B.yml").contains("{{localhost_password_2}}"));
    assert!(
        text_of(&c, "C.yml").contains("{{localhost_password}}"),
        "the same value reuses its variable"
    );
    assert_eq!(
        stored(&c, &store, "localhost_password").as_deref(),
        Some("first-pw")
    );
    assert_eq!(
        stored(&c, &store, "localhost_password_2").as_deref(),
        Some("second-pw")
    );
}

#[test]
fn storing_never_replaces_a_different_stored_value() {
    let store = MemoryStore::new();
    let (_d, mut c) = with_dev(&store);
    import_curl(&mut c, &store, "A", "curl -u a:first-pw http://h.test/x");
    // A caller that skips claiming still cannot overwrite the first request's secret.
    let mut parsed = parse_request(None, "curl -u a:other-pw http://h.test/x").expect("parses");
    add_request(&mut c, &ItemPath::root(), "B", &parsed.request).expect("adds");
    store_request_secrets(&mut c, &parsed.request, None, &store, &mut parsed.notes);
    assert_eq!(
        stored(&c, &store, "h_test_password").as_deref(),
        Some("first-pw")
    );
    assert!(
        parsed
            .notes
            .iter()
            .any(|n| n.detail.contains("already holds a different value")),
        "{:?}",
        parsed.notes
    );
}

#[test]
fn a_credential_that_is_not_exactly_one_reference_moves_whole_and_still_resolves() {
    let input = postman(
        r#"{"name":"Me","request":{"method":"GET","url":"https://x.test/me",
        "header":[{"key":"Cookie","value":"session=part-lit-20; csrf={{c}}"},
          {"key":"Authorization","value":"Bearer {{token}}"}]}}"#,
    );
    let (_d, c, report, store) = import(&input);
    assert_protected(
        &c,
        &report,
        &store,
        &[("me_cookie", "session=part-lit-20; csrf={{c}}")],
    );
    assert!(
        all_text(c.root()).contains("Bearer {{token}}"),
        "a scheme and a reference hold nothing secret"
    );
    let options = ContextOptions::new()
        .env("Environment")
        .secrets(Arc::new(store) as Arc<dyn SecretStore>)
        .env_var("c", "csrf-value")
        .env_var("token", "t");
    let ctx = VarContext::new(&c, options).expect("context");
    let file = c
        .request(&ItemPath::new("Me.yml").expect("path"))
        .expect("request");
    let resolved = resolve(file, &ctx).expect("resolves");
    let cookie = resolved
        .headers()
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("cookie"))
        .map(|(_, v)| v.as_str());
    assert_eq!(cookie, Some("session=part-lit-20; csrf=csrf-value"));
}

const SPEC_V1: &str = r#"{"openapi":"3.0.3","info":{"title":"Keys","version":"1"},
  "servers":[{"url":"https://x.test"}],
  "paths":{"/a":{"get":{"operationId":"getA","parameters":[
    {"name":"X-Api-Key","in":"header","schema":{"type":"string"},"example":"spec-key-30"}],
    "responses":{"200":{"description":"ok"}}}}}}"#;

const SPEC_V2: &str = r#"{"openapi":"3.0.3","info":{"title":"Keys","version":"2"},
  "servers":[{"url":"https://x.test"}],
  "paths":{"/a":{"get":{"operationId":"getA","parameters":[
    {"name":"X-Api-Key","in":"header","schema":{"type":"string"},"example":"spec-key-30"}],
    "responses":{"200":{"description":"ok"}}}},
   "/b":{"get":{"operationId":"getB","parameters":[
    {"name":"X-Api-Key","in":"header","schema":{"type":"string"},"example":"spec-key-31"}],
    "responses":{"200":{"description":"ok"}}}}}}"#;

fn openapi(text: &str) -> apinata_core::import::ImportBundle {
    parse_collections(
        Some(CollectionFormat::OpenApi),
        text.as_bytes(),
        &CollectionOptions::default(),
    )
    .expect("parses")
    .remove(0)
}

#[test]
fn an_update_keeps_spec_credentials_out_of_files_and_a_second_preview_is_clean() {
    let parent = TempDir::new().expect("tempdir");
    let store = Arc::new(MemoryStore::new());
    let v1 = openapi(SPEC_V1);
    let (mut c, _) = write_collection(
        parent.path(),
        &v1.collection,
        &v1.environments,
        store.as_ref(),
    )
    .expect("writes");
    assert!(!all_text(c.root()).contains("spec-key-30"));
    let mut options = UpdateOptions::default();
    options.secrets = Some(Arc::clone(&store) as Arc<dyn SecretStore>);
    let unchanged = plan_update(&c, &v1, &options).expect("plans");
    assert!(unchanged.changed.is_empty(), "{unchanged:?}");
    let v2 = openapi(SPEC_V2);
    let applied = apply_update(&mut c, &v2, &options, None).expect("applies");
    assert_eq!(applied.added.len(), 1, "{applied:?}");
    let text = all_text(c.root());
    assert!(!text.contains("spec-key-31"), "{text}");
    let c = Collection::open(c.root()).expect("reopens");
    let again = plan_update(&c, &v2, &options).expect("plans");
    assert!(
        again.changed.is_empty() && again.added.is_empty(),
        "{again:?}"
    );
    let names: Vec<String> = c.environments()[0]
        .variables()
        .into_iter()
        .filter(|v| v.secret)
        .map(|v| v.name)
        .collect();
    let added = names
        .iter()
        .find(|n| stored(&c, &store, n).as_deref() == Some("spec-key-31"));
    assert!(added.is_some(), "the new key is stored: {names:?}");
}

#[test]
fn the_environment_holding_imported_secrets_becomes_the_default() {
    let input = postman(
        r#"{"name":"R","request":{"method":"GET","url":"https://x.test/r",
        "auth":{"type":"bearer","bearer":[{"key":"token","value":"tok-40"}]}}}"#,
    );
    let (_d, c, _, _) = import(&input);
    assert_eq!(
        c.choose_environment(None),
        EnvironmentChoice::Preset("Environment".to_owned())
    );
}

#[test]
fn an_inline_oauth1_private_key_is_kept_out_of_the_file() {
    let input = postman(
        r#"{"name":"Signed","request":{"method":"GET","url":"https://x.test/s",
        "auth":{"type":"oauth1","oauth1":[{"key":"consumerKey","value":"ck"},
          {"key":"signatureMethod","value":"RSA-SHA1"},
          {"key":"privateKey","value":"-----BEGIN PRIVATE KEY-----pem-41"}]}}}"#,
    );
    let (_d, c, report, store) = import(&input);
    assert_protected(
        &c,
        &report,
        &store,
        &[("signed_private_key", "-----BEGIN PRIVATE KEY-----pem-41")],
    );
}

#[test]
fn placing_items_without_an_environment_creates_one_and_lists_what_failed() {
    let dir = TempDir::new().expect("tempdir");
    let store = MemoryStore::new();
    let mut c = Collection::create(&dir.path().join("c"), "c").expect("create");
    let input = postman(
        r#"{"name":"R","request":{"method":"GET","url":"https://x.test/r",
        "auth":{"type":"bearer","bearer":[{"key":"token","value":"placed-tok-50"}]}}}"#,
    );
    let bundles =
        parse_collections(None, input.as_bytes(), &CollectionOptions::default()).expect("parses");
    let placed = place_items(&mut c, &ItemPath::root(), &bundles[0], &store).expect("places");
    assert_protected(&c, &placed.report, &store, &[("r_token", "placed-tok-50")]);
    assert_eq!(
        c.choose_environment(None),
        EnvironmentChoice::Preset("Environment".to_owned())
    );
}

#[test]
fn placing_items_with_several_environments_declares_the_secret_in_each() {
    let dir = TempDir::new().expect("tempdir");
    let store = MemoryStore::new();
    let mut c = Collection::create(&dir.path().join("c"), "c").expect("create");
    c.create_environment("dev", &store).expect("dev");
    c.create_environment("prod", &store).expect("prod");
    let input = postman(
        r#"{"name":"R","request":{"method":"GET","url":"https://x.test/r",
        "auth":{"type":"bearer","bearer":[{"key":"token","value":"placed-tok-51"}]}}}"#,
    );
    let bundles =
        parse_collections(None, input.as_bytes(), &CollectionOptions::default()).expect("parses");
    let placed = place_items(&mut c, &ItemPath::root(), &bundles[0], &store).expect("places");
    assert!(placed.report.secrets_to_enter.is_empty());
    for env in ["dev", "prod"] {
        let key = c.secret_key(env, "r_token");
        assert_eq!(
            store.get(&key).expect("get").as_deref(),
            Some("placed-tok-51"),
            "{env}"
        );
    }
}

#[test]
fn url_userinfo_query_keys_and_form_passwords_are_kept_out_of_the_file() {
    let input = postman(
        r#"{"name":"Login","request":{"method":"POST",
        "url":{"raw":"https://ann:url-pw-60@x.test/l?api_key=q-key-61&page=2",
          "protocol":"https","host":["x","test"],"path":["l"],
          "auth":{"user":"ann","password":"url-pw-60"},
          "query":[{"key":"api_key","value":"q-key-61"},{"key":"page","value":"2"}]},
        "body":{"mode":"urlencoded","urlencoded":[{"key":"user","value":"ann"},{"key":"password","value":"form-pw-62"}]}}}"#,
    );
    let (_d, c, report, store) = import(&input);
    let text = all_text(c.root());
    for value in ["url-pw-60", "q-key-61", "form-pw-62"] {
        assert!(!text.contains(value), "`{value}` in a file:\n{text}");
    }
    for (var, value) in [
        ("login_api_key", "q-key-61"),
        ("login_password", "form-pw-62"),
    ] {
        assert_eq!(stored(&c, &store, var).as_deref(), Some(value), "{var}");
    }
    assert!(text.contains("page"), "other params stay: {text}");
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.detail.contains("`login_api_key`")),
        "{:?}",
        report.notes
    );
}

#[test]
fn a_curl_url_with_a_password_is_kept_out_of_the_file() {
    let store = MemoryStore::new();
    let (_d, mut c) = with_dev(&store);
    import_curl(
        &mut c,
        &store,
        "U",
        "curl 'https://ann:curl-url-pw-63@x.test/u?token=curl-q-64'",
    );
    let text = text_of(&c, "U.yml");
    assert!(
        !text.contains("curl-url-pw-63") && !text.contains("curl-q-64"),
        "{text}"
    );
    assert_eq!(
        stored(&c, &store, "x_test_token").as_deref(),
        Some("curl-q-64")
    );
}

/// A collection imported from `SPEC_V1`, its values in `store`.
fn keys_v1(store: &MemoryStore) -> (TempDir, Collection) {
    let parent = TempDir::new().expect("tempdir");
    let v1 = openapi(SPEC_V1);
    let (c, _) =
        write_collection(parent.path(), &v1.collection, &v1.environments, store).expect("writes");
    (parent, c)
}

/// The one variable besides `baseUrl` the request file at `rel` references.
fn referenced(c: &Collection, rel: &str) -> String {
    let text = std::fs::read_to_string(c.root().join(rel)).expect("reads");
    let names: Vec<&str> = text
        .split("{{")
        .skip(1)
        .filter_map(|rest| rest.split_once("}}").map(|(name, _)| name))
        .filter(|name| *name != "baseUrl")
        .collect();
    assert_eq!(names.len(), 1, "{text}");
    names[0].to_owned()
}

#[test]
fn an_update_never_points_a_request_at_a_name_holding_another_value() {
    // A first, clean update shows which name the new operation's key takes.
    let clean = MemoryStore::new();
    let (_p, mut c) = keys_v1(&clean);
    let mut options = UpdateOptions::default();
    options.secrets = Some(Arc::new(clean) as Arc<dyn SecretStore>);
    let added = apply_update(&mut c, &openapi(SPEC_V2), &options, None).expect("applies");
    let rel = format!("{}", added.added[0]);
    let name = referenced(&c, &rel);

    // Here the collection already keeps another credential under that name.
    let store = Arc::new(MemoryStore::new());
    let (_q, mut c) = keys_v1(store.as_ref());
    let env = c.environments()[0].name().to_owned();
    let e = c.environment_mut(&env).expect("env");
    let mut vars = e.variables();
    vars.push(apinata_core::EnvVariable::secret(name.clone()));
    e.set_variables(vars);
    e.save().expect("saves");
    store
        .set(&c.secret_key(&env, &name), "other-60")
        .expect("sets");
    options.secrets = Some(Arc::clone(&store) as Arc<dyn SecretStore>);
    apply_update(&mut c, &openapi(SPEC_V2), &options, None).expect("applies");
    let c = Collection::open(c.root()).expect("reopens");
    assert_eq!(stored(&c, &store, &name).as_deref(), Some("other-60"));
    let used = referenced(&c, &rel);
    assert_ne!(used, name, "the request must not take over `{name}`");
    assert_eq!(stored(&c, &store, &used).as_deref(), Some("spec-key-31"));
    let again = plan_update(&c, &openapi(SPEC_V2), &options).expect("plans");
    assert!(
        again.changed.is_empty() && again.added.is_empty(),
        "{again:?}"
    );
}

#[test]
fn an_update_keeps_a_default_environment_already_chosen() {
    let store = Arc::new(MemoryStore::new());
    let (_p, mut c) = keys_v1(store.as_ref());
    c.create_environment("other", store.as_ref()).expect("env");
    c.file_mut().set_default_environment("other");
    c.file_mut().save().expect("saves");
    let mut options = UpdateOptions::default();
    options.secrets = Some(Arc::clone(&store) as Arc<dyn SecretStore>);
    apply_update(&mut c, &openapi(SPEC_V2), &options, None).expect("applies");
    let c = Collection::open(c.root()).expect("reopens");
    assert_eq!(
        c.choose_environment(None),
        EnvironmentChoice::Preset("other".to_owned())
    );
}

/// A store that reads from `MemoryStore` and refuses every write.
struct ReadOnly<'a>(&'a MemoryStore);

impl SecretStore for ReadOnly<'_> {
    fn get(
        &self,
        key: &apinata_core::SecretKey,
    ) -> Result<Option<String>, apinata_core::SecretError> {
        self.0.get(key)
    }
    fn set(&self, _: &apinata_core::SecretKey, _: &str) -> Result<(), apinata_core::SecretError> {
        Err(apinata_core::SecretError::Unavailable {
            message: "read only".to_owned(),
        })
    }
    fn delete(&self, _: &apinata_core::SecretKey) -> Result<(), apinata_core::SecretError> {
        Ok(())
    }
}

#[test]
fn a_secret_that_could_not_be_stored_after_a_rename_is_noted_by_its_new_name() {
    let store = MemoryStore::new();
    let (_d, mut c) = with_dev(&store);
    import_curl(&mut c, &store, "A", "curl -u a:first-pw http://h.test/x");
    let refusing = ReadOnly(&store);
    let mut parsed = parse_request(None, "curl -u a:other-pw http://h.test/x").expect("parses");
    claim_secret_names(&c, &mut parsed.request, None, &refusing);
    let name = parsed.request.secrets[0].name.clone();
    assert_ne!(name, "h_test_password");
    add_request(&mut c, &ItemPath::root(), "B", &parsed.request).expect("adds");
    store_request_secrets(&mut c, &parsed.request, None, &refusing, &mut parsed.notes);
    let note = parsed
        .notes
        .iter()
        .find(|n| n.detail.contains("keychain"))
        .expect("a note says why");
    assert!(!note.detail.contains("read only"), "{note:?}");
    assert!(note.detail.contains(&format!("`{name}`")), "{note:?}");
    assert!(!note.detail.contains("`h_test_password`"), "{note:?}");
}
