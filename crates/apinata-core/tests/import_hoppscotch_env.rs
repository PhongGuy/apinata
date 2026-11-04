//! Hoppscotch environment exports.

use apinata_core::import::{
    CollectionFormat, CollectionOptions, DocumentError, EnvironmentFormat, FormatError,
    ImportedEnvironment, NoteKind, parse_collections, parse_environments, write_collection,
};
use apinata_core::{EnvVariable, MemoryStore, SecretStore, VarValue};
use serde_json::{Value, json};

fn envs(input: Value) -> Vec<ImportedEnvironment> {
    let bytes = serde_json::to_vec(&input).expect("serialise");
    parse_environments(Some(EnvironmentFormat::Hoppscotch), &bytes).expect("parses")
}

fn plain(name: &str, value: &str) -> EnvVariable {
    EnvVariable::new(name, VarValue::Text(value.into()))
}

#[test]
fn each_version_reads_its_value_member() {
    let v0 = json!({"name": "v0", "variables": [{"key": "a", "value": "1"}]});
    let v1 = json!({"v": 1, "id": "x", "name": "v1", "variables": [{"key": "a", "value": "1", "secret": false}, {"key": "s", "secret": true}]});
    let v2 = json!({"v": 2, "id": "y", "name": "v2", "variables": [
        {"key": "a", "initialValue": "1", "currentValue": "9", "secret": false},
        {"key": "b", "initialValue": "", "currentValue": "2", "secret": false},
        {"key": "n", "initialValue": 7, "currentValue": "", "secret": false},
        {"key": "s", "initialValue": "", "currentValue": "", "secret": true}]});
    assert_eq!(envs(v0)[0].variables, [plain("a", "1")]);
    assert_eq!(
        envs(v1)[0].variables,
        [plain("a", "1"), EnvVariable::secret("s")]
    );
    let got = envs(v2);
    assert_eq!(got[0].name, "v2");
    assert_eq!(
        got[0].variables,
        [
            plain("a", "1"),
            plain("b", "2"),
            plain("n", "7"),
            EnvVariable::secret("s")
        ]
    );
    let got_notes: Vec<(&str, NoteKind)> = got[0]
        .notes
        .iter()
        .map(|n| (n.source.as_str(), n.kind))
        .collect();
    assert_eq!(
        got_notes,
        [("variables[0].currentValue", NoteKind::Dropped)]
    );
}

#[test]
fn an_array_gives_every_environment_and_a_nameless_one_is_global() {
    let got = envs(json!([{"v": 0, "name": "a", "variables": []},
        {"v": 2, "variables": [{"key": "g", "initialValue": "<<x>>", "currentValue": "", "secret": false}]}]));
    let names: Vec<&str> = got.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["a", "Global"]);
    assert_eq!(got[1].variables, [plain("g", "{{x}}")]);
    assert_eq!(envs(json!({"v": 1, "variables": []}))[0].name, "Global");
}

#[test]
fn a_newer_version_reads_as_version_2_with_a_note() {
    let got = envs(
        json!({"v": 3, "name": "n", "variables": [{"key": "a", "initialValue": "1", "currentValue": "", "secret": false}]}),
    );
    assert_eq!(got[0].variables, [plain("a", "1")]);
    assert!(
        got[0]
            .notes
            .iter()
            .any(|n| n.kind == NoteKind::Converted && n.source == "v"),
        "{:?}",
        got[0].notes
    );
}

#[test]
fn input_that_is_not_an_environment_export_is_invalid() {
    for input in [
        &b"[]"[..],
        b"[1]",
        b"\"x\"",
        br#"{"name":"x"}"#,
        br#"{"name":"x","values":[]}"#,
    ] {
        let e = parse_environments(Some(EnvironmentFormat::Hoppscotch), input).unwrap_err();
        assert!(
            matches!(e, FormatError::Hoppscotch(DocumentError::Invalid { .. })),
            "{e:?}"
        );
    }
}

#[test]
fn only_unknown_dynamic_variables_in_values_need_input() {
    let got = envs(json!([{"name": "a", "variables": [
        {"key": "id", "value": "<<$guid>>"},
        {"key": "odd", "value": "<<$guid>><<$nope>>"}]}]));
    assert_eq!(
        got[0].variables,
        [plain("id", "{{$guid}}"), plain("odd", "{{$guid}}{{$nope}}")]
    );
    let needs: Vec<_> = got[0]
        .notes
        .iter()
        .filter(|n| n.kind == NoteKind::NeedsInput)
        .collect();
    assert!(
        needs.iter().all(|n| !n.detail.contains("guid")),
        "{needs:?}"
    );
    assert!(
        needs
            .iter()
            .any(|n| n.source == "[0].variables[1]" && n.detail.contains("$nope")),
        "{needs:?}"
    );
    assert!(
        needs.iter().all(|n| n.source != "[0].variables[0]"),
        "{needs:?}"
    );
}

