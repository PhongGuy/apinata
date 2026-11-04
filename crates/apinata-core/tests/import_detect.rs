//! Detection of the new formats, the tightened Postman rules, and named-format errors.

use apinata_core::import::{
    CollectionFormat as C, DocumentError, EnvironmentFormat as E, FormatError, parse_collection,
    parse_environment,
};

fn coll(input: &str) -> Option<C> {
    C::detect(input.as_bytes())
}

fn env(input: &str) -> Option<E> {
    E::detect(input.as_bytes())
}

#[test]
fn ids_labels_and_order_cover_every_format() {
    let ids: Vec<&str> = C::ALL.iter().map(|f| f.id()).collect();
    assert_eq!(ids, ["postman", "openapi", "insomnia", "hoppscotch", "har"]);
    assert_eq!(
        E::ALL.iter().map(|f| f.id()).collect::<Vec<_>>(),
        ["postman", "hoppscotch"]
    );
    for f in C::ALL {
        assert_eq!(C::from_id(f.id()), Some(*f));
        assert!(!f.label().to_lowercase().contains("bruno"), "{}", f.label());
    }
    assert_eq!(E::from_id("hoppscotch"), Some(E::Hoppscotch));
    let e = parse_collection(None, b"{\"other\":1}").unwrap_err();
    for f in C::ALL {
        assert!(e.to_string().contains(f.label()), "{e}");
    }
}

#[test]
fn each_new_collection_format_is_detected_from_json_or_yaml() {
    let cases = [
        (r#"{"openapi":"3.1.0"}"#, C::OpenApi),
        (r#"{"swagger":"2.0"}"#, C::OpenApi),
        (
            "openapi: 3.0.3\ninfo: {title: t, version: '1'}\npaths: {}\n",
            C::OpenApi,
        ),
        ("openapi: 4.0\n", C::OpenApi),
        (
            r#"{"_type":"export","__export_format":4,"resources":[]}"#,
            C::Insomnia,
        ),
        ("type: collection.insomnia.rest/5.0\nname: c\n", C::Insomnia),
        ("type: spec.insomnia.rest/5.0\n", C::Insomnia),
        ("type: environment.insomnia.rest/5.0\n", C::Insomnia),
        (
            r#"{"v":12,"name":"c","folders":[],"requests":[]}"#,
            C::Hoppscotch,
        ),
        (r#"{"name":"c","folders":[],"requests":[]}"#, C::Hoppscotch),
        (
            r#"[{"name":"a","folders":[],"requests":[]},{"v":2,"name":"b","folders":[],"requests":[]}]"#,
            C::Hoppscotch,
        ),
        (r#"{"log":{"version":"1.2","entries":[]}}"#, C::Har),
    ];
    for (input, want) in cases {
        assert_eq!(coll(input), Some(want), "{input}");
        assert_eq!(env(input), None, "{input}");
    }
}

#[test]
fn near_misses_are_not_detected() {
    for input in [
        r#"{"_type":"export","__export_format":"4"}"#,
        "type: collection.insomnia.rest/4.0\n",
        r#"{"v":"12","name":"c","folders":[],"requests":[]}"#,
        r#"[{"name":"a","folders":[],"requests":[]},{"name":"b","requests":[]}]"#,
        r#"{"requests":[]}"#,
        r#"{"log":{"entries":{}}}"#,
        r#"{"log":{}}"#,
        "[]",
        "{}",
        "null",
        "42",
        "\"text\"",
        "a: 1\n",
        "",
    ] {
        assert_eq!((coll(input), env(input)), (None, None), "{input}");
    }
}

#[test]
fn the_tightened_postman_rules_leave_hoppscotch_files_alone() {
    assert_eq!(
        coll(r#"{"id":"x","name":"c","order":[],"requests":[]}"#),
        Some(C::Postman)
    );
    assert_eq!(
        coll(r#"{"name":"c","folders":[],"requests":[],"order":[]}"#),
        Some(C::Postman)
    );
    assert_eq!(env(r#"{"name":"dev","values":[]}"#), Some(E::Postman));
    assert_eq!(env(r#"{"name":"dev","values":[],"v":"1"}"#), None);
}

#[test]
fn hoppscotch_environments_are_detected_as_objects_or_arrays() {
    for input in [
        r#"{"v":2,"id":"x","name":"dev","variables":[]}"#,
        r#"{"v":1,"variables":[]}"#,
        r#"[{"name":"a","variables":[]},{"name":"b","variables":[]}]"#,
    ] {
        assert_eq!(
            (coll(input), env(input)),
            (None, Some(E::Hoppscotch)),
            "{input}"
        );
    }
    // The last is a Hoppscotch v0 global export, which is not supported.
    for input in [
        r#"{"variables":[]}"#,
        r#"{"name":"a","variables":[],"requests":[]}"#,
        "[]",
        r#"[{"key":"a","value":"b"}]"#,
    ] {
        assert_eq!(env(input), None, "{input}");
    }
}

#[test]
fn a_named_format_wraps_loader_errors_in_its_own_variant() {
    let e = parse_collection(Some(C::OpenApi), b"{").unwrap_err();
    assert!(
        matches!(
            e,
            FormatError::OpenApi(DocumentError::Parse { offset: 0, .. })
        ),
        "{e:?}"
    );
    assert!(
        e.to_string()
            .starts_with("OpenAPI: cannot be read at byte 0"),
        "{e}"
    );
    let e = parse_collection(Some(C::Insomnia), b"a: [1").unwrap_err();
    assert!(
        matches!(e, FormatError::Insomnia(DocumentError::Parse { .. })),
        "{e:?}"
    );
    let e = parse_environment(Some(E::Hoppscotch), b"{").unwrap_err();
    assert!(
        matches!(e, FormatError::Hoppscotch(DocumentError::Parse { .. })),
        "{e:?}"
    );
    for f in [C::OpenApi, C::Insomnia, C::Hoppscotch, C::Har] {
        let text = parse_collection(Some(f), b"{}").unwrap_err().to_string();
        assert!(!text.to_lowercase().contains("bruno"), "{text}");
    }
    let text = parse_environment(Some(E::Hoppscotch), b"{}")
        .unwrap_err()
        .to_string();
    assert!(!text.to_lowercase().contains("bruno"), "{text}");
    let big = vec![b'a'; (16 << 20) + 1];
    let e = parse_collection(Some(C::Har), &big).unwrap_err();
    assert!(
        matches!(
            e,
            FormatError::Har(DocumentError::TooLarge { unit: "bytes", .. })
        ),
        "{e:?}"
    );
}
