//! Every key neither consumed nor exempt yields a note (spec criterion 3).

use crate::openapi_support;

use apinata_core::import::{ImportNote, NoteKind};
use openapi_support::{import, notes, request};
use serde_json::{Value, json};

const PROBE: &str = "zzProbe";

fn base() -> Value {
    json!({"openapi": "3.1.0", "jsonSchemaDialect": "d", "externalDocs": {},
        "info": {"title": "t", "version": "1", "summary": "s", "termsOfService": "x", "contact": {}, "license": {}, "x-logo": {}},
        "servers": [{"url": "https://{h}.test", "variables": {"h": {"default": "x", "description": "host"}}}],
        "tags": [{"name": "a", "description": "A", "externalDocs": {}}],
        "security": [{"key": []}],
        "components": {"securitySchemes": {"key": {"type": "apiKey", "in": "header", "name": "K", "description": "d"}},
            "schemas": {"Unused": {"type": "object"}}},
        "paths": {"/p": {"summary": "s", "description": "d", "parameters": [{"name": "q", "in": "query", "style": "form", "allowReserved": true}],
            "get": {"summary": "G", "tags": ["a"], "deprecated": true, "externalDocs": {}, "responses": {"200": {"description": "ok"}},
                "parameters": [{"name": "h", "in": "header", "explode": false, "deprecated": true, "allowEmptyValue": true, "content": {}}],
                "requestBody": {"description": "b", "required": true, "content": {"application/json": {"schema": {"type": "object"}, "encoding": {}}}}}}}})
}

fn dropped(list: &[&ImportNote]) -> Vec<String> {
    list.iter()
        .filter(|n| n.kind == NoteKind::Dropped)
        .map(|n| n.source.clone())
        .collect()
}

/// A note's detail without its backtick- or double-quoted spans: the text the importer wrote.
fn fixed_text(detail: &str) -> String {
    let unticked: String = detail.split('`').step_by(2).collect();
    unticked.split('"').step_by(2).collect()
}

#[test]
fn exempt_and_consumed_keys_are_silent() {
    let c = import(&base()).collection;
    assert!(dropped(&notes(&c)).is_empty(), "{:?}", dropped(&notes(&c)));
}

#[test]
fn swagger_2_exempt_keys_are_silent() {
    let doc = json!({"swagger": "2.0", "host": "x.test", "schemes": ["https"], "produces": ["application/json"], "externalDocs": {},
        "paths": {"/s": {"get": {"summary": "S", "produces": ["text/plain"], "schemes": ["http"], "responses": {},
            "parameters": [{"name": "n", "in": "query", "type": "integer", "minimum": 1, "collectionFormat": "csv"}]}}}});
    let c = import(&doc).collection;
    assert!(dropped(&notes(&c)).is_empty(), "{:?}", dropped(&notes(&c)));
}

#[test]
fn an_unknown_key_at_every_level_is_noted() {
    let at: [&[&str]; 9] = [
        &[],
        &["info"],
        &["tags", "0"],
        &["servers", "0"],
        &["servers", "0", "variables", "h"],
        &["paths", "/p"],
        &["paths", "/p", "get"],
        &["paths", "/p", "get", "parameters", "0"],
        &[
            "paths",
            "/p",
            "get",
            "requestBody",
            "content",
            "application/json",
        ],
    ];
    for path in at {
        let mut doc = base();
        let mut target = &mut doc;
        for key in path {
            target = match key.parse::<usize>() {
                Ok(i) => &mut target[i],
                Err(_) => &mut target[*key],
            };
        }
        target[PROBE] = json!(1);
        let want: String = path
            .iter()
            .map(|k| {
                if k.parse::<usize>().is_ok() {
                    format!("[{k}]")
                } else {
                    format!(".{k}")
                }
            })
            .collect();
        let want = format!("{want}.{PROBE}").trim_start_matches('.').to_owned();
        assert_eq!(
            dropped(&notes(&import(&doc).collection)),
            [want],
            "{path:?}"
        );
    }
    let mut doc = base();
    doc["components"]["securitySchemes"]["key"][PROBE] = json!(1);
    assert_eq!(
        dropped(&notes(&import(&doc).collection)),
        [format!("components.securitySchemes.key.{PROBE}")]
    );
}

#[test]
fn request_level_notes_land_on_the_request() {
    let mut doc = base();
    doc["paths"]["/p"]["get"][PROBE] = json!(1);
    let c = import(&doc).collection;
    let r = request(&c, "a/G");
    let source = format!("paths./p.get.{PROBE}");
    assert!(
        r.notes.iter().any(|n| n.kind == NoteKind::Dropped
            && n.source == source
            && n.detail.contains(&format!("`{PROBE}`"))),
        "{:?}",
        r.notes
    );
}

#[test]
fn responses_links_and_callbacks_are_not_imported() {
    let mut doc = base();
    doc["paths"]["/p"]["get"]["responses"]["200"]["links"] = json!({"next": {"operationId": "x"}});
    doc["paths"]["/p"]["get"]["callbacks"] = json!({"onEvent": {}});
    let c = import(&doc).collection;
    let mut details: Vec<String> = notes(&c)
        .iter()
        .filter(|n| n.kind == NoteKind::Dropped)
        .map(|n| n.detail.clone())
        .collect();
    details.sort();
    assert_eq!(
        details,
        [
            "callbacks are not imported",
            "response links are not imported"
        ]
    );
}

#[test]
fn no_note_names_another_tool() {
    let mut doc = base();
    doc["paths"]["/p"]["get"]["callbacks"] = json!({});
    doc["paths"]["/p"]["get"]["x-bruno-variants"] = json!([]);
    doc["paths"]["/p"]["get"]["brunoKey"] = json!(1);
    doc["paths"]["/p"]["post"] = json!({"summary": "P", "security": [{"oauth": []}, {"key": []}],
        "requestBody": {"content": {"image/png": {}, "multipart/form-data": {}}}});
    doc["paths"]["/p"]["put"] = json!({"summary": "U", "requestBody": {"content": {"multipart/form-data": {"schema": {"properties": {"f": {"type": "string", "format": "binary"}}}}}}});
    doc["components"]["securitySchemes"]["oauth"] = json!({"type": "oauth2", "flows": {}});
    let c = import(&doc).collection;
    let all = notes(&c);
    assert!(all.len() >= 6, "{all:?}");
    assert!(
        all.iter().any(|n| n.detail.contains("`brunoKey`")),
        "quoted content is kept: {all:?}"
    );
    for n in all {
        assert!(
            !fixed_text(&n.detail).to_lowercase().contains("bruno"),
            "{}",
            n.detail
        );
    }
}
