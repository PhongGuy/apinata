//! Bruno's environment inheritance (`extends`) and its flattening into one variable map.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::Value;

use crate::collection::Environment;
use crate::js;

/// One environment variable's value after inheritance.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum EnvValue {
    /// A value read from the file.
    Plain(Value),
    /// A secret row; its value must come from `--env-var`.
    Secret,
}

/// The selected environment flattened with its ancestors, plus warnings about broken chains.
#[derive(Debug, Default)]
pub(crate) struct ResolvedEnv {
    pub(crate) vars: BTreeMap<String, EnvValue>,
    /// Each secret name in `vars` mapped to the environment whose row won the merge.
    pub(crate) secret_owners: BTreeMap<String, String>,
    pub(crate) warnings: Vec<String>,
}

/// One usable row of an environment's `variables` list.
#[derive(Debug)]
struct Row {
    /// The environment this row was read from.
    owner: String,
    name: String,
    enabled: bool,
    value: EnvValue,
}

/// Resolves environment `name` (a file stem) with its `extends` ancestors; `None` if no such file.
pub(crate) fn resolve_environment(envs: &[Environment], name: &str) -> Option<ResolvedEnv> {
    let target = envs.iter().find(|e| e.name() == name)?;
    let mut warnings = Vec::new();
    let ancestors = ancestors(envs, target, &mut warnings);

    // Ancestors fill one map per kind, nearest last; an own enabled row hides the inherited
    // row of its own kind only, so a plain row cannot hide an inherited secret.
    let mut plain: Vec<Row> = Vec::new();
    let mut secret: Vec<Row> = Vec::new();
    for env in &ancestors {
        for row in rows(env).into_iter().filter(|r| r.enabled) {
            let map = if row.value == EnvValue::Secret {
                &mut secret
            } else {
                &mut plain
            };
            upsert(map, row);
        }
    }
    let own = rows(target);
    for row in own.iter().filter(|r| r.enabled) {
        let map = if row.value == EnvValue::Secret {
            &mut secret
        } else {
            &mut plain
        };
        map.retain(|r| r.name != row.name);
    }

    // Port of toVariablesMap: a secret anywhere in the list wins over plain rows of its name.
    let mut vars = BTreeMap::new();
    let mut secret_owners = BTreeMap::new();
    let mut secret_names = HashSet::new();
    for row in plain.into_iter().chain(secret).chain(own) {
        if !row.enabled {
            continue;
        }
        if row.value == EnvValue::Secret {
            secret_names.insert(row.name.clone());
            secret_owners.insert(row.name.clone(), row.owner);
        } else if secret_names.contains(&row.name) {
            continue;
        }
        vars.insert(row.name, row.value);
    }
    Some(ResolvedEnv {
        vars,
        secret_owners,
        warnings,
    })
}

/// The names of `env`'s own usable rows in file order, repeats kept once.
pub(crate) fn own_names(env: &Environment) -> Vec<String> {
    let mut seen = HashSet::new();
    rows(env)
        .into_iter()
        .filter(|r| seen.insert(r.name.clone()))
        .map(|r| r.name)
        .collect()
}

/// Inserts like a JavaScript `Map.set`: an existing name keeps its position and takes the new row.
fn upsert(map: &mut Vec<Row>, row: Row) {
    match map.iter_mut().find(|r| r.name == row.name) {
        Some(existing) => *existing = row,
        None => map.push(row),
    }
}

/// Port of `getInheritedEnvironments`: the `extends` chain, root first, stopping at a missing
/// parent or at a name already walked, each reported as a warning.
fn ancestors<'a>(
    envs: &'a [Environment],
    target: &'a Environment,
    warnings: &mut Vec<String>,
) -> Vec<&'a Environment> {
    let index: HashMap<&str, &Environment> = envs.iter().map(|e| (e.name(), e)).collect();
    let mut walked = vec![target.name()];
    let mut found = Vec::new();
    let mut current = target;
    while let Some(parent_name) = extends(current) {
        let Some(parent) = index.get(parent_name.as_str()).copied() else {
            warnings.push(format!(
                "environment `{}` extends `{parent_name}`, which does not exist",
                current.name()
            ));
            break;
        };
        if let Some(start) = walked.iter().position(|n| *n == parent.name()) {
            let mut path = walked[start..].to_vec();
            path.push(parent.name());
            warnings.push(format!(
                "environment inheritance cycle: {}",
                path.join(" -> ")
            ));
            break;
        }
        walked.push(parent.name());
        found.push(parent);
        current = parent;
    }
    found.reverse();
    found
}

/// The trimmed `extends` name, when it is a non-empty string.
fn extends(env: &Environment) -> Option<String> {
    match env.document().get(&["extends"])? {
        yaml_serde::Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_owned()),
        _ => None,
    }
}

