//! Every vendored fixture is detected as its own format and kind only (research L3 and L4).

use apinata_core::import::{CollectionFormat, EnvironmentFormat};
use std::fs;
use std::path::{Path, PathBuf};

/// Fixtures that no rule claims: malformed, non-HAR or non-format files (research L3).
const UNDETECTED: &[&str] = &[
    "har/kong/form-data-input.json",
    "har/kong/minimal-input.json",
    "har/kong/shallow-input.json",
    "hoppscotch/collections/malformed-coll-2.json",
    "hoppscotch/collections/malformed-coll.json",
    "hoppscotch/collections/notjson-coll.txt",
    "hoppscotch/environments/env-flag-envs.json",
    "hoppscotch/environments/malformed-envs.json",
    "insomnia/bruno/insomnia-malformed.json",
    "openapi/bruno/openapi-broken.json",
    "openapi/bruno/openapi-empty.yaml",
    "openapi/bruno/openapi-invalid-extension.txt",
    "openapi/bruno/openapi-malformed.yaml",
    "openapi/bruno/openapi-multifile-endpoint.yaml",
    "postman/collections/postman-invalid-missing-info.json",
    "postman/collections/postman-malformed.json",
    "postman/environments/postman-global-env.json",
];

type Detected = (Option<CollectionFormat>, Option<EnvironmentFormat>);

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("fixture dir") {
        let path = entry.expect("entry").path();
        let name = path
            .file_name()
            .expect("name")
            .to_string_lossy()
            .into_owned();
        if path.is_dir() {
            files(&path, out);
        } else if !matches!(name.as_str(), "NOTICE.md" | "LICENSE" | "LICENSE.md")
            // The spec-update fixtures are this project's own and tested by import_update_*.
            && !name.starts_with("update-")
        {
            out.push(path);
        }
    }
}

#[test]
fn every_fixture_is_detected_as_its_own_format_only() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let sets: [(&str, Detected); 7] = [
        (
            "postman/collections",
            (Some(CollectionFormat::Postman), None),
        ),
        (
            "postman/environments",
            (None, Some(EnvironmentFormat::Postman)),
        ),
        ("openapi", (Some(CollectionFormat::OpenApi), None)),
        ("insomnia", (Some(CollectionFormat::Insomnia), None)),
        (
            "hoppscotch/collections",
            (Some(CollectionFormat::Hoppscotch), None),
        ),
        (
            "hoppscotch/environments",
            (None, Some(EnvironmentFormat::Hoppscotch)),
        ),
        ("har", (Some(CollectionFormat::Har), None)),
    ];
    let (mut seen, mut wrong) = (0, Vec::new());
    for (dir, expected) in sets {
        let mut list = Vec::new();
        files(&root.join(dir), &mut list);
        for path in list {
            let rel = path
                .strip_prefix(&root)
                .expect("under root")
                .to_string_lossy()
                .replace('\\', "/");
            let input = fs::read(&path).expect("read");
            let got = (
                CollectionFormat::detect(&input),
                EnvironmentFormat::detect(&input),
            );
            let want = if UNDETECTED.contains(&rel.as_str()) {
                (None, None)
            } else {
                expected
            };
            if got != want {
                wrong.push(format!("{rel}: {got:?}, want {want:?}"));
            }
            seen += 1;
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
    assert_eq!(
        seen, 155,
        "fixture count changed; update this test and the NOTICE files"
    );
}
