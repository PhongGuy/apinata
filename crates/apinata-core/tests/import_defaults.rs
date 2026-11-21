//! What an imported collection needs before its first run: variables it references without a
//! value, the environment runs use by default, and hints that paste into a shell.

use crate::import_secrets::import;
use apinata_core::import::{
    CollectionOptions, NoteKind, parse_collections, parse_postman_environment, write_collection,
};
use apinata_core::{
    ContextOptions, EnvironmentChoice, MemoryStore, ResolveError, UndefinedHint, VarContext,
    resolve,
};
use serde_json::Value;

const HOPPSCOTCH: &str = include_str!("fixtures/import-defaults/hoppscotch.json");
const INSOMNIA: &str = include_str!("fixtures/import-defaults/insomnia.json");

/// The Insomnia fixture with every credential removed, so nothing but its base environment
/// can make a default.
fn insomnia_without_credentials() -> String {
    let mut doc: Value = serde_json::from_str(INSOMNIA).expect("fixture parses");
    for resource in doc["resources"].as_array_mut().expect("resources") {
        if let Some(map) = resource.as_object_mut() {
            map.remove("authentication");
        }
    }
    doc.to_string()
}

#[test]
fn a_reference_without_an_imported_value_is_listed_as_needing_one() {
    let (_d, _c, report, _s) = import(HOPPSCOTCH);
    let needs: Vec<_> = report
        .notes
        .iter()
        .filter(|n| n.kind == NoteKind::NeedsInput && n.detail.starts_with("needs a value"))
        .collect();
    assert_eq!(needs.len(), 1, "{:#?}", report.notes);
    assert!(
        needs[0].detail.starts_with("needs a value: `base`;"),
        "{}",
        needs[0].detail
    );
    assert_eq!(needs[0].file, None);
}

#[test]
fn a_reference_an_environment_defines_needs_no_value() {
    let (_d, _c, report, _s) = import(&insomnia_without_credentials());
    let listed: Vec<_> = report
        .notes
        .iter()
        .filter(|n| n.detail.starts_with("needs a value"))
        .map(|n| n.detail.as_str())
        .collect();
    // `host` comes from the base environment, and the template tag's own note asks for `now`.
    assert!(listed.is_empty(), "{listed:?}");
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.detail.contains("give `now` a value before sending")),
        "{:#?}",
        report.notes
    );
}

/// A Postman collection whose one request writes `url`.
fn postman(url: &str) -> String {
    serde_json::json!({
        "info": {
            "name": "P",
            "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
        },
        "item": [{"name": "R", "request": {"method": "GET", "url": url}}]
    })
    .to_string()
}

#[test]
fn an_unknown_dynamic_name_is_not_offered_as_a_collection_variable() {
    let (_d, _c, report, _s) = import(&postman("{{base}}/{{$notADynamicVariable}}"));
    let listed: Vec<_> = report
        .notes
        .iter()
        .filter(|n| n.detail.starts_with("needs a value"))
        .map(|n| n.detail.as_str())
        .collect();
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert!(
        listed[0].starts_with("needs a value: `base`;"),
        "{}",
        listed[0]
    );
}

#[test]
fn a_postman_import_with_several_environments_defaults_to_the_first() {
    let parent = tempfile::TempDir::new().expect("tempdir");
    let bundles = parse_collections(
        None,
        postman("{{base_url}}/x").as_bytes(),
        &CollectionOptions::default(),
    )
    .expect("parses");
    let envs: Vec<_> = ["postman-env.json", "postman-env-2.json"]
        .iter()
        .map(|f| {
            let path = format!(
                "{}/tests/fixtures/postman/environments/{f}",
                env!("CARGO_MANIFEST_DIR")
            );
            parse_postman_environment(&std::fs::read(path).expect("fixture")).expect("parses")
        })
        .collect();
    let (c, report) = write_collection(
        parent.path(),
        &bundles[0].collection,
        &envs,
        &MemoryStore::new(),
    )
    .expect("writes");
    assert_eq!(report.environments, ["Postman Env", "Postman Env Two"]);
    assert_eq!(
        c.choose_environment(None),
        EnvironmentChoice::Preset("Postman Env".into())
    );
}

