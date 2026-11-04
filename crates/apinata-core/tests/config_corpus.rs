//! The folder, collection and environment corpus is complete and every case is claimed.

use crate::config_parity;

use config_parity::{all_cases, expected};
use std::collections::BTreeSet;

/// Every prefix the edit tests claim; a case outside them would never be checked.
const PREFIXES: &[&str] = &[
    "folder-name",
    "folder-seq-",
    "folder-docs",
    "folder-lists-",
    "collection-name",
    "collection-docs",
    "collection-lists-",
    "env-vars-",
    "env-color",
    "env-extends-",
    "env-secret-",
    "env-op-",
];

#[test]
fn every_case_has_its_expected_files_and_a_known_prefix() {
    let cases = all_cases();
    assert_eq!(cases.len(), 28);
    let ids: BTreeSet<&str> = cases.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids.len(), cases.len(), "ids are unique");
    for case in &cases {
        assert!(!case.files.is_empty(), "{} writes no file", case.id);
        for file in &case.files {
            assert!(!expected(&case.id, file).is_empty(), "{} {file}", case.id);
        }
        let claimed = PREFIXES.iter().filter(|p| case.id.starts_with(*p)).count();
        assert_eq!(claimed, 1, "{} must match exactly one prefix", case.id);
    }
}
