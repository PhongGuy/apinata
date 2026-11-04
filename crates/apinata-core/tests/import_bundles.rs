//! Plural parses, and size limits through dispatch.

use apinata_core::import::{
    CollectionFormat, CollectionOptions, EnvironmentFormat, FormatError, GroupBy, parse_collection,
    parse_collections, parse_environments,
};

const V21: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";
const ENV: &str = r#"{"name":"dev","values":[{"key":"host","value":"x.test","enabled":true}]}"#;

fn postman(name: &str) -> String {
    format!(
        r#"{{"info":{{"name":"{name}","schema":"{V21}"}},"item":[{{"name":"r","request":{{"method":"GET","url":"https://x.test/r"}}}}]}}"#
    )
}

#[test]
fn a_postman_collection_is_one_bundle_without_environments() {
    let json = postman("c");
    let options = CollectionOptions::default();
    let auto = parse_collections(None, json.as_bytes(), &options).expect("parsed");
    assert_eq!(auto.len(), 1);
    assert_eq!(auto[0].collection.name, "c");
    assert!(auto[0].environments.is_empty());
    let named = parse_collections(Some(CollectionFormat::Postman), json.as_bytes(), &options)
        .expect("parsed");
    assert_eq!(format!("{auto:?}"), format!("{named:?}"));
}

#[test]
fn options_default_to_tag_grouping_and_no_name() {
    let options = CollectionOptions::default();
    assert_eq!(
        (options.group_by, options.name.as_deref()),
        (GroupBy::Tags, None)
    );
}

#[test]
fn a_postman_environment_is_one_environment() {
    let envs = parse_environments(None, ENV.as_bytes()).expect("parsed");
    assert_eq!(
        envs.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        ["dev"]
    );
}

#[test]
fn a_byte_order_mark_is_accepted() {
    let json = format!("\u{feff}{}", postman("c"));
    assert_eq!(
        CollectionFormat::detect(json.as_bytes()),
        Some(CollectionFormat::Postman)
    );
    assert_eq!(
        parse_collection(None, json.as_bytes())
            .expect("parsed")
            .name,
        "c"
    );
}

#[test]
fn input_over_the_byte_cap_is_too_large_and_never_detected() {
    let mut big = vec![b' '; (64 << 20) + 1];
    big[0] = b'{';
    assert_eq!(CollectionFormat::detect(&big), None);
    assert_eq!(EnvironmentFormat::detect(&big), None);
    let e = parse_collection(None, &big).unwrap_err();
    assert!(
        matches!(e, FormatError::TooLarge { limit, unit: "bytes" } if limit == 64 << 20),
        "{e:?}"
    );
    assert_eq!(e.to_string(), "the input is larger than 67108864 bytes");
    assert!(matches!(
        parse_environments(None, &big),
        Err(FormatError::TooLarge { .. })
    ));
    let named = parse_collection(Some(CollectionFormat::Postman), &big);
    assert!(
        matches!(named, Err(FormatError::TooLarge { .. })),
        "{named:?}"
    );
}

#[test]
fn input_over_the_value_budget_is_too_large() {
    let zeros = format!("[{}0]", "0,".repeat(2_000_000));
    let e = parse_collection(None, zeros.as_bytes()).unwrap_err();
    assert!(
        matches!(
            e,
            FormatError::TooLarge {
                limit: 2_000_000,
                unit: "values"
            }
        ),
        "{e:?}"
    );
}
