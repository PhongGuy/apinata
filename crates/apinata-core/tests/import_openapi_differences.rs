//! Departures from Bruno 1301ea17's OpenAPI import, each with its test, and
//! the summaries compared with Bruno's.

use apinata_core::import::{
    CollectionFormat, CollectionOptions, ImportNote, ImportedItem, ImportedRequest,
    parse_collections,
};
use apinata_core::{
    ApiKeyPlacement, AuthSpec, BodySpec, KeyValue, MultipartValue, ParamKind, VarValue,
};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

const FIELD: bool = true;
const DOC: bool = false;

/// (id, compared, test that pins it, departure)
#[rustfmt::skip]
const DIFFERENCES: &[(&str, bool, &str, &str)] = &[
    ("security-empty", FIELD, "empty_operation_security_is_none", "`security: []` on an operation is no auth"),
    ("http-scheme-case", FIELD, "http_schemes_match_case_insensitively", "http schemes match in any case"),
    ("apikey-auth-only", FIELD, "an_api_key_is_auth_only_and_cookie_keys_are_dropped", "an apiKey is auth only; cookie keys are dropped"),
    ("unsupported-auth", FIELD, "unsupported_schemes_become_none_with_a_note", "mutualTLS is no auth; oauth2, openIdConnect and digest import with placeholders"),
    ("secret-placeholders", FIELD, "auth_placeholders_become_secret_variables", "auth placeholders are secret variables"),
    ("reserved-and-cookie-params", DOC, "reserved_headers_cookies_and_unknown_locations_are_dropped_with_notes", "reserved headers and cookie params are dropped"),
    ("no-paths", FIELD, "a_document_without_paths_imports_empty", "a document without paths imports empty"),
    ("versions", FIELD, "unknown_versions_are_unsupported", "only 2.0 and 3.0 to 3.2 are read"),
    ("names-per-folder", DOC, "names_are_deduplicated_per_folder", "names are deduplicated per folder"),
    ("query-in-url", FIELD, "enabled_query_params_are_in_the_url", "enabled query params are also in the URL"),
    ("collection-format", DOC, "swagger_2_parameters_use_their_collection_format", "2.0 array defaults use collectionFormat"),
    ("refs-anywhere", DOC, "local_pointers_resolve_anywhere_with_escapes_and_indexes", "any local pointer resolves"),
    ("ref-siblings", FIELD, "siblings_follow_the_version_rules", "3.1 reference siblings are merged shallowly"),
    ("ref-cycles", FIELD, "recursive_schemas_stop_at_the_cycle_with_a_note", "a $ref cycle yields an empty object"),
    ("vendor-variants", DOC, "vendor_extensions_add_no_requests", "x- extensions add no requests"),
    ("all-of", DOC, "all_of_members_merge_in_samples", "allOf is merged in samples"),
    ("all-of-form-fields", DOC, "form_fields_come_from_the_example_or_the_properties", "form fields include allOf properties"),
    ("one-of", DOC, "one_of_and_any_of_take_their_first_branch", "oneOf and anyOf take their first branch"),
    ("examples-const", DOC, "const_and_3_1_examples_feed_samples", "3.1 examples and const feed samples"),
    ("media-parameters", DOC, "media_type_parameters_are_ignored", "media type parameters are ignored"),
    ("xml-escaped", DOC, "xml_bodies_follow_the_schema_shape", "XML values are escaped"),
    ("sparql-dropped", DOC, "text_and_other_media_types", "sparql-query bodies are dropped with a note"),
    ("responses", FIELD, "responses_links_and_callbacks_are_not_imported", "response examples, links, callbacks are not imported"),
    ("sample-bounds", DOC, "deep_and_large_samples_are_cut_with_a_note", "samples stop at 16 levels, 128 nested hops, 64 KiB and a step budget"),
    ("extra-media-types", DOC, "media_type_parameters_are_ignored", "media types after the first get a note"),
    ("external-value", DOC, "an_external_example_is_never_fetched", "an externalValue is never fetched"),
    ("json-text", DOC, "array_and_object_values_are_json_text", "array and object values are JSON text, not `a,b` or `[object Object]`"),
    ("deep-paths", DOC, "a_deep_path_stops_at_32_folders_and_many_paths_still_import", "path folders nest at most 32 deep"),
];

/// Every `.rs` file under the core's tests and the OpenAPI importer, fixtures and snapshots skipped.
fn sources() -> String {
    let core = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut text = String::new();
    let mut dirs = vec![core.join("tests"), core.join("src/import/openapi")];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir).expect("dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() && !path.ends_with("fixtures") && !path.ends_with("snapshots") {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                text.push_str(&fs::read_to_string(&path).expect("read"));
            }
        }
    }
    text
}

