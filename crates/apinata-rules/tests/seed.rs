//! The seed against the rule: a new workspace's own files pass its own check.

use apinata_rules::RULE_SET;

use crate::common::{ENV, unparsable, verdict};

#[test]
fn the_seed_collection_passes_the_kind_rule() {
    let seed = RULE_SET.seed();
    let collection = seed
        .iter()
        .find(|f| f.path() == "opencollection.yml")
        .unwrap();
    assert_eq!(verdict("opencollection.yml", collection.bytes()), None);
    assert_eq!(
        verdict(ENV, collection.bytes()),
        None,
        "it reads as an environment file too"
    );
}

#[test]
fn the_seed_gitignore_is_not_an_environment_file() {
    let seed = RULE_SET.seed();
    let gitignore = seed.iter().find(|f| f.path() == ".gitignore").unwrap();
    assert_eq!(verdict(".gitignore", gitignore.bytes()), None);
    assert_eq!(
        verdict(ENV, gitignore.bytes()),
        unparsable(ENV),
        "a scalar root, refused under an environment path"
    );
}
