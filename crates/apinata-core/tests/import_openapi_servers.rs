//! Servers become environments and request variables.

use crate::openapi_support;

use apinata_core::VarValue;
use apinata_core::import::{ImportNote, ImportedEnvironment, NoteKind};
use openapi_support::{import, notes, request};
use serde_json::json;

fn text(value: &VarValue) -> String {
    match value {
        VarValue::Text(t) => t.clone(),
        VarValue::Typed { data, .. } => data.clone(),
        _ => unreachable!("text or typed"),
    }
}

/// Each variable as name, text and whether it is secret.
type Vars = Vec<(String, String, bool)>;

fn vars(env: &ImportedEnvironment) -> Vars {
    env.variables
        .iter()
        .map(|v| (v.name.clone(), text(&v.value), v.secret))
        .collect()
}

fn v(name: &str, value: &str) -> (String, String, bool) {
    (name.to_owned(), value.to_owned(), false)
}

#[test]
fn each_server_becomes_an_environment_with_its_variables() {
    let doc = json!({"openapi": "3.1.0", "paths": {}, "servers": [
        {"url": "https://api.example.test/v1/", "description": "Production"},
        {"url": "https://{region}.example.test:{port}/{base}", "name": "Regional", "variables": {
            "region": {"default": "eu", "enum": ["eu", "us"]}, "port": {"default": 8443}, "base": {"enum": ["v2", "v3"]}, "unused": {}
        }},
        {"url": "https://staging.example.test", "description": "Production"},
        {"url": "http://localhost:8080"}
    ]});
    let bundle = import(&doc);
    let envs: Vec<(&str, Vars)> = bundle
        .environments
        .iter()
        .map(|e| (e.name.as_str(), vars(e)))
        .collect();
    assert_eq!(
        envs,
        [
            (
                "Production",
                vec![v("baseUrl", "https://api.example.test/v1")]
            ),
            (
                "Regional",
                vec![
                    v(
                        "baseUrl",
                        "https://{{region}}.example.test:{{port}}/{{base}}"
                    ),
                    v("region", "eu"),
                    v("port", "8443"),
                    v("base", "v2"),
                    v("unused", "")
                ]
            ),
            (
                "Production (2)",
                vec![v("baseUrl", "https://staging.example.test")]
            ),
            ("Environment 4", vec![v("baseUrl", "http://localhost:8080")]),
        ]
    );
    assert!(
        notes(&bundle.collection)
            .iter()
            .all(|n| n.kind != NoteKind::NeedsInput)
    );
}

#[test]
fn without_an_absolute_server_base_url_needs_input() {
    let cases = [
        json!({"openapi": "3.0.0", "paths": {}}),
        json!({"openapi": "3.0.0", "paths": {}, "servers": []}),
        json!({"openapi": "3.0.0", "paths": {}, "servers": [{"url": "/api/v3"}, {"url": "{scheme}://x.test"}]}),
        json!({"swagger": "2.0", "paths": {}, "basePath": "/v2"}),
        json!({"swagger": "2.0", "paths": {}}),
    ];
    for doc in cases {
        let c = import(&doc).collection;
        let needs: Vec<&ImportNote> = c
            .notes
            .iter()
            .filter(|n| n.kind == NoteKind::NeedsInput)
            .collect();
        assert_eq!(needs.len(), 1, "{doc}");
        assert!(needs[0].detail.contains("baseUrl"), "{}", needs[0].detail);
    }
    let relative =
        import(&json!({"openapi": "3.0.0", "paths": {}, "servers": [{"url": "/api/v3/"}]}));
    assert_eq!(vars(&relative.environments[0]), [v("baseUrl", "/api/v3")]);
}

#[test]
fn swagger_2_servers_come_from_schemes_host_and_base_path() {
    let one = import(
        &json!({"swagger": "2.0", "paths": {}, "host": "petstore.test:8080", "basePath": "/v2/"}),
    );
    let got: Vec<(&str, Vars)> = one
        .environments
        .iter()
        .map(|e| (e.name.as_str(), vars(e)))
        .collect();
    assert_eq!(
        got,
        [(
            "Environment",
            vec![v("baseUrl", "https://petstore.test:8080/v2")]
        )]
    );
    let two = import(
        &json!({"swagger": "2.0", "paths": {}, "host": "p.test", "schemes": ["http", "https"]}),
    );
    let urls: Vec<(&str, String)> = two
        .environments
        .iter()
        .map(|e| (e.name.as_str(), vars(e)[0].1.clone()))
        .collect();
    assert_eq!(
        urls,
        [
            ("Environment 1", "http://p.test".to_owned()),
            ("Environment 2", "https://p.test".to_owned())
        ]
    );
    let relative = import(&json!({"swagger": "2.0", "paths": {}, "basePath": "/v2"}));
    assert_eq!(vars(&relative.environments[0]), [v("baseUrl", "/v2")]);
}

#[test]
fn path_and_operation_servers_become_request_variables() {
    let doc = json!({"openapi": "3.0.0", "servers": [{"url": "https://x.test"}], "paths": {
        "/a": {
            "servers": [{"url": "https://path.test/"}],
            "get": {"summary": "path level"},
            "put": {"summary": "operation level", "servers": [
                {"url": "https://{env}.op.test", "variables": {"env": {"default": "dev"}}},
                {"url": "https://other.test"}
            ]}
        },
        "/b": {"get": {"summary": "root level"}}
    }});
    let c = import(&doc).collection;
    let vars_of = |name: &str| -> Vec<(String, String)> {
        request(&c, name)
            .request
            .vars
            .iter()
            .map(|v| (v.name.clone(), text(&v.value)))
            .collect()
    };
    assert_eq!(
        vars_of("path level"),
        [("baseUrl".to_owned(), "https://path.test".to_owned())]
    );
    assert_eq!(
        vars_of("operation level"),
        [
            ("baseUrl".to_owned(), "https://{{env}}.op.test".to_owned()),
            ("env".to_owned(), "dev".to_owned())
        ]
    );
    assert!(vars_of("root level").is_empty());
    let put = request(&c, "operation level");
    assert!(
        put.notes
            .iter()
            .any(|n| n.kind == NoteKind::Converted && n.source == "paths./a.put.servers"),
        "{:?}",
        put.notes
    );
}

#[test]
fn a_server_variable_named_base_url_defines_base_url_once() {
    let doc = json!({"openapi": "3.1.0", "paths": {}, "servers": [
        {"url": "{baseUrl}", "variables": {"baseUrl": {"default": "https://api.example.test"}}},
        {"url": "{baseUrl}/v1/", "variables": {"baseUrl": {"default": "https://api.example.test"}}},
        {"url": "{baseUrl}", "variables": {"baseUrl": {}}}
    ]});
    let bundle = import(&doc);
    let envs: Vec<Vars> = bundle.environments.iter().map(vars).collect();
    assert_eq!(
        envs,
        [
            vec![v("baseUrl", "https://api.example.test")],
            vec![v("baseUrl", "https://api.example.test/v1")],
            vec![v("baseUrl", "")],
        ]
    );
}