#[test]
fn the_base_environment_is_the_default_without_any_credentials() {
    let (_d, c, report, _s) = import(&insomnia_without_credentials());
    assert_eq!(report.environments, ["Base Environment"]);
    assert_eq!(
        c.choose_environment(None),
        EnvironmentChoice::Preset("Base Environment".into())
    );
    assert!(
        report.notes.iter().any(|n| n
            .detail
            .contains("runs use the environment `Base Environment` by default")),
        "{:#?}",
        report.notes
    );
}

#[test]
fn an_import_without_environments_records_no_default() {
    let input = r#"{"v": 2, "name": "Plain", "folders": [], "requests": [
        {"v": "1", "name": "Ping", "method": "GET", "endpoint": "http://127.0.0.1:1/ping",
         "params": [], "headers": [], "preRequestScript": "", "testScript": "",
         "auth": {"authType": "none", "authActive": true}, "body": {"contentType": null, "body": null}}
    ], "auth": {"authType": "none", "authActive": true}, "headers": []}"#;
    let (_d, c, report, _s) = import(input);
    assert!(report.environments.is_empty());
    assert_eq!(c.choose_environment(None), EnvironmentChoice::None);
}

#[test]
fn the_run_error_for_an_unset_reference_points_at_a_collection_variable() {
    let (_d, c, _report, _s) = import(HOPPSCOTCH);
    let env = c.choose_environment(None);
    let mut options = ContextOptions::new();
    if let Some(name) = env.name() {
        options = options.env(name);
    }
    let ctx = VarContext::new(&c, options).expect("context");
    let req = c
        .tree()
        .requests()
        .iter()
        .find(|r| r.name() == "Create order")
        .expect("request");
    let err = resolve(req, &ctx).expect_err("base is unset");
    assert!(
        matches!(&err, ResolveError::UndefinedVariable { name, .. } if name == "base"),
        "{err:?}"
    );
    assert_eq!(err.hint(), Some(&UndefinedHint::Undeclared));
    assert_eq!(
        err.cli_hint().as_deref(),
        Some(
            "no environment or collection variable defines base; add it to the collection's variables or pass --env-var base=VALUE"
        )
    );
}

// The expected text is POSIX quoting; Windows quotes with double quotes instead.
#[cfg(unix)]
#[test]
fn hints_quote_names_a_shell_would_split() {
    let hint = UndefinedHint::DefinedElsewhere {
        environments: vec!["Base Environment".into()],
    };
    assert_eq!(
        hint.cli_text("host"),
        "host is defined in environment Base Environment; pass --env 'Base Environment'"
    );
    let secret = UndefinedHint::SecretWithoutValue {
        environment: "dev".into(),
    };
    assert!(
        secret
            .cli_text("my token")
            .contains("pass --env-var 'my token=VALUE'"),
        "{}",
        secret.cli_text("my token")
    );
    assert_eq!(
        UndefinedHint::Undeclared.cli_text("it's"),
        "no environment or collection variable defines it's; add it to the collection's variables or pass --env-var 'it'\\''s=VALUE'"
    );
}

// The expected text is POSIX quoting; Windows quotes with double quotes instead.
#[cfg(unix)]
#[test]
fn shell_words_stay_bare_when_safe() {
    assert_eq!(apinata_core::shell_word("dev"), "dev");
    assert_eq!(apinata_core::shell_word("a/b-c_d.e:f=g"), "a/b-c_d.e:f=g");
    assert_eq!(apinata_core::shell_word(""), "''");
    assert_eq!(apinata_core::shell_word("$HOME"), "'$HOME'");
}
