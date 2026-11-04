//! OpenAPI documents: versions and the operation walk.

use crate::openapi_support;

use openapi_support::{import, requests, tree};
use serde_json::json;

#[test]
fn every_supported_version_imports_its_operations_in_document_order() {
    for version in [
        json!({"openapi": "3.0.3"}),
        json!({"openapi": "3.1.0"}),
        json!({"openapi": "3.2.0"}),
        json!({"swagger": "2.0"}),
    ] {
        let mut doc = json!({
            "info": {"title": "  Pets API ", "description": "All about pets", "version": "1"},
            "paths": {
                "/pets": {"post": {"summary": "Create pet", "description": "Adds a pet"}, "get": {"operationId": "listPets"}},
                "/pets/{petId}/toys/{toy-id}": {"delete": {}}
            }
        });
        doc.as_object_mut()
            .expect("object")
            .extend(version.as_object().expect("object").clone());
        let c = import(&doc).collection;
        assert_eq!(
            (c.name.as_str(), c.docs.as_str()),
            ("Pets API", "All about pets")
        );
        assert_eq!(
            tree(&c),
            [
                "R 1 Create pet POST {{baseUrl}}/pets",
                "R 2 listPets GET {{baseUrl}}/pets",
                "R 3 delete /pets/:petId/toys/:toy-id DELETE {{baseUrl}}/pets/:petId/toys/:toy-id",
            ],
            "{version}"
        );
        assert_eq!(requests(&c)[0].1.request.docs, "Adds a pet");
    }
}

#[test]
fn swagger_2_has_no_trace_method() {
    let doc = json!({"swagger": "2.0", "paths": {"/a": {"trace": {}, "get": {}}}});
    assert_eq!(
        tree(&import(&doc).collection),
        ["R 1 get /a GET {{baseUrl}}/a"]
    );
}