#[test]
fn every_listed_difference_has_its_test() {
    let text = sources();
    for (id, _, test, _) in DIFFERENCES {
        assert!(
            text.contains(&format!("fn {test}(")),
            "{id}: no test named {test}"
        );
    }
    let mut ids: Vec<&str> = DIFFERENCES.iter().map(|d| d.0).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), DIFFERENCES.len(), "ids are unique");
}

/// The fixtures 4f's detection matrix leaves undetected; neither side summarises them.
const UNDETECTED: &[&str] = &[
    "bruno/openapi-broken.json",
    "bruno/openapi-empty.yaml",
    "bruno/openapi-invalid-extension.txt",
    "bruno/openapi-malformed.yaml",
    "bruno/openapi-multifile-endpoint.yaml",
];

/// Copied verbatim from 4h's `walk` in `tests/import_openapi_fixtures.rs`, so both files apply
/// the same filter to the same corpus.
fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("dir") {
        let path = entry.expect("entry").path();
        let name = path
            .file_name()
            .expect("name")
            .to_string_lossy()
            .into_owned();
        if path.is_dir() {
            walk(&path, out);
        } else if !matches!(name.as_str(), "NOTICE.md" | "LICENSE" | "LICENSE.md")
            // The spec-update fixtures are this project's own and tested by import_update_*.
            && !name.starts_with("update-")
        {
            out.push(path);
        }
    }
}

/// The detected OpenAPI fixtures as `(relative path, bytes)`, sorted.
fn corpus() -> Vec<(String, Vec<u8>)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/openapi");
    let mut list = Vec::new();
    walk(&root, &mut list);
    list.sort();
    list.into_iter()
        .map(|p| {
            let rel = p
                .strip_prefix(&root)
                .expect("under root")
                .to_string_lossy()
                .replace('\\', "/");
            (rel, fs::read(&p).expect("read"))
        })
        .filter(|(rel, _)| !UNDETECTED.contains(&rel.as_str()))
        .collect()
}

fn all_notes<'a>(items: &'a [ImportedItem], out: &mut Vec<&'a ImportNote>) {
    for item in items {
        match item {
            ImportedItem::Folder(f) => {
                out.extend(&f.notes);
                all_notes(&f.items, out);
            }
            ImportedItem::Request(r) => out.extend(&r.notes),
            _ => {}
        }
    }
}

/// The documented-only ids `sample-bounds` and `external-value` must not occur in this corpus;
/// if this fails, the id that tripped becomes FIELD and gets a `known.json` entry.
#[test]
fn documented_differences_do_not_occur_in_the_corpus() {
    let markers = [
        "nesting deeper than",
        "nested more than",
        "the sample was cut",
        "schema steps",
        "the external example",
    ];
    let options = CollectionOptions::default();
    let corpus = corpus();
    assert_eq!(corpus.len(), 31, "4h's fixture count");
    for (rel, bytes) in corpus {
        let Ok(bundles) = parse_collections(Some(CollectionFormat::OpenApi), &bytes, &options)
        else {
            continue;
        };
        for b in &bundles {
            let c = &b.collection;
            let mut notes: Vec<&ImportNote> = c.notes.iter().chain(&c.unwritten).collect();
            all_notes(&c.items, &mut notes);
            for n in notes {
                assert!(
                    !markers.iter().any(|m| n.detail.contains(m)),
                    "{rel}: {}",
                    n.detail
                );
            }
        }
    }
}

/// The comparison summary of one fixture: the error text, or the first bundle in the shape
/// `bruno-summary.mjs` writes for the converter's output.
fn summary(fixture: &str, input: &[u8]) -> Value {
    let options = CollectionOptions::default();
    let bundles = match parse_collections(Some(CollectionFormat::OpenApi), input, &options) {
        Ok(bundles) => bundles,
        Err(e) => return json!({"fixture": fixture, "result": "error", "error": e.to_string()}),
    };
    let bundle = bundles.first().expect("one bundle");
    let c = &bundle.collection;
    let environments: Vec<Value> = bundle
        .environments
        .iter()
        .map(|e| {
            let variables: Vec<Value> = e
                .variables
                .iter()
                .map(|v| json!([v.name, var_text(&v.value), v.secret]))
                .collect();
            json!({"name": e.name, "variables": variables})
        })
        .collect();
    json!({
        "fixture": fixture,
        "result": "ok",
        "name": c.name,
        "auth": auth(&c.auth),
        "environments": environments,
        "items": items(&c.items),
    })
}