#[test]
fn secrets_are_reported_by_their_source_when_written() {
    let env = envs(json!([{"name": "a", "variables": []},
        {"name": "dev", "variables": [{"key": "host", "value": "x"}, {"key": "token", "secret": true}]}])).remove(1);
    let input = br#"{"v":12,"name":"c","folders":[],"requests":[]}"#;
    let coll = parse_collections(
        Some(CollectionFormat::Hoppscotch),
        input,
        &CollectionOptions::default(),
    )
    .expect("collection")
    .remove(0)
    .collection;
    let dir = tempfile::TempDir::new().expect("tempdir");
    let (_, report) =
        write_collection(dir.path(), &coll, &[env], &MemoryStore::new()).expect("writes");
    assert_eq!(
        report.secrets_to_enter,
        [("dev".to_owned(), "token".to_owned())]
    );
    let found = report
        .notes
        .iter()
        .any(|n| n.kind == NoteKind::NeedsInput && n.source == "[1].variables[1]");
    assert!(found, "{:?}", report.notes);
    let text =
        std::fs::read_to_string(dir.path().join("c/environments/dev.yml")).expect("env file");
    assert!(text.contains("secret: true"), "{text}");
}

#[test]
fn unknown_environment_keys_are_reported_and_exempt_ones_are_not() {
    let input = json!({"v": 2, "id": "i", "name": "e", "zzApinataProbe": 1,
        "variables": [{"key": "a", "initialValue": "1", "currentValue": "", "secret": false, "zzApinataProbe": 1}]});
    let env = envs(input).remove(0);
    let sources: Vec<&str> = env
        .notes
        .iter()
        .filter(|n| n.kind == NoteKind::Dropped)
        .map(|n| n.source.as_str())
        .collect();
    assert_eq!(sources, ["zzApinataProbe", "variables[0].zzApinataProbe"]);
}

#[test]
fn a_secret_with_a_value_goes_to_the_secret_store() {
    let env = envs(json!({"v": 2, "name": "dev", "variables": [
        {"key": "token", "initialValue": "<<seed>>-s3cret", "currentValue": "", "secret": true}]}))
    .remove(0);
    assert!(env.variables[0].secret);
    assert!(env.notes.is_empty(), "{:?}", env.notes);
    let input = br#"{"v":12,"name":"c","folders":[],"requests":[]}"#;
    let coll = parse_collections(
        Some(CollectionFormat::Hoppscotch),
        input,
        &CollectionOptions::default(),
    )
    .expect("collection")
    .remove(0)
    .collection;
    let dir = tempfile::TempDir::new().expect("tempdir");
    let store = MemoryStore::new();
    let (collection, report) = write_collection(dir.path(), &coll, &[env], &store).expect("writes");
    assert!(
        report.secrets_to_enter.is_empty(),
        "{:?}",
        report.secrets_to_enter
    );
    let key = collection.secret_key("dev", "token");
    assert_eq!(
        store.get(&key).expect("reads"),
        Some("{{seed}}-s3cret".into())
    );
    let text =
        std::fs::read_to_string(dir.path().join("c/environments/dev.yml")).expect("env file");
    assert!(!text.contains("s3cret"), "{text}");
}

#[test]
fn a_variable_without_a_name_is_dropped_with_a_note() {
    let env = envs(json!({"name": "e", "variables": [{"value": "1"}, {"key": "", "value": "2"}, {"key": "a", "value": "3"}]})).remove(0);
    assert_eq!(env.variables, [plain("a", "3")]);
    let sources: Vec<&str> = env
        .notes
        .iter()
        .filter(|n| n.kind == NoteKind::Dropped)
        .map(|n| n.source.as_str())
        .collect();
    assert_eq!(sources, ["variables[0]", "variables[1]"]);
}

#[test]
fn a_differing_current_value_is_noted_in_an_environment() {
    let env = envs(json!({"v": 2, "name": "e", "variables": [
        {"key": "a", "initialValue": "1", "currentValue": "9", "secret": true},
        {"key": "b", "initialValue": "2", "currentValue": "", "secret": false}]}))
    .remove(0);
    let got: Vec<(&str, NoteKind)> = env
        .notes
        .iter()
        .map(|n| (n.source.as_str(), n.kind))
        .collect();
    assert_eq!(got, [("variables[0].currentValue", NoteKind::Dropped)]);
}
