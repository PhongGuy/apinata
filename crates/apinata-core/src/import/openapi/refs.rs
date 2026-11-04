//! Lazy resolution of local `$ref` pointers.
//!
//! A `$ref` is followed at each use rather than inlined, so a document's size bounds the work.
//! Refs that cannot be followed (external, anchors, missing targets, cycles, over-long chains)
//! become `{}` with a note.

use super::bounds::approx_size;
use super::version::Version;
use crate::import::{ImportNote, NoteKind};
use percent_encoding::percent_decode_str;
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::collections::HashSet;

/// Whether a `$ref` stands for a Reference Object or a Schema Object; the two differ in how
/// siblings of the `$ref` are treated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefKind {
    Object,
    Schema,
}

/// A value with its `$ref` chain followed. `followed` lists each hop as a canonical pointer and
/// is empty both for a value without a `$ref` and for a ref that was cut to `{}`.
pub(crate) struct Resolved<'v> {
    pub(crate) value: Cow<'v, Value>,
    pub(crate) followed: Vec<String>,
}

/// Follows local JSON pointers within one document, collecting notes about refs it cut.
pub(crate) struct Resolver<'a> {
    root: &'a Value,
    version: Version,
    /// Pointers of the values a caller is currently walking into, set by `enter` and `leave`.
    stack: Vec<String>,
    notes: Vec<ImportNote>,
    noted: HashSet<(String, String)>,
    /// Approximate bytes of the sibling merges built since the last `take_copied`.
    copied: u64,
}

impl<'a> Resolver<'a> {
    /// The most `$ref` hops one resolution follows before cutting the chain.
    pub(crate) const MAX_CHAIN: usize = 64;

    pub(crate) fn new(root: &'a Value, version: Version) -> Resolver<'a> {
        Resolver {
            root,
            version,
            stack: Vec::new(),
            notes: Vec::new(),
            noted: HashSet::new(),
            copied: 0,
        }
    }

    /// Follow `value`'s `$ref` chain to its target. Values borrowed from the document come
    /// back borrowed unless 3.1 or 3.2 siblings of a `$ref` apply, which gives an owned merge;
    /// a ref that cannot be followed becomes an owned `{}` with a note on `source`.
    pub(crate) fn resolve<'v>(
        &mut self,
        value: &'v Value,
        kind: RefKind,
        source: &str,
    ) -> Resolved<'v>
    where
        'a: 'v,
    {
        self.follow(value, kind, source, true)
    }

