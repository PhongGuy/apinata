//! Structural JSON comparison.

use serde_json::{Number, Value};

use super::*;

/// Changes kept before the rest are only counted.
const MAX_CHANGES: usize = 1000;

/// Compares two JSON texts by structure: keys by name, arrays by index, numbers by value.
/// Returns `None` when either side is not valid JSON, so the caller can fall back to text.
pub(super) fn compare_json(old: &str, new: &str) -> Option<BodyComparison> {
    let old: Value = serde_json::from_str(old).ok()?;
    let new: Value = serde_json::from_str(new).ok()?;
    let mut walk = Walk::default();
    walk.node("$", &old, &new);
    if walk.changes.is_empty() && walk.overflow == 0 && !walk.order_differs {
        return Some(BodyComparison::Identical);
    }
    let order_only = walk.changes.is_empty() && walk.overflow == 0;
    Some(BodyComparison::Json {
        changes: walk.changes,
        truncated_changes: walk.overflow,
        order_only,
    })
}

/// The walker's output: kept changes, the overflow count and whether any object reordered keys.
#[derive(Default)]
struct Walk {
    changes: Vec<JsonChange>,
    overflow: usize,
    order_differs: bool,
}

impl Walk {
    fn push(&mut self, path: &str, kind: JsonChangeKind, old: Option<&Value>, new: Option<&Value>) {
        if self.changes.len() >= MAX_CHANGES {
            self.overflow += 1;
            return;
        }
        let render = |v: &Value| serde_json::to_string(v).unwrap_or_default();
        self.changes.push(JsonChange {
            path: path.to_owned(),
            kind,
            old: old.map(render),
            new: new.map(render),
        });
    }

    fn node(&mut self, path: &str, old: &Value, new: &Value) {
        match (old, new) {
            (Value::Object(a), Value::Object(b)) => {
                // Same keys in a different order is not a change, but it is worth reporting.
                if a.len() == b.len()
                    && a.keys().all(|k| b.contains_key(k))
                    && a.keys().ne(b.keys())
                {
                    self.order_differs = true;
                }
                for (k, av) in a {
                    let p = child_key(path, k);
                    match b.get(k) {
                        Some(bv) => self.node(&p, av, bv),
                        None => self.push(&p, JsonChangeKind::Removed, Some(av), None),
                    }
                }
                for (k, bv) in b {
                    if !a.contains_key(k) {
                        self.push(&child_key(path, k), JsonChangeKind::Added, None, Some(bv));
                    }
                }
            }
            (Value::Array(a), Value::Array(b)) => {
                for i in 0..a.len().max(b.len()) {
                    let p = format!("{path}[{i}]");
                    match (a.get(i), b.get(i)) {
                        (Some(av), Some(bv)) => self.node(&p, av, bv),
                        (Some(av), None) => self.push(&p, JsonChangeKind::Removed, Some(av), None),
                        (None, Some(bv)) => self.push(&p, JsonChangeKind::Added, None, Some(bv)),
                        (None, None) => {}
                    }
                }
            }
            (Value::Number(a), Value::Number(b)) if numbers_equal(a, b) => {}
            (a, b)
                if std::mem::discriminant(a) == std::mem::discriminant(b)
                    && !matches!(a, Value::Number(_))
                    && a == b => {}
            _ => self.push(path, JsonChangeKind::Changed, Some(old), Some(new)),
        }
    }
}

/// Integers compare exactly; anything else compares as `f64`, so `1` equals `1.0`.
fn numbers_equal(a: &Number, b: &Number) -> bool {
    if let (Some(x), Some(y)) = (a.as_i64(), b.as_i64()) {
        return x == y;
    }
    if let (Some(x), Some(y)) = (a.as_u64(), b.as_u64()) {
        return x == y;
    }
    a.as_f64() == b.as_f64()
}