fn var_text(value: &VarValue) -> &str {
    match value {
        VarValue::Text(t) => t,
        VarValue::Typed { data, .. } => data,
        _ => "",
    }
}

fn items(list: &[ImportedItem]) -> Vec<Value> {
    list.iter()
        .filter_map(|item| match item {
            ImportedItem::Folder(f) => {
                Some(json!({"type": "folder", "name": f.name, "items": items(&f.items)}))
            }
            ImportedItem::Request(r) => Some(request(&r.name, &r.request)),
            _ => None,
        })
        .collect()
}

fn request(name: &str, r: &ImportedRequest) -> Value {
    let params: Vec<Value> = r
        .params
        .iter()
        .map(|p| {
            let kind = if p.kind == ParamKind::Path {
                "path"
            } else {
                "query"
            };
            json!([p.name, p.value, kind, p.enabled])
        })
        .collect();
    let vars: Vec<Value> = r
        .vars
        .iter()
        .map(|v| json!([v.name, var_text(&v.value)]))
        .collect();
    json!({
        "type": "request",
        "name": name,
        "method": r.method,
        "url": r.url,
        "params": params,
        "headers": rows(&r.headers),
        "body": body(&r.body),
        "auth": auth(&r.auth),
        "vars": vars,
        "examples": 0,
    })
}

fn rows(list: &[KeyValue]) -> Vec<Value> {
    list.iter()
        .map(|kv| json!([kv.name, kv.value, kv.enabled]))
        .collect()
}

/// JSON bodies compare as parsed values, so formatting differences between the two sides vanish.
fn body(spec: &BodySpec) -> Value {
    match spec {
        BodySpec::None => json!({"mode": "none"}),
        BodySpec::Json(t) => {
            let value = serde_json::from_str::<Value>(t).unwrap_or_else(|_| json!(t));
            json!({"mode": "json", "value": value})
        }
        BodySpec::Xml(t) => json!({"mode": "xml", "value": t}),
        BodySpec::Text(t) => json!({"mode": "text", "value": t}),
        BodySpec::FormUrlEncoded(fields) => {
            json!({"mode": "formUrlEncoded", "value": rows(fields)})
        }
        BodySpec::Multipart(fields) => {
            let value: Vec<Value> = fields
                .iter()
                .map(|f| {
                    let text = match &f.value {
                        MultipartValue::Text(t) => t.as_str(),
                        _ => "<file>",
                    };
                    json!([f.name, text, f.enabled])
                })
                .collect();
            json!({"mode": "multipartForm", "value": value})
        }
        _ => json!({"mode": "other"}),
    }
}

fn auth(spec: &AuthSpec) -> Value {
    match spec {
        AuthSpec::None => json!({"mode": "none"}),
        AuthSpec::Inherit => json!({"mode": "inherit"}),
        AuthSpec::Basic { username, password } => {
            json!({"mode": "basic", "username": username, "password": password})
        }
        AuthSpec::Bearer { token } => json!({"mode": "bearer", "token": token}),
        AuthSpec::ApiKey {
            key,
            value,
            placement,
        } => {
            let placement = if *placement == ApiKeyPlacement::Query {
                "queryparams"
            } else {
                "header"
            };
            json!({"mode": "apikey", "key": key, "value": value, "placement": placement})
        }
        _ => json!({"mode": "other"}),
    }
}

#[test]
#[ignore = "writes summaries for the comparison script; set OPENAPI_SUMMARY_DIR"]
fn write_summaries_for_the_comparison_script() {
    let out = PathBuf::from(std::env::var("OPENAPI_SUMMARY_DIR").expect("OPENAPI_SUMMARY_DIR"));
    fs::create_dir_all(&out).expect("mkdir");
    let corpus = corpus();
    assert_eq!(corpus.len(), 31, "4h's fixture count");
    for (rel, bytes) in corpus {
        let name = format!("{}.json", rel.replace('/', "__"));
        let text = serde_json::to_string_pretty(&summary(&rel, &bytes)).expect("JSON");
        fs::write(out.join(name), text).expect("write");
    }
}

#[test]
fn the_comparison_script_knows_exactly_the_compared_differences() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reference/2026-10-10-openapi-comparison/known.json");
    let known: Value =
        serde_json::from_str(&fs::read_to_string(path).expect("known.json")).expect("JSON");
    let mut ids: Vec<&str> = known
        .as_array()
        .expect("array")
        .iter()
        .map(|k| k["id"].as_str().expect("id"))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    let mut compared: Vec<&str> = DIFFERENCES.iter().filter(|d| d.1).map(|d| d.0).collect();
    compared.sort_unstable();
    assert_eq!(ids, compared);
}