    /// `resolve` for a Reference Object without its siblings laid over the target, so the
    /// result is borrowed from the document unless the ref was cut; for walks that only read keys.
    pub(crate) fn target<'v>(&mut self, value: &'v Value, source: &str) -> Resolved<'v>
    where
        'a: 'v,
    {
        self.follow(value, RefKind::Object, source, false)
    }

    fn follow<'v>(
        &mut self,
        value: &'v Value,
        kind: RefKind,
        source: &str,
        merge: bool,
    ) -> Resolved<'v>
    where
        'a: 'v,
    {
        let mut current = value;
        let mut followed = Vec::new();
        // The siblings of each hop's `$ref`, outermost first, applied once the target is known.
        let mut siblings: Vec<&'v Map<String, Value>> = Vec::new();
        // Each hop is checked against the chain cap, the walk stack and this chain before it
        // is taken, so a cycle is cut at its first repeat.
        while let Some(target) = ref_of(current) {
            if followed.len() >= Self::MAX_CHAIN {
                return self.cut(
                    source,
                    NoteKind::Converted,
                    format!(
                        "`$ref` chain at `{target}` is longer than {} hops; replaced with an empty object",
                        Self::MAX_CHAIN
                    ),
                );
            }
            let Some(fragment) = target.strip_prefix('#') else {
                return self.cut(
                    source,
                    NoteKind::Dropped,
                    format!("external `$ref` `{target}` is not followed"),
                );
            };
            if !fragment.is_empty() && !fragment.starts_with('/') {
                return self.cut(
                    source,
                    NoteKind::Dropped,
                    format!("anchor `$ref` `{target}` is not followed"),
                );
            }
            let segments = decode(fragment);
            let pointer = canonical(&segments);
            if self.stack.contains(&pointer) || followed.contains(&pointer) {
                return self.cut(
                    source,
                    NoteKind::Converted,
                    format!(
                        "`$ref` `{target}` refers back to itself; replaced with an empty object"
                    ),
                );
            }
            let Some(next) = walk(self.root, &segments) else {
                return self.cut(
                    source,
                    NoteKind::Dropped,
                    format!("`$ref` `{target}` points at nothing and is not followed"),
                );
            };
            followed.push(pointer);
            if let Some(map) = current.as_object() {
                siblings.push(map);
            }
            current = next;
        }
        if kind == RefKind::Schema
            && let Some(target) = current.get("$dynamicRef")
        {
            let detail = format!("`$dynamicRef` `{}` is not followed", text_of(target));
            self.note(source, NoteKind::Dropped, detail);
        }
        let value = if merge {
            self.merge_siblings(current, &siblings, kind)
        } else {
            Cow::Borrowed(current)
        };
        Resolved { value, followed }
    }

    /// Lay the applicable siblings over `target`, innermost hop first so the outermost wins.
    /// Existing keys keep their place and new ones are appended; with nothing to apply, the
    /// target stays borrowed. The merge's size is added to `copied` for the caller to charge.
    fn merge_siblings<'v>(
        &mut self,
        target: &'v Value,
        siblings: &[&Map<String, Value>],
        kind: RefKind,
    ) -> Cow<'v, Value> {
        let applies = |key: &str| match (self.version, kind) {
            (Version::Swagger2 | Version::V30, _) => false,
            (_, RefKind::Schema) => key != "$ref",
            (_, RefKind::Object) => key == "summary" || key == "description",
        };
        let Some(base) = target.as_object() else {
            return Cow::Borrowed(target);
        };
        if !siblings.iter().any(|map| map.keys().any(|k| applies(k))) {
            return Cow::Borrowed(target);
        }
        self.copied += approx_size(target);
        let mut merged = base.clone();
        for map in siblings.iter().rev() {
            for (key, value) in map.iter().filter(|(k, _)| applies(k)) {
                self.copied += key.len() as u64 + approx_size(value);
                merged.insert(key.clone(), value.clone());
            }
        }
        Cow::Owned(Value::Object(merged))
    }

    /// Mark the pointers of a resolved value as being walked, so a ref back to them is a cycle.
    pub(crate) fn enter(&mut self, followed: &[String]) {
        self.stack.extend(followed.iter().cloned());
    }

    /// Undo the matching `enter`.
    pub(crate) fn leave(&mut self, followed: &[String]) {
        let keep = self.stack.len().saturating_sub(followed.len());
        self.stack.truncate(keep);
    }

    /// The notes gathered so far, oldest first.
    pub(crate) fn take_notes(&mut self) -> Vec<ImportNote> {
        std::mem::take(&mut self.notes)
    }

    /// The approximate bytes copied by sibling merges since the last call.
    pub(crate) fn take_copied(&mut self) -> u64 {
        std::mem::take(&mut self.copied)
    }

    fn cut<'v>(&mut self, source: &str, kind: NoteKind, detail: String) -> Resolved<'v> {
        self.note(source, kind, detail);
        Resolved {
            value: Cow::Owned(Value::Object(Map::new())),
            followed: Vec::new(),
        }
    }

    /// Record a note once per source and detail.
    fn note(&mut self, source: &str, kind: NoteKind, detail: String) {
        if self.noted.insert((source.to_owned(), detail.clone())) {
            self.notes.push(ImportNote::new(source, kind, detail));
        }
    }
}

fn text_of(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned)
}

fn ref_of(value: &Value) -> Option<&str> {
    value.as_object()?.get("$ref")?.as_str()
}

/// Split a fragment such as `/paths/~1pets/get` into RFC 6901 segments, percent-decoding first.
fn decode(fragment: &str) -> Vec<String> {
    let decoded = percent_decode_str(fragment).decode_utf8_lossy();
    let Some(rest) = decoded.strip_prefix('/') else {
        return Vec::new();
    };
    rest.split('/')
        .map(|s| s.replace("~1", "/").replace("~0", "~"))
        .collect()
}

fn canonical(segments: &[String]) -> String {
    let mut pointer = String::from("#");
    for segment in segments {
        pointer.push('/');
        pointer.push_str(&segment.replace('~', "~0").replace('/', "~1"));
    }
    pointer
}