/// The rows of `variables` that have a name and, unless secret, a usable value.
fn rows(env: &Environment) -> Vec<Row> {
    let Some(yaml_serde::Value::Sequence(items)) = env.document().get(&["variables"]) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let name = js::yaml_to_string(item.get("name")?)?;
            if name.is_empty() {
                return None;
            }
            let flag = |key: &str| item.get(key).and_then(yaml_serde::Value::as_bool) == Some(true);
            let value = if flag("secret") {
                EnvValue::Secret
            } else {
                EnvValue::Plain(js::variable_value(item.get("value"))?)
            };
            Some(Row {
                owner: env.name().to_owned(),
                name,
                enabled: !flag("disabled"),
                value,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Collection;
    use serde_json::json;
    use std::path::Path;

    fn plain(value: serde_json::Value) -> EnvValue {
        EnvValue::Plain(value)
    }

    fn fixture() -> Collection {
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bruno/env-inheritance");
        Collection::open(root).expect("opens")
    }

    #[test]
    fn dev_inherits_from_base_like_bruno() {
        let c = fixture();
        let env = resolve_environment(c.environments(), "dev").expect("dev exists");
        assert!(env.warnings.is_empty(), "{:?}", env.warnings);
        let expected: BTreeMap<String, EnvValue> = [
            ("host", plain(json!("http://localhost:8081"))),
            ("base_only", plain(json!("base_only_value"))),
            ("api_key", plain(json!("plain_api_key"))),
            ("session_id", plain(json!("base_session"))),
            ("shadowed_by_disabled", plain(json!("from_base"))),
            ("overridden_plain", EnvValue::Secret),
            ("base_number", plain(json!(42.0))),
            ("base_boolean", plain(json!(true))),
            ("base_object", plain(json!({"region": "eu"}))),
            ("base_token", EnvValue::Secret),
            ("base_secret_object", EnvValue::Secret),
            ("overridden_secret", EnvValue::Secret),
            ("api_url", plain(json!("https://dev.example.com"))),
            ("dev_only", plain(json!("dev_only_value"))),
            ("typed_override", plain(json!("overridden_in_dev"))),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        assert_eq!(env.vars, expected);
    }

    #[test]
    fn a_secret_row_anywhere_in_the_chain_wins() {
        let c = fixture();
        let staging = resolve_environment(c.environments(), "staging").expect("staging");
        assert_eq!(staging.vars.get("api_key"), Some(&EnvValue::Secret));
        assert_eq!(staging.vars.get("typed_override"), Some(&plain(json!(1.0))));
        let qa = resolve_environment(c.environments(), "qa").expect("qa");
        assert_eq!(qa.vars.get("api_key"), Some(&EnvValue::Secret));
        assert_eq!(
            qa.vars.get("api_url"),
            Some(&plain(json!("https://qa.example.com")))
        );
        assert_eq!(
            qa.vars.get("base_only"),
            Some(&plain(json!("base_only_value")))
        );
    }

    #[test]
    fn broken_chains_keep_the_ancestors_found_and_warn() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        std::fs::write(
            dir.path().join("opencollection.yml"),
            "opencollection: \"1.0.0\"\ninfo:\n  name: t\n",
        )
        .expect("root");
        std::fs::create_dir(dir.path().join("environments")).expect("mkdir");
        let envs = [
            (
                "a",
                "name: a\nextends: b\nvariables:\n  - name: from_a\n    value: a\n",
            ),
            (
                "b",
                "name: b\nextends: a\nvariables:\n  - name: from_b\n    value: b\n",
            ),
            (
                "lost",
                "name: lost\nextends: nope\nvariables:\n  - name: own\n    value: o\n",
            ),
            ("spaced", "name: spaced\nextends: \" b \"\nvariables: []\n"),
            ("listed", "name: listed\nextends: [b]\nvariables: []\n"),
        ];
        for (name, text) in envs {
            std::fs::write(
                dir.path().join("environments").join(format!("{name}.yml")),
                text,
            )
            .expect("env");
        }
        let c = Collection::open(dir.path()).expect("opens");
        let a = resolve_environment(c.environments(), "a").expect("a");
        assert_eq!(a.vars.get("from_a"), Some(&plain(json!("a"))));
        assert_eq!(a.vars.get("from_b"), Some(&plain(json!("b"))));
        assert!(
            a.warnings.iter().any(|w| w.contains("a -> b -> a")),
            "{:?}",
            a.warnings
        );
        let lost = resolve_environment(c.environments(), "lost").expect("lost");
        assert_eq!(lost.vars.len(), 1);
        assert!(
            lost.warnings.iter().any(|w| w.contains("nope")),
            "{:?}",
            lost.warnings
        );
        let spaced = resolve_environment(c.environments(), "spaced").expect("spaced");
        assert!(spaced.vars.contains_key("from_b"));
        let listed = resolve_environment(c.environments(), "listed").expect("listed");
        assert!(listed.vars.is_empty());
        assert!(resolve_environment(c.environments(), "missing").is_none());
    }
}
