//! Descriptions on headers, params and form fields against the
//! description parity corpus, written by Bruno's own writer.

use apinata_core::{BodySpec, KeyValue, MultipartField, Param, ParamKind, RequestFile};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reference/2026-10-09-description-parity")
}

fn kv(name: &str, value: &str, description: &str) -> KeyValue {
    let mut kv = KeyValue::new(name, value);
    kv.description = Some(description.to_owned());
    kv
}

fn param(name: &str, value: &str, kind: ParamKind, description: &str) -> Param {
    let mut param = Param::new(name, value, kind);
    param.description = Some(description.to_owned());
    param
}

fn described(mut field: MultipartField, description: &str) -> MultipartField {
    field.description = Some(description.to_owned());
    field
}

/// The Apinata edit matching each corpus case id.
fn apply(id: &str, request: &mut RequestFile) {
    match id {
        "desc-new-headers" => {
            let mut off = kv("x-c", "3", "");
            off.enabled = false;
            request
                .set_headers(vec![kv("x-a", "1", "Header A"), kv("x-b", "2", "   "), off])
                .expect("valid");
        }
        "desc-new-params" => request.set_params(vec![
            param("q", "2", ParamKind::Query, "Search term"),
            param("id", "7", ParamKind::Path, " \t"),
        ]),
        "desc-new-form" => request.set_body(BodySpec::FormUrlEncoded(vec![
            kv("a", "1", "First"),
            kv("b", "2", ""),
        ])),
        "desc-new-multipart" => request.set_body(BodySpec::Multipart(vec![
            described(MultipartField::text("t", "v"), "Text part"),
            described(
                MultipartField::files("f", vec!["a.png".into()]),
                "File part",
            ),
            described(MultipartField::text("n", "w"), " "),
        ])),
        "desc-edit-keeps" => {
            let mut headers = request.headers();
            headers[0].value = "changed".into();
            request.set_headers(headers).expect("valid");
        }
        "desc-clear" => {
            let mut headers = request.headers();
            headers.iter_mut().for_each(|h| h.description = None);
            request.set_headers(headers).expect("valid");
            let mut params = request.params();
            params.iter_mut().for_each(|p| p.description = None);
            request.set_params(params);
            let BodySpec::FormUrlEncoded(mut fields) = request.body() else {
                panic!("form body")
            };
            fields.iter_mut().for_each(|f| f.description = None);
            request.set_body(BodySpec::FormUrlEncoded(fields));
        }
        other => panic!("no Apinata edit for case {other}"),
    }
}

#[test]
fn descriptions_are_written_as_bruno_writes_them() {
    let ids: Vec<String> =
        serde_json::from_str(&fs::read_to_string(corpus().join("cases.json")).expect("cases"))
            .expect("valid");
    assert!(!ids.is_empty(), "the corpus has cases");
    let source = fs::read_to_string(corpus().join("source.yml")).expect("source.yml");
    let mut failures = Vec::new();
    for id in &ids {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("described.yml");
        fs::write(&path, &source).expect("write copy");
        let mut request = RequestFile::load(&path).expect("source loads");
        apply(id, &mut request);
        request.save().expect("saves");
        let actual = fs::read_to_string(&path).expect("read back");
        let expected = fs::read_to_string(corpus().join("expected").join(format!("{id}.yml")))
            .expect("expected");
        if actual != expected {
            failures.push(format!(
                "{id}\n  expected {expected:?}\n  actual   {actual:?}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} cases differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn descriptions_are_read_from_strings_and_content_mappings() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("r.yml");
    let yaml = "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: https://x.test\n  headers:\n    - name: a\n      value: \"1\"\n      description: Plain\n    - name: b\n      value: \"2\"\n      description:\n        content: From object\n    - name: c\n      value: \"3\"\n      description: \"  \"\n  params:\n    - name: q\n      value: \"1\"\n      type: query\n      description: Search\n";
    fs::write(&path, yaml).expect("write");
    let request = RequestFile::load(&path).expect("loads");
    let headers = request.headers();
    assert_eq!(headers[0].description.as_deref(), Some("Plain"));
    assert_eq!(headers[1].description.as_deref(), Some("From object"));
    assert_eq!(
        headers[2].description, None,
        "a blank description reads as none"
    );
    assert_eq!(request.params()[0].description.as_deref(), Some("Search"));
}

#[test]
fn a_description_change_makes_entries_unequal() {
    assert_ne!(kv("a", "1", "one"), kv("a", "1", "two"));
    assert_ne!(
        param("q", "1", ParamKind::Query, "one"),
        Param::new("q", "1", ParamKind::Query)
    );
    assert_eq!(KeyValue::new("a", "1").description, None);
}