/// Appends a key as `.key` when it is a plain identifier, else as an escaped `['key']`.
fn child_key(path: &str, key: &str) -> String {
    let mut chars = key.chars();
    let plain = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if plain {
        format!("{path}.{key}")
    } else {
        let escaped = key.replace('\\', "\\\\").replace('\'', "\\'");
        format!("{path}['{escaped}']")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmp(a: &str, b: &str) -> BodyComparison {
        compare_json(a, b).expect("both parse")
    }
    fn only(c: BodyComparison) -> Vec<JsonChange> {
        let BodyComparison::Json { changes, .. } = c else {
            panic!("{c:?}")
        };
        changes
    }

    #[test]
    fn whitespace_only_is_identical() {
        assert_eq!(
            cmp("{\"a\":1}", "{ \"a\" : 1 }\n"),
            BodyComparison::Identical
        );
    }
    #[test]
    fn one_equals_one_point_zero() {
        assert_eq!(cmp("{\"a\":1}", "{\"a\":1.0}"), BodyComparison::Identical);
    }
    #[test]
    fn key_order_alone_is_reported_as_order_only() {
        assert_eq!(
            cmp("{\"a\":1,\"b\":2}", "{\"b\":2,\"a\":1}"),
            BodyComparison::Json {
                changes: vec![],
                truncated_changes: 0,
                order_only: true
            }
        );
    }
    #[test]
    fn a_changed_leaf_has_a_jsonpath_and_compact_values() {
        let c = only(cmp(
            r#"{"items":[{"id":1},{"id":2},{"id":3}]}"#,
            r#"{"items":[{"id":1},{"id":2},{"id":4}]}"#,
        ));
        assert_eq!(c.len(), 1);
        assert_eq!(
            (
                c[0].path.as_str(),
                c[0].kind,
                c[0].old.as_deref(),
                c[0].new.as_deref()
            ),
            (
                "$.items[2].id",
                JsonChangeKind::Changed,
                Some("3"),
                Some("4")
            )
        );
    }
    #[test]
    fn an_added_parent_is_one_change() {
        let c = only(cmp(r#"{"a":1}"#, r#"{"a":1,"b":{"x":1,"y":[2]}}"#));
        assert_eq!(c.len(), 1);
        assert_eq!(
            (
                c[0].path.as_str(),
                c[0].kind,
                c[0].old.as_deref(),
                c[0].new.as_deref()
            ),
            (
                "$.b",
                JsonChangeKind::Added,
                None,
                Some(r#"{"x":1,"y":[2]}"#)
            )
        );
    }
    #[test]
    fn a_removed_key_and_a_longer_array() {
        let c = only(cmp(r#"{"a":1,"l":[1]}"#, r#"{"l":[1,2]}"#));
        let got: Vec<_> = c.iter().map(|c| (c.path.as_str(), c.kind)).collect();
        assert_eq!(
            got,
            [
                ("$.a", JsonChangeKind::Removed),
                ("$.l[1]", JsonChangeKind::Added)
            ]
        );
    }
    #[test]
    fn a_type_change_is_one_changed_node() {
        let c = only(cmp(r#"{"a":{"x":1}}"#, r#"{"a":"s"}"#));
        assert_eq!(
            (c.len(), c[0].path.as_str(), c[0].kind),
            (1, "$.a", JsonChangeKind::Changed)
        );
    }
    #[test]
    fn odd_keys_use_bracket_paths() {
        let c = only(cmp(r#"{"a b":1,"it's":1}"#, r#"{"a b":2,"it's":2}"#));
        let paths: Vec<_> = c.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(paths, ["$['a b']", "$['it\\'s']"]);
    }
    #[test]
    fn a_scalar_root_compares() {
        let c = only(cmp("1", "2"));
        assert_eq!(
            (c[0].path.as_str(), c[0].old.as_deref(), c[0].new.as_deref()),
            ("$", Some("1"), Some("2"))
        );
    }
    #[test]
    fn duplicate_keys_keep_the_last() {
        assert_eq!(
            cmp(r#"{"a":1,"a":2}"#, r#"{"a":2}"#),
            BodyComparison::Identical
        );
    }
    #[test]
    fn order_plus_a_change_is_not_order_only() {
        let found = cmp(r#"{"a":1,"b":2}"#, r#"{"b":3,"a":1}"#);
        let BodyComparison::Json {
            changes,
            order_only,
            ..
        } = found
        else {
            panic!()
        };
        assert_eq!((changes.len(), order_only), (1, false));
    }
    #[test]
    fn changes_past_the_cap_are_counted() {
        let old = format!("[{}]", vec!["0"; 1500].join(","));
        let new = format!("[{}]", vec!["1"; 1500].join(","));
        let BodyComparison::Json {
            changes,
            truncated_changes,
            ..
        } = cmp(&old, &new)
        else {
            panic!()
        };
        assert_eq!((changes.len(), truncated_changes), (1000, 500));
    }
    #[test]
    fn invalid_json_on_either_side_gives_none() {
        assert!(compare_json("{", "{}").is_none());
        assert!(compare_json("{}", "nope").is_none());
    }
}
