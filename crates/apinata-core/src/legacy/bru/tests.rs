//! Corpus-driven parser tests: each manifest group's inputs, and their CRLF and BOM variants,
//! parse to the reference parser's JSON or fail on the line the reference reports.

use super::{BruError, parse_bruno_json, parse_collection, parse_environment, parse_request};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

type Parse<'a> = &'a dyn Fn(&str, &str) -> Result<Value, BruError>;

const VARIANTS: [&str; 3] = ["lf", "crlf", "bom"];

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reference/2026-10-14-bru-migration-parity")
}

struct Entry {
    input: String,
    parser: String,
    group: String,
}

fn manifest() -> Vec<Entry> {
    let path = corpus().join("parsed/manifest.json");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let rows: Vec<Value> = serde_json::from_str(&text).expect("manifest is a JSON array");
    rows.iter()
        .map(|row| {
            let field = |key: &str| {
                row[key]
                    .as_str()
                    .unwrap_or_else(|| panic!("manifest row lacks {key}: {row}"))
                    .to_owned()
            };
            Entry {
                input: field("input"),
                parser: field("parser"),
                group: field("group"),
            }
        })
        .collect()
}

fn source(input: &str) -> String {
    let path = corpus().join("sources").join(input);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn variant_text(lf: &str, variant: &str) -> String {
    match variant {
        "lf" => lf.to_owned(),
        "crlf" => lf.replace('\n', "\r\n"),
        "bom" => format!("\u{feff}{lf}"),
        other => panic!("unknown variant {other}"),
    }
}

enum Oracle {
    Parsed(Value),
    Rejected(Option<usize>),
}

fn read_json(path: &Path) -> Option<Value> {
    let text = fs::read_to_string(path).ok()?;
    // Oracles hold lone-surrogate escapes and 1e400-style numbers that serde_json rejects.
    Some(super::json::parse(&text).unwrap_or_else(|e| panic!("{}: {}", path.display(), e.message)))
}

/// A variant's own oracle when the generator wrote one, else the LF oracle.
fn oracle(input: &str, variant: &str) -> Oracle {
    let parsed = corpus().join("parsed");
    let suffixes = if variant == "lf" {
        vec![String::new()]
    } else {
        vec![format!(".{variant}"), String::new()]
    };
    for suffix in suffixes {
        if let Some(value) = read_json(&parsed.join(format!("{input}{suffix}.json"))) {
            return Oracle::Parsed(value);
        }
        if let Some(error) = read_json(&parsed.join(format!("{input}{suffix}.error.json"))) {
            return Oracle::Rejected(error["line"].as_u64().and_then(|l| usize::try_from(l).ok()));
        }
    }
    panic!("no oracle for {input} ({variant})");
}

/// Example-block failures carry the parser's own message text, which is not pinned.
fn scrub_example_errors(value: &mut Value) {
    if let Some(examples) = value.get_mut("examples").and_then(Value::as_array_mut) {
        for example in examples {
            if let Some(error) = example.get_mut("error") {
                *error = Value::String("<example parse error>".into());
            }
        }
    }
}

fn assert_groups_match(groups: &[&str], parse: Parse<'_>) {
    let entries: Vec<Entry> = manifest()
        .into_iter()
        .filter(|e| groups.contains(&e.group.as_str()))
        .collect();
    for group in groups {
        assert!(
            entries.iter().any(|e| e.group == *group),
            "group {group} has no inputs"
        );
    }
    let mut failures = Vec::new();
    for entry in &entries {
        let lf = source(&entry.input);
        for variant in VARIANTS {
            let label = format!("{} [{variant}]", entry.input);
            let text = variant_text(&lf, variant);
            match (
                oracle(&entry.input, variant),
                parse(entry.parser.as_str(), &text),
            ) {
                (Oracle::Parsed(mut want), Ok(mut got)) => {
                    scrub_example_errors(&mut want);
                    scrub_example_errors(&mut got);
                    if want != got {
                        failures.push(format!("{label}: differs\n  want {want}\n  got  {got}"));
                    }
                }
                (Oracle::Parsed(_), Err(e)) => {
                    failures.push(format!(
                        "{label}: rejected at line {}: {}",
                        e.line, e.message
                    ));
                }
                (Oracle::Rejected(_), Ok(_)) => {
                    failures.push(format!("{label}: accepted, the reference rejects it"));
                }
                (Oracle::Rejected(Some(line)), Err(e)) if e.line != line => {
                    failures.push(format!(
                        "{label}: rejected at line {}, reference line {line}",
                        e.line
                    ));
                }
                (Oracle::Rejected(_), Err(_)) => {}
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} parses differ:\n{}",
        failures.len(),
        entries.len() * VARIANTS.len(),
        failures
            .iter()
            .take(15)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// Fails loudly when a task's synthetic coverage file is missing from the corpus.
fn assert_synthetic(group: &str, names: &[&str]) {
    let dir = corpus().join("sources/synthetic").join(group);
    let missing: Vec<&str> = names
        .iter()
        .copied()
        .filter(|n| !dir.join(n).is_file())
        .collect();
    assert!(missing.is_empty(), "synthetic/{group} lacks {missing:?}");
}

#[test]
fn basics_inputs_match_the_reference_parser() {
    assert_synthetic(
        "basics",
        &[
            "meta.bru",
            "meta-defaults.bru",
            "meta-duplicate-key.bru",
            "proto-key.bru",
            "proto-key-list.bru",
            "list-bmp-letters.bru",
            "malformed-list-astral.bru",
            "multiline-straddle-carrier.bru",
            "verb-get.bru",
            "verb-post.bru",
            "verb-put.bru",
            "verb-delete.bru",
            "verb-patch.bru",
            "verb-options.bru",
            "verb-head.bru",
            "verb-connect.bru",
            "verb-trace.bru",
            "http-custom.bru",
            "http-no-method.bru",
            "params.bru",
            "headers.bru",
            "headers-repeated.bru",
            "multiline.bru",
            "vars.bru",
            "assert.bru",
            "scripts-tests-docs.bru",
            "bodies-text.bru",
            "empty-blocks.bru",
            "whitespace.bru",
            "malformed-unknown-block.bru",
            "malformed-unterminated.bru",
            "malformed-no-colon.bru",
            "malformed-indented-close.bru",
            "malformed-list.bru",
            "malformed-trailing-garbage.bru",
            "malformed-key-only.bru",
            "malformed-assert-list.bru",
        ],
    );
    assert_groups_match(&["basics"], &|parser, text| match parser {
        "request" => parse_request(text),
        other => panic!("basics holds only request inputs, not {other}"),
    });
}

#[test]
fn auth_inputs_match_the_reference_parser() {
    assert_synthetic(
        "auth",
        &[
            "awsv4.bru",
            "basic.bru",
            "bearer.bru",
            "digest.bru",
            "ntlm.bru",
            "wsse.bru",
            "apikey.bru",
            "oauth1-text.bru",
            "oauth1-file.bru",
            "oauth1-list.bru",
            "oauth1-private-key-list.bru",
            "malformed-oauth1-private-key-list.bru",
            "oauth2-password.bru",
            "oauth2-authorization-code.bru",
            "oauth2-client-credentials.bru",
            "oauth2-implicit.bru",
            "oauth2-unknown-grant.bru",
            "oauth2-auto-flags.bru",
            "oauth2-flag-json.bru",
            "oauth2-additional-params.bru",
            "edgegrid-number.bru",
            "edgegrid-blank.bru",
            "edgegrid-nan.bru",
            "malformed-auth-mode.bru",
            "malformed-auth-pair.bru",
        ],
    );
    assert_groups_match(&["auth"], &|parser, text| match parser {
        "request" => parse_request(text),
        other => panic!("auth holds only request inputs, not {other}"),
    });
}

#[test]
fn request_inputs_match_the_reference_parser() {
    assert_synthetic(
        "request",
        &[
            "body-form-urlencoded.bru",
            "body-multipart.bru",
            "body-file.bru",
            "body-grpc.bru",
            "body-ws.bru",
            "body-graphql.bru",
            "settings-all.bru",
            "settings-timeout.bru",
            "settings-timeout-garbage.bru",
            "settings-redirects-zero.bru",
            "settings-redirects-exp.bru",
            "settings-redirects-negative.bru",
            "settings-redirects-blank.bru",
            "settings-keepalive.bru",
            "settings-omit-headers-twice.bru",
            "app.bru",
            "grpc.bru",
            "ws.bru",
            "metadata.bru",
            "annotations-description.bru",
            "annotations-typed-pre.bru",
            "annotations-typed-json.bru",
            "annotations-typed-number-list.bru",
            "settings-lists.bru",
            "annotations-typed-json-carrier.bru",
            "settings-keepalive-infinity.bru",
            "settings-keepalive-negative-infinity.bru",
            "annotations-typed-post.bru",
            "annotations-headers-params.bru",
            "annotations-oauth2-additional-params.bru",
            "malformed-annotation-before-close.bru",
            "malformed-annotation-paren.bru",
            "malformed-body-file-list.bru",
        ],
    );
    assert_groups_match(&["request"], &|parser, text| match parser {
        "request" => parse_request(text),
        other => panic!("request holds only request inputs, not {other}"),
    });
}

#[test]
fn example_inputs_match_the_reference_parser() {
    assert_synthetic(
        "examples",
        &[
            "name-description.bru",
            "description-multiline.bru",
            "request-full.bru",
            "response.bru",
            "two-examples.bru",
            "response-header-lists.bru",
            "dictionary-multiline.bru",
            "two-response-sections.bru",
            "empty-example.bru",
            "rejected-example.bru",
        ],
    );
    assert_groups_match(&["examples"], &|parser, text| match parser {
        "request" => parse_request(text),
        other => panic!("examples holds only request inputs, not {other}"),
    });
}

fn parse_any(parser: &str, text: &str) -> Result<Value, BruError> {
    match parser {
        "request" => parse_request(text),
        "collection" => parse_collection(text),
        "environment" => parse_environment(text),
        "bruno_json" => parse_bruno_json(text),
        other => panic!("unknown parser {other}"),
    }
}

#[test]
fn collection_folder_environment_and_marker_inputs_match_the_reference() {
    assert_synthetic(
        "collection",
        &[
            "full/collection.bru",
            "full/bruno.json",
            "folder-a/folder.bru",
            "malformed-collection/collection.bru",
            "malformed-brace-space/collection.bru",
            "malformed-oauth1-private-key-list/collection.bru",
            "environments/plain.bru",
            "environments/multiline.bru",
            "environments/annotations.bru",
            "environments/secret.bru",
            "environments/secret-empty.bru",
            "environments/extends-name.bru",
            "environments/extends-list.bru",
            "environments/extends-invalid.bru",
            "environments/color.bru",
            "environments/externalsecrets.bru",
            "environments/empty.bru",
            "environments/proto-key.bru",
            "proto-key/folder.bru",
            "environments/malformed-secret.bru",
            "bad-json/bruno.json",
            "bruno-json-numbers/bruno.json",
        ],
    );
    assert_groups_match(&["collection"], &parse_any);
}

#[test]
fn invalid_bruno_json_names_its_line() {
    let err = parse_bruno_json("{\n  \"name\": \"x\",\n}\n").unwrap_err();
    assert_eq!(err.line, 3);
}

#[test]
fn whole_collections_match_the_reference() {
    assert_groups_match(&["collections"], &parse_any);
}

#[test]
fn acceptance_agrees_with_the_reference_on_every_input() {
    let mut disagreements = Vec::new();
    let mut rejected = 0;
    for entry in manifest() {
        let lf = source(&entry.input);
        for variant in VARIANTS {
            let ours = parse_any(&entry.parser, &variant_text(&lf, variant)).is_ok();
            let theirs = matches!(oracle(&entry.input, variant), Oracle::Parsed(_));
            if !theirs {
                rejected += 1;
            }
            if ours != theirs {
                disagreements.push(format!(
                    "{} [{variant}]: reference ok={theirs}, ours ok={ours}",
                    entry.input
                ));
            }
        }
    }
    assert!(rejected > 0, "the corpus holds no rejected input");
    assert!(disagreements.is_empty(), "{}", disagreements.join("\n"));
}

fn bru_files(dir: &Path, root: &Path, out: &mut Vec<String>) {
    for entry in fs::read_dir(dir).expect("readable corpus dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            bru_files(&path, root, out);
        } else if path.extension().is_some_and(|e| e == "bru") {
            let rel = path.strip_prefix(root).expect("under root");
            let parts: Vec<String> = rel
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            out.push(parts.join("/"));
        }
    }
}

#[test]
fn the_manifest_lists_every_source_bru_file() {
    let root = corpus().join("sources");
    let mut on_disk = Vec::new();
    bru_files(&root, &root, &mut on_disk);
    on_disk.sort();
    let mut listed: Vec<String> = manifest()
        .into_iter()
        .map(|e| e.input)
        .filter(|i| i.ends_with(".bru"))
        .collect();
    listed.sort();
    assert_eq!(listed, on_disk);
}