/// Walk the segments from the root; an array segment must be a plain decimal index.
fn walk<'a>(root: &'a Value, segments: &[String]) -> Option<&'a Value> {
    segments
        .iter()
        .try_fold(root, |value, segment| match value {
            Value::Object(map) => map.get(segment),
            Value::Array(items) if segment.bytes().all(|b| b.is_ascii_digit()) => {
                items.get(segment.parse::<usize>().ok()?)
            }
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::NoteKind;
    use serde_json::{Value, json};

    fn doc() -> Value {
        json!({
            "components": {
                "schemas": {
                    "Pet": {"type": "object", "description": "a pet", "properties": {"id": {"type": "integer"}}},
                    "Alias": {"$ref": "#/components/schemas/Pet"},
                    "a/b~c": {"type": "string"},
                    "Self": {"$ref": "#/components/schemas/Self"},
                    "Ping": {"$ref": "#/components/schemas/Pong"},
                    "Pong": {"$ref": "#/components/schemas/Ping"},
                    "Tree": {"type": "object", "properties": {"child": {"$ref": "#/components/schemas/Tree"}}}
                },
                "parameters": {"Id": {"name": "id", "in": "path", "description": "the id"}}
            },
            "paths": {"/pets/{id}": {"get": {"parameters": [{"name": "q", "in": "query"}]}}},
            "list": [10, 20]
        })
    }

    fn reference(target: &str) -> Value {
        json!({ "$ref": target })
    }

    fn kinds(notes: &[ImportNote]) -> Vec<(NoteKind, &str)> {
        notes.iter().map(|n| (n.kind, n.source.as_str())).collect()
    }

    #[test]
    fn a_value_without_a_ref_is_borrowed_as_is() {
        let root = doc();
        let mut r = Resolver::new(&root, Version::V30);
        let plain = json!({"type": "string"});
        let got = r.resolve(&plain, RefKind::Schema, "s");
        assert!(matches!(got.value, Cow::Borrowed(_)));
        assert_eq!(*got.value, plain);
        assert!(got.followed.is_empty());
        assert!(r.take_notes().is_empty());
    }

    #[test]
    fn a_target_is_borrowed_without_its_siblings() {
        let root = doc();
        let mut r = Resolver::new(&root, Version::V31);
        let input = json!({"$ref": "#/components/parameters/Id", "summary": "s"});
        assert!(matches!(
            r.resolve(&input, RefKind::Object, "s").value,
            Cow::Owned(_)
        ));
        assert!(r.take_copied() > 0);
        let got = r.target(&input, "s");
        assert!(matches!(got.value, Cow::Borrowed(_)));
        assert_eq!(*got.value, root["components"]["parameters"]["Id"]);
        assert_eq!(r.take_copied(), 0);
    }

    #[test]
    fn local_pointers_resolve_anywhere_with_escapes_and_indexes() {
        let root = doc();
        let mut r = Resolver::new(&root, Version::V30);
        let pet = root["components"]["schemas"]["Pet"].clone();
        let cases = [
            ("#/components/schemas/a~1b~0c", json!({"type": "string"})),
            (
                "#/paths/~1pets~1%7Bid%7D/get/parameters/0",
                json!({"name": "q", "in": "query"}),
            ),
            ("#/list/1", json!(20)),
            (
                "#/components/parameters/Id",
                root["components"]["parameters"]["Id"].clone(),
            ),
            ("#/components/schemas/Alias", pet),
        ];
        for (pointer, want) in cases {
            let input = reference(pointer);
            let got = r.resolve(&input, RefKind::Object, "s");
            assert_eq!(*got.value, want, "{pointer}");
            assert!(
                matches!(got.value, Cow::Borrowed(_)),
                "{pointer} is borrowed, never copied"
            );
        }
        let alias = reference("#/components/schemas/Alias");
        let followed = r.resolve(&alias, RefKind::Schema, "s").followed;
        assert_eq!(
            followed,
            ["#/components/schemas/Alias", "#/components/schemas/Pet"]
        );
        assert!(r.take_notes().is_empty());
    }

    #[test]
    fn external_anchor_and_missing_refs_become_empty_objects_with_dropped_notes() {
        let root = doc();
        let mut r = Resolver::new(&root, Version::V31);
        let targets = [
            "other.yaml#/Pet",
            "https://example.test/pet.json",
            "#pet-anchor",
            "#/components/schemas/Nope",
            "#/list/7",
        ];
        for target in targets {
            let input = reference(target);
            let got = r.resolve(&input, RefKind::Schema, "src");
            assert_eq!(*got.value, json!({}), "{target}");
            assert!(got.followed.is_empty());
        }
        let notes = r.take_notes();
        assert_eq!(notes.len(), 5);
        assert!(
            notes
                .iter()
                .all(|n| n.kind == NoteKind::Dropped && n.source == "src")
        );
        for (note, target) in notes.iter().zip(targets) {
            assert!(note.detail.contains(target), "{}", note.detail);
        }
        assert!(
            notes[0].detail.contains("not followed"),
            "{}",
            notes[0].detail
        );
    }

    #[test]
    fn ref_cycles_become_empty_objects_with_one_converted_note() {
        let root = doc();
        let mut r = Resolver::new(&root, Version::V30);
        for target in ["#/components/schemas/Self", "#/components/schemas/Ping"] {
            let input = reference(target);
            for _ in 0..3 {
                assert_eq!(*r.resolve(&input, RefKind::Schema, target).value, json!({}));
            }
        }
        let notes = r.take_notes();
        assert_eq!(
            kinds(&notes),
            [
                (NoteKind::Converted, "#/components/schemas/Self"),
                (NoteKind::Converted, "#/components/schemas/Ping"),
            ]
        );
        assert!(
            notes[1].detail.contains("empty object"),
            "{}",
            notes[1].detail
        );
    }

    #[test]
    fn a_ref_already_being_walked_is_a_cycle() {
        let root = doc();
        let mut r = Resolver::new(&root, Version::V30);
        let tree = reference("#/components/schemas/Tree");
        let outer = r.resolve(&tree, RefKind::Schema, "body");
        assert_eq!(outer.followed, ["#/components/schemas/Tree"]);
        r.enter(&outer.followed);
        let child = &outer.value["properties"]["child"];
        assert_eq!(*r.resolve(child, RefKind::Schema, "body").value, json!({}));
        r.leave(&outer.followed);
        assert_eq!(
            r.resolve(child, RefKind::Schema, "body").value["type"],
            "object"
        );
        assert_eq!(kinds(&r.take_notes()), [(NoteKind::Converted, "body")]);
    }

    #[test]
    fn a_chain_longer_than_the_cap_is_cut() {
        let mut schemas = serde_json::Map::new();
        for i in 1..=Resolver::MAX_CHAIN {
            schemas.insert(
                format!("s{i}"),
                reference(&format!("#/components/schemas/s{}", i + 1)),
            );
        }
        schemas.insert(
            format!("s{}", Resolver::MAX_CHAIN + 1),
            json!({"type": "string"}),
        );
        let root = json!({"components": {"schemas": schemas}});
        let mut r = Resolver::new(&root, Version::V30);
        let within = reference("#/components/schemas/s2");
        let got = r.resolve(&within, RefKind::Schema, "a");
        assert_eq!(
            (got.value.into_owned(), got.followed.len()),
            (json!({"type": "string"}), 64)
        );
        let over = reference("#/components/schemas/s1");
        assert_eq!(*r.resolve(&over, RefKind::Schema, "b").value, json!({}));
        let notes = r.take_notes();
        assert_eq!(kinds(&notes), [(NoteKind::Converted, "b")]);
        assert!(notes[0].detail.contains("64"), "{}", notes[0].detail);
    }

    #[test]
    fn a_dynamic_ref_is_reported_and_the_schema_kept() {
        let root = doc();
        let mut r = Resolver::new(&root, Version::V31);
        let schema = json!({"$dynamicRef": "#node", "type": "object"});
        assert_eq!(*r.resolve(&schema, RefKind::Schema, "s").value, schema);
        let notes = r.take_notes();
        assert_eq!(kinds(&notes), [(NoteKind::Dropped, "s")]);
        assert!(
            notes[0].detail.contains("$dynamicRef"),
            "{}",
            notes[0].detail
        );
    }

    #[test]
    fn siblings_follow_the_version_rules() {
        let root = doc();
        let object = json!({"$ref": "#/components/parameters/Id", "description": "override", "summary": "s", "in": "query"});
        let schema =
            json!({"$ref": "#/components/schemas/Pet", "description": "sibling", "nullable": true});
        let pet = root["components"]["schemas"]["Pet"].clone();
        let id = root["components"]["parameters"]["Id"].clone();
        for version in [Version::V30, Version::Swagger2] {
            let mut r = Resolver::new(&root, version);
            assert_eq!(*r.resolve(&object, RefKind::Object, "p").value, id);
            assert_eq!(*r.resolve(&schema, RefKind::Schema, "s").value, pet);
        }
        for version in [Version::V31, Version::V32] {
            let mut r = Resolver::new(&root, version);
            let p = r.resolve(&object, RefKind::Object, "p");
            assert_eq!(
                *p.value,
                json!({"name": "id", "in": "path", "description": "override", "summary": "s"})
            );
            let s = r.resolve(&schema, RefKind::Schema, "s");
            assert!(matches!(s.value, Cow::Owned(_)));
            let want = json!({"type": "object", "description": "sibling", "properties": {"id": {"type": "integer"}}, "nullable": true});
            assert_eq!(*s.value, want);
        }
    }
}
