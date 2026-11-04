//! Format dispatch: stable ids, detection, and parsing by detected or named format.

use apinata_core::import::{
    CollectionFormat, EnvironmentFormat, FormatError, NoteKind, PostmanError, RequestFormat,
    parse_collection, parse_environment, parse_request,
};

const V21: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";
const ENV: &str = r#"{"name":"dev","values":[{"key":"host","value":"x.test","enabled":true}],"_postman_variable_scope":"environment"}"#;
const V1: &str = r#"{"id":"x","name":"c","order":[],"requests":[]}"#;

fn postman(name: &str) -> String {
    format!(
        r#"{{"info":{{"name":"{name}","schema":"{V21}"}},"item":[{{"name":"r","request":{{"method":"GET","url":"https://x.test/r"}}}}]}}"#
    )
}

#[test]
fn formats_and_note_kinds_have_stable_ids() {
    assert_eq!(&CollectionFormat::ALL[..1], [CollectionFormat::Postman]);
    assert_eq!(&EnvironmentFormat::ALL[..1], [EnvironmentFormat::Postman]);
    assert_eq!(
        RequestFormat::ALL
            .iter()
            .map(|f| f.id())
            .collect::<Vec<_>>(),
        ["curl"]
    );
    assert_eq!(
        CollectionFormat::from_id("postman"),
        Some(CollectionFormat::Postman)
    );
    assert_eq!(
        EnvironmentFormat::from_id("postman"),
        Some(EnvironmentFormat::Postman)
    );
    assert_eq!(RequestFormat::from_id("curl"), Some(RequestFormat::Curl));
    assert_eq!(RequestFormat::from_id("wget"), None);
    for label in [
        CollectionFormat::Postman.label(),
        EnvironmentFormat::Postman.label(),
        RequestFormat::Curl.label(),
    ] {
        assert!(!label.to_lowercase().contains("bruno"), "{label}");
    }
    let kinds = [
        NoteKind::Dropped,
        NoteKind::Converted,
        NoteKind::Renamed,
        NoteKind::NeedsInput,
    ];
    assert_eq!(
        kinds.map(NoteKind::id),
        ["dropped", "converted", "renamed", "needs-input"]
    );
}

#[test]
fn a_postman_collection_is_detected_bare_enveloped_or_v1() {
    let enveloped = format!(r#"{{"collection":{}}}"#, postman("c"));
    for input in [postman("c"), enveloped, V1.to_owned()] {
        assert_eq!(
            CollectionFormat::detect(input.as_bytes()),
            Some(CollectionFormat::Postman),
            "{input}"
        );
    }
    for other in [
        "",
        "not json",
        "[]",
        r#"{"requests":[]}"#,
        r#"{"info":{"name":"c"}}"#,
        ENV,
    ] {
        assert_eq!(CollectionFormat::detect(other.as_bytes()), None, "{other}");
    }
}

#[test]
fn parsing_by_detection_matches_parsing_by_name() {
    let json = postman("c");
    let auto = parse_collection(None, json.as_bytes()).expect("detected");
    let named = parse_collection(Some(CollectionFormat::Postman), json.as_bytes()).expect("parsed");
    assert_eq!(auto.name, "c");
    assert_eq!(format!("{auto:?}"), format!("{named:?}"));
}

#[test]
fn an_unrecognized_input_names_the_supported_formats() {
    let e = parse_collection(None, br#"{"other":1}"#).unwrap_err();
    assert!(
        matches!(
            e,
            FormatError::Unrecognized {
                kind: "collection",
                ..
            }
        ),
        "{e:?}"
    );
    assert!(
        e.to_string().contains(CollectionFormat::Postman.label()),
        "{e}"
    );
    let e = parse_environment(None, b"{}").unwrap_err();
    assert!(
        e.to_string()
            .starts_with("not a recognized environment format"),
        "{e}"
    );
    let e = parse_request(None, "GET https://x.test").unwrap_err();
    assert!(e.to_string().contains(RequestFormat::Curl.label()), "{e}");
}

#[test]
fn a_named_format_reports_its_own_errors() {
    let e = parse_collection(None, V1.as_bytes()).unwrap_err();
    assert!(
        matches!(
            e,
            FormatError::Postman(PostmanError::UnsupportedSchema { .. })
        ),
        "{e:?}"
    );
    let e = parse_collection(Some(CollectionFormat::Postman), b"{").unwrap_err();
    assert!(
        matches!(e, FormatError::Postman(PostmanError::Parse { .. })),
        "{e:?}"
    );
    assert!(matches!(
        parse_collection(None, b"{"),
        Err(FormatError::Unrecognized { .. })
    ));
    let e = parse_request(Some(RequestFormat::Curl), "curl 'https://x.test").unwrap_err();
    assert!(matches!(e, FormatError::Curl(_)), "{e:?}");
}

#[test]
fn a_postman_environment_is_detected_and_globals_are_not() {
    assert_eq!(
        EnvironmentFormat::detect(ENV.as_bytes()),
        Some(EnvironmentFormat::Postman)
    );
    let unscoped = r#"{"name":"dev","values":[]}"#;
    assert_eq!(
        EnvironmentFormat::detect(unscoped.as_bytes()),
        Some(EnvironmentFormat::Postman)
    );
    let globals = r#"{"name":"g","values":[],"_postman_variable_scope":"globals"}"#;
    assert_eq!(EnvironmentFormat::detect(globals.as_bytes()), None);
    assert_eq!(EnvironmentFormat::detect(postman("c").as_bytes()), None);
    assert_eq!(
        parse_environment(None, ENV.as_bytes())
            .expect("parsed")
            .name,
        "dev"
    );
}

#[test]
fn a_curl_command_is_detected_by_its_first_word() {
    for text in [
        "curl https://x.test",
        "  \n curl\t'https://x.test'",
        "/usr/bin/curl https://x.test",
    ] {
        assert_eq!(
            RequestFormat::detect(text),
            Some(RequestFormat::Curl),
            "{text:?}"
        );
    }
    for text in [
        "https://x.test",
        "curly https://x.test",
        "curl",
        "wget https://x.test",
        "",
    ] {
        assert_eq!(RequestFormat::detect(text), None, "{text:?}");
    }
    let parsed = parse_request(None, "curl -X POST https://x.test/a -k").expect("parsed");
    assert_eq!(
        (parsed.request.method.as_str(), parsed.request.url.as_str()),
        ("POST", "https://x.test/a")
    );
    assert!(
        parsed.notes.iter().any(|n| n.kind == NoteKind::Dropped),
        "{:?}",
        parsed.notes
    );
}

/// The Postman parse offset of `input`, parsed as a named collection or environment.
fn postman_offset(input: &[u8], environment: bool) -> usize {
    let e = if environment {
        parse_environment(Some(EnvironmentFormat::Postman), input).unwrap_err()
    } else {
        parse_collection(Some(CollectionFormat::Postman), input).unwrap_err()
    };
    match e {
        FormatError::Postman(PostmanError::Parse { offset, .. }) => offset,
        e => panic!("expected a Postman parse error, got {e:?}"),
    }
}

#[test]
fn a_postman_parse_offset_indexes_the_input_as_given_bom_included() {
    let bad = br#"{"name":"dev",}"#;
    let with_bom = [b"\xEF\xBB\xBF".as_slice(), bad].concat();
    for environment in [false, true] {
        assert_eq!(postman_offset(bad, environment), 14, "env {environment}");
        assert_eq!(
            postman_offset(&with_bom, environment),
            17,
            "env {environment}"
        );
    }
}
