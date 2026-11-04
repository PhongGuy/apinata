//! Bounded loading of import input: JSON or YAML bytes become one `serde_json::Value` under
//! per-syntax byte caps and a value budget, so a hostile file fails fast instead of exhausting
//! memory. Each format's parser then reads the loaded value.

use super::yaml_prepass::{self, Refusal};
use super::{DocumentError, ImportNote, NoteKind};
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use std::cell::Cell;
use std::fmt;

const BOM: &[u8] = b"\xef\xbb\xbf";

/// The deepest container nesting `merge_key_notes` walks, matching the YAML parser's own limit.
const MAX_DEPTH: usize = 128;

/// The ceilings applied while loading.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    pub(crate) json_bytes: u64,
    pub(crate) yaml_bytes: u64,
    /// Every scalar, container and map key counts as one value; YAML aliases count per use.
    pub(crate) values: u64,
    /// The most YAML events the pre-pass streams before refusing.
    pub(crate) yaml_events: u64,
}

impl Limits {
    pub(crate) const DEFAULT: Limits = Limits {
        json_bytes: 64 << 20,
        yaml_bytes: 16 << 20,
        values: 2_000_000,
        yaml_events: 2_000_000,
    };
}

/// Which parser read the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Syntax {
    Json,
    Yaml,
}

/// A loaded document and the syntax it was written in.
#[derive(Debug)]
pub(crate) struct Loaded {
    pub(crate) value: serde_json::Value,
    pub(crate) syntax: Syntax,
}

/// Load `input` under the default limits; see `load_with`.
pub(crate) fn load(input: &[u8]) -> Result<Loaded, DocumentError> {
    load_with(input, Limits::DEFAULT)
}

/// Load one JSON or YAML document. Input whose first non-blank byte after an optional BOM is
/// `{` or `[` is JSON, anything else YAML. Fails with `TooLarge` past a byte cap, the YAML event
/// budget or the value budget, and with `Parse` on a syntax error, non-UTF-8 YAML, more than one
/// YAML document, or nesting past the parser's limit; every offset indexes `input` as given.
pub(crate) fn load_with(input: &[u8], limits: Limits) -> Result<Loaded, DocumentError> {
    let body = strip_bom(input);
    let shift = input.len() - body.len();
    let first = body
        .iter()
        .find(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n'));
    let syntax = match first {
        Some(b'{' | b'[') => Syntax::Json,
        _ => Syntax::Yaml,
    };
    let cap = match syntax {
        Syntax::Json => limits.json_bytes,
        Syntax::Yaml => limits.yaml_bytes,
    };
    if body.len() as u64 > cap {
        return Err(DocumentError::TooLarge {
            limit: cap,
            unit: "bytes",
        });
    }
    if syntax == Syntax::Yaml {
        yaml_prepass_checked(body, shift, limits)?;
    }

    // The budget failure travels through each deserializer as its own error type, so the
    // `spent` flag, not the message, tells it apart from a syntax error.
    let left = Cell::new(limits.values);
    let spent = Cell::new(false);
    let seed = Seed {
        left: &left,
        spent: &spent,
    };
    let parsed = match syntax {
        Syntax::Json => {
            let mut de = serde_json::Deserializer::from_slice(body);
            seed.deserialize(&mut de)
                .and_then(|value| de.end().map(|()| value))
                .map_err(|e| (json_error_offset(body, &e), e.to_string()))
        }
        Syntax::Yaml => seed
            .deserialize(yaml_serde::Deserializer::from_slice(body))
            .map_err(|e| (e.location().map_or(0, |l| l.index()), e.to_string())),
    };
    match parsed {
        Ok(value) => Ok(Loaded { value, syntax }),
        Err(_) if spent.get() => Err(DocumentError::TooLarge {
            limit: limits.values,
            unit: "values",
        }),
        Err((offset, message)) => Err(DocumentError::Parse {
            offset: offset + shift,
            message,
        }),
    }
}

/// Refuse a YAML body that is not UTF-8, nests too deeply or streams too many events, before
/// `yaml_serde` builds anything; `shift` is the BOM length, so offsets index the input as given.
fn yaml_prepass_checked(body: &[u8], shift: usize, limits: Limits) -> Result<(), DocumentError> {
    let text = std::str::from_utf8(body).map_err(|e| DocumentError::Parse {
        offset: shift + e.valid_up_to(),
        message: e.to_string(),
    })?;
    yaml_prepass::prepass(text, limits.yaml_events).map_err(|refusal| match refusal {
        Refusal::TooManyEvents => DocumentError::TooLarge {
            limit: limits.yaml_events,
            unit: "events",
        },
        Refusal::At { offset, message } => DocumentError::Parse {
            offset: shift + offset,
            message,
        },
    })
}

/// `input` without a leading UTF-8 byte order mark.
pub(crate) fn strip_bom(input: &[u8]) -> &[u8] {
    input.strip_prefix(BOM).unwrap_or(input)
}

/// The byte offset of a JSON error: its line's start plus its column, saturating.
pub(crate) fn json_error_offset(json: &[u8], error: &serde_json::Error) -> usize {
    let line_start = match error.line() {
        0 | 1 => 0,
        line => json
            .iter()
            .enumerate()
            .filter(|(_, b)| **b == b'\n')
            .nth(line - 2)
            .map_or(json.len(), |(i, _)| i + 1),
    };
    line_start.saturating_add(error.column().saturating_sub(1))
}

/// One `Converted` note per YAML merge key (`<<`) in `value`, in document order: the loader
/// keeps a merge key as a literal key rather than applying it.
pub(crate) fn merge_key_notes(value: &Value) -> Vec<ImportNote> {
    let mut notes = Vec::new();
    walk_merge_keys(value, "", 0, &mut notes);
    notes
}

fn walk_merge_keys(value: &Value, path: &str, depth: usize, notes: &mut Vec<ImportNote>) {
    if depth >= MAX_DEPTH {
        return;
    }
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                if key == "<<" {
                    notes.push(ImportNote::new(
                        path.clone(),
                        NoteKind::Converted,
                        "the YAML merge key is not applied; its value stays under the key `<<`",
                    ));
                }
                walk_merge_keys(child, &path, depth + 1, notes);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                walk_merge_keys(child, &format!("{path}[{index}]"), depth + 1, notes);
            }
        }
        _ => {}
    }
}

/// Builds a `Value` from either deserializer, spending one unit of the shared budget per value
/// and map key. A YAML tag is dropped and its content kept.
#[derive(Clone, Copy)]
struct Seed<'a> {
    left: &'a Cell<u64>,
    spent: &'a Cell<bool>,
}

impl Seed<'_> {
    fn take<E: de::Error>(&self) -> Result<(), E> {
        let n = self.left.get();
        if n == 0 {
            self.spent.set(true);
            return Err(E::custom("value budget exceeded"));
        }
        self.left.set(n - 1);
        Ok(())
    }
}

impl<'de> DeserializeSeed<'de> for Seed<'_> {
    type Value = Value;

    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Seed<'_> {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any value")
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Value, E> {
        self.take()?;
        Ok(Value::Bool(v))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Value, E> {
        self.take()?;
        Ok(Value::Number(v.into()))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Value, E> {
        self.take()?;
        Ok(Value::Number(v.into()))
    }

    // Integers past 64 bits become the nearest double, as serde_json reads them from JSON.
    fn visit_i128<E: de::Error>(self, v: i128) -> Result<Value, E> {
        self.visit_f64(v as f64)
    }

    fn visit_u128<E: de::Error>(self, v: u128) -> Result<Value, E> {
        self.visit_f64(v as f64)
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Value, E> {
        self.take()?;
        Ok(Number::from_f64(v).map_or(Value::Null, Value::Number))
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Value, E> {
        self.take()?;
        Ok(Value::String(v.to_owned()))
    }

    fn visit_string<E: de::Error>(self, v: String) -> Result<Value, E> {
        self.take()?;
        Ok(Value::String(v))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        self.take()?;
        Ok(Value::Null)
    }

    fn visit_none<E: de::Error>(self) -> Result<Value, E> {
        self.take()?;
        Ok(Value::Null)
    }

    fn visit_some<D: de::Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        d.deserialize_any(self)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Value, A::Error> {
        self.take()?;
        let mut out = Vec::new();
        while let Some(v) = a.next_element_seed(self)? {
            out.push(v);
        }
        Ok(Value::Array(out))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Value, A::Error> {
        self.take()?;
        let mut out = Map::new();
        while let Some(k) = a.next_key_seed(self)? {
            let key = match k {
                Value::String(s) => s,
                other => other.to_string(),
            };
            let v = a.next_value_seed(self)?;
            out.insert(key, v);
        }
        Ok(Value::Object(out))
    }

    fn visit_enum<A: de::EnumAccess<'de>>(self, a: A) -> Result<Value, A::Error> {
        use de::VariantAccess;
        let (_tag, content): (String, A::Variant) = a.variant()?;
        content.newtype_variant_seed(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::NoteKind;
    use serde::de::DeserializeSeed;
    use serde_json::{Value, json};

    fn limits(json_bytes: u64, yaml_bytes: u64, values: u64) -> Limits {
        Limits {
            json_bytes,
            yaml_bytes,
            values,
            yaml_events: 1_000,
        }
    }

    fn nested(depth: usize) -> String {
        format!("{}{}", "[".repeat(depth), "]".repeat(depth))
    }

    #[test]
    fn json_and_yaml_load_to_the_same_ordered_value() {
        let j = load(br#" {"z": [1, "x", null, true], "a": {"c": 1.5}}"#).expect("json");
        let y = load(b"z: [1, x, null, true]\na:\n  c: 1.5\n").expect("yaml");
        assert_eq!((j.syntax, y.syntax), (Syntax::Json, Syntax::Yaml));
        assert_eq!(j.value, json!({"z": [1, "x", null, true], "a": {"c": 1.5}}));
        assert_eq!(j.value, y.value);
        let keys: Vec<&String> = y.value.as_object().expect("object").keys().collect();
        assert_eq!(keys, ["z", "a"]);
    }

    #[test]
    fn a_bom_and_leading_blanks_are_skipped_and_empty_yaml_is_null() {
        assert_eq!(
            load(b"\xef\xbb\xbf \r\n\t{\"a\": 1}").expect("json").value,
            json!({"a": 1})
        );
        assert_eq!(
            load(b"\xef\xbb\xbfa: 1").expect("yaml").value,
            json!({"a": 1})
        );
        assert_eq!(load(b"").expect("empty").value, Value::Null);
        assert_eq!(
            load(b"# only a comment\n").expect("comment").value,
            Value::Null
        );
    }

    #[test]
    fn integers_past_64_bits_load_as_numbers_in_both_syntaxes() {
        let j = load(br#"{"a": 99999999999999999999, "b": -99999999999999999999}"#).expect("json");
        let y = load(b"a: 99999999999999999999\nb: -99999999999999999999\n").expect("yaml");
        assert_eq!(j.value, json!({"a": 1e20, "b": -1e20}));
        assert_eq!(y.value, j.value);
    }

    #[test]
    fn yaml_keys_become_strings_and_tags_are_dropped() {
        let v = load(b"200: ok\ntrue: t\n1.5: f\na: !custom {x: [1, !t 2]}\nb: !!str 12\nbase: &b {x: 1}\nd:\n  <<: *b\n  y: 2\n")
            .expect("yaml")
            .value;
        let want = json!({"200": "ok", "true": "t", "1.5": "f", "a": {"x": [1, 2]}, "b": "12",
            "base": {"x": 1}, "d": {"<<": {"x": 1}, "y": 2}});
        assert_eq!(v, want);
    }

    #[test]
    fn syntax_errors_carry_a_byte_offset_into_the_input_as_given() {
        let offset = |input: &[u8]| match load(input) {
            Err(DocumentError::Parse { offset, .. }) => offset,
            other => panic!("{other:?}"),
        };
        assert_eq!(offset(b"{\"a\": 1,\n \"b\": x}"), 15);
        assert_eq!(offset(b"\xef\xbb\xbf{\"a\": 1,\n \"b\": x}"), 18);
        assert_eq!(offset(b"{} x"), 3);
        assert_eq!(offset(b"{"), 0);
        assert_eq!(offset(b"a: 1\nb: [1, 2\nc: 3\n"), 15);
        assert!(matches!(
            load(b"a: 1\n---\nb: 2\n"),
            Err(DocumentError::Parse { .. })
        ));
    }

    #[test]
    fn nesting_past_each_parsers_limit_is_a_parse_error() {
        assert!(load(nested(127).as_bytes()).is_ok());
        assert!(matches!(
            load(nested(128).as_bytes()),
            Err(DocumentError::Parse { offset: 127, .. })
        ));
        let block = |depth: usize| format!("{}x", "- ".repeat(depth));
        assert!(load(block(128).as_bytes()).is_ok());
        assert!(matches!(
            load(block(129).as_bytes()),
            Err(DocumentError::Parse { offset: 256, .. })
        ));
    }

    #[test]
    fn the_byte_caps_apply_per_syntax_before_parsing() {
        let small = limits(8, 4, 100);
        assert!(load_with(b"[1,2,34]", small).is_ok());
        assert_eq!(
            load_with(b"[1,2,345]", small).unwrap_err(),
            DocumentError::TooLarge {
                limit: 8,
                unit: "bytes"
            }
        );
        assert!(load_with(b"a: 1", small).is_ok());
        assert_eq!(
            load_with(b"a: 12", small).unwrap_err(),
            DocumentError::TooLarge {
                limit: 4,
                unit: "bytes"
            }
        );
        assert!(load_with(b"\xef\xbb\xbfa: 1", small).is_ok());
        assert_eq!(
            load_with(b"{{{{{{{{{", small).unwrap_err(),
            DocumentError::TooLarge {
                limit: 8,
                unit: "bytes"
            }
        );
    }

    #[test]
    fn the_real_caps_are_64_mib_of_json_and_16_mib_of_yaml() {
        let Limits {
            json_bytes,
            yaml_bytes,
            values,
            yaml_events,
        } = Limits::DEFAULT;
        assert_eq!(
            (json_bytes, yaml_bytes, values, yaml_events),
            (64 << 20, 16 << 20, 2_000_000, 2_000_000)
        );
        let mut json = vec![b' '; (64 << 20) + 1];
        json[0] = b'[';
        assert_eq!(
            load(&json).unwrap_err(),
            DocumentError::TooLarge {
                limit: 64 << 20,
                unit: "bytes"
            }
        );
        let yaml = vec![b'a'; (16 << 20) + 1];
        assert_eq!(
            load(&yaml).unwrap_err(),
            DocumentError::TooLarge {
                limit: 16 << 20,
                unit: "bytes"
            }
        );
    }

    #[test]
    fn the_value_budget_counts_keys_and_stops_both_parsers() {
        // The array, 1, the object, the key "a" and 2: five values.
        for input in [&br#"[1, {"a": 2}]"#[..], b"- 1\n- a: 2\n"] {
            assert!(load_with(input, limits(64, 64, 5)).is_ok());
            assert_eq!(
                load_with(input, limits(64, 64, 4)).unwrap_err(),
                DocumentError::TooLarge {
                    limit: 4,
                    unit: "values"
                }
            );
        }
    }

    #[test]
    fn merge_keys_are_noted_once_each_with_their_path() {
        let v = load(b"base: &b {x: 1}\nlist:\n  - <<: *b\n    y: 2\nd:\n  e:\n    <<: [*b]\n")
            .expect("yaml")
            .value;
        let notes = merge_key_notes(&v);
        let got: Vec<(&str, NoteKind)> =
            notes.iter().map(|n| (n.source.as_str(), n.kind)).collect();
        assert_eq!(
            got,
            [
                ("list[0].<<", NoteKind::Converted),
                ("d.e.<<", NoteKind::Converted)
            ]
        );
        assert!(
            notes[0].detail.contains("not applied"),
            "{}",
            notes[0].detail
        );
        assert!(merge_key_notes(&json!({"a": {"b": [1]}})).is_empty());
    }

    #[test]
    fn an_alias_bomb_stops_at_the_value_budget() {
        // About 2.1 billion values when expanded (research L2).
        let mut bomb = format!("a0: &a0 [{}]\n", vec!["x"; 4000].join(","));
        for n in 1..=6 {
            let refs = vec![format!("*a{}", n - 1); 9].join(",");
            bomb.push_str(&format!("a{n}: &a{n} [{refs}]\n"));
        }
        assert_eq!(bomb.len(), 8286);
        // With the pre-pass budget lifted, the deserializer's own value budget is what stops it.
        let no_prepass_cap = Limits {
            yaml_events: u64::MAX,
            ..Limits::DEFAULT
        };
        assert_eq!(
            load_with(bomb.as_bytes(), no_prepass_cap).unwrap_err(),
            DocumentError::TooLarge {
                limit: 2_000_000,
                unit: "values"
            }
        );
        // By default the pre-pass counts aliases as expanded and refuses first.
        assert_eq!(
            load(bomb.as_bytes()).unwrap_err(),
            DocumentError::TooLarge {
                limit: 2_000_000,
                unit: "events"
            }
        );
    }

    #[test]
    fn the_pre_pass_refuses_deep_or_long_yaml_before_loading() {
        let parse = |input: &[u8]| match load(input) {
            Err(DocumentError::Parse { offset, message }) => (offset, message),
            other => panic!("{other:?}"),
        };
        let deep = "nesting deeper than 128".to_owned();
        // 1 MiB of unclosed `[`: saphyr's scanner looks ahead and hits its own 255-level flow
        // limit (a u8) before our per-event 128 check sees event 129; both refuse, at once.
        let mut open = b"a: ".to_vec();
        open.resize(1 << 20, b'[');
        assert_eq!(parse(&open).0, 258);
        assert_eq!(parse("- ".repeat(1 << 19).as_bytes()), (256, deep.clone()));
        // A quote in a block scalar or a bracket in a plain scalar does not hide what follows.
        assert_eq!(
            parse(format!("- |\n  \"\n{}x", "- ".repeat(300)).as_bytes()),
            (264, deep.clone())
        );
        assert_eq!(
            parse(format!("- x [\n{}x", "- ".repeat(300)).as_bytes()),
            (262, deep)
        );
        // StreamStart, DocumentStart, SequenceStart, two scalars, SequenceEnd, DocumentEnd, StreamEnd.
        let events = |yaml_events| Limits {
            yaml_events,
            ..Limits::DEFAULT
        };
        assert!(load_with(b"- 1\n- 2\n", events(8)).is_ok());
        assert_eq!(
            load_with(b"- 1\n- 2\n", events(7)).unwrap_err(),
            DocumentError::TooLarge {
                limit: 7,
                unit: "events"
            }
        );
        assert!(matches!(
            load(b"a: \xff\n"),
            Err(DocumentError::Parse { offset: 3, .. })
        ));
    }

    #[test]
    fn brackets_and_quotes_inside_scalars_are_not_structure() {
        let plain: String = (0..200).map(|i| format!("t{i}: see [x\n")).collect();
        assert!(load(plain.as_bytes()).is_ok());
        let many = "[".repeat(300);
        let text = format!("a: '{many}'\nb: \"{many}\"\n# {many}\nc: ^[a-z\nd: |\n  {many}\n");
        assert!(load(text.as_bytes()).is_ok());
    }

    #[test]
    fn a_tab_after_a_colon_passes_and_tabs_in_scalars_are_kept() {
        assert_eq!(load(b"a:\tb\n").expect("tab").value, json!({"a": "b"}));
        assert_eq!(
            load(b"a:\tb\nc:\t[1]\n").expect("tabs").value,
            json!({"a": "b", "c": [1]})
        );
        let v = load(b"a: \"x:\ty\"\nb: 'x:\ty'\nc: |\n  x:\ty\n")
            .expect("scalars")
            .value;
        assert_eq!(v, json!({"a": "x:\ty", "b": "x:\ty", "c": "x:\ty\n"}));
        assert!(matches!(
            load(b"a: x:\ty\n"),
            Err(DocumentError::Parse { .. })
        ));
    }

    // A known divergence: saphyr-parser refuses a tab leading a quoted scalar's continuation
    // line, which yaml_serde loads.
    #[test]
    fn a_tab_led_quoted_continuation_is_refused_though_yaml_serde_loads_it() {
        for text in [&b"a: \"x\n\ty\"\n"[..], b"a:\n  b: 'x\n\ty'\n"] {
            assert!(yaml_serde::from_slice::<Value>(text).is_ok());
            let refused = load(text);
            assert!(
                matches!(&refused, Err(DocumentError::Parse { message, .. }) if message.contains("tab")),
                "{refused:?}"
            );
        }
    }

    #[test]
    fn the_pre_pass_accepts_every_repository_yaml_file_the_loader_accepts() {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for path in entries.flatten().map(|e| e.path()) {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if path.is_dir() && !matches!(name.as_str(), "target" | "node_modules" | ".git") {
                    walk(&path, out);
                } else if name.ends_with(".yml") || name.ends_with(".yaml") {
                    out.push(path);
                }
            }
        }
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut files = Vec::new();
        walk(&repo.join("crates"), &mut files);
        assert!(files.len() > 100, "only {} YAML files found", files.len());
        let refused: Vec<String> = files
            .iter()
            .filter_map(|path| {
                let bytes = std::fs::read(path).expect("read");
                let body = strip_bom(&bytes);
                let plain = yaml_serde::from_slice::<yaml_serde::Value>(body).is_ok();
                let text = std::str::from_utf8(body).ok()?;
                (plain && crate::import::yaml_prepass::prepass(text, u64::MAX).is_err())
                    .then(|| path.display().to_string())
            })
            .collect();
        assert!(refused.is_empty(), "{refused:#?}");
    }

    /// One case per process: `APINATA_PROBE=<case>`, `APINATA_PROBE_BYTES` (default 16 MiB),
    /// `APINATA_PROBE_RAW=1` to skip the pre-pass.
    #[test]
    #[ignore = "measurement: up to 3.5 GB of memory"]
    fn yaml_cap_probe() {
        let var = |key: &str| std::env::var(key).ok();
        let case = var("APINATA_PROBE").expect("APINATA_PROBE=<case>");
        let bytes: usize =
            var("APINATA_PROBE_BYTES").map_or(16 << 20, |b| b.parse().expect("bytes"));
        let (head, unit) = match case.as_str() {
            "open" => ("a: ", "[".to_owned()),
            "dash" => ("", "- ".to_owned()),
            "query" => ("", "? ".to_owned()),
            "dash4" => ("", "- - - - x\n".to_owned()),
            "dash128" => ("", format!("{}x\n", "- ".repeat(128))),
            "query4" => ("", "? ? ? ? x\n".to_owned()),
            "flow" => ("a: [[]", ",[]".to_owned()),
            "block" => ("", "- 0\n".to_owned()),
            "anchors" => ("", "- &a{} x\n".to_owned()),
            other => panic!("unknown case {other}"),
        };
        let mut text = head.to_owned();
        for i in 0.. {
            let next = unit.replace("{}", &i.to_string());
            if text.len() + next.len() + 1 > bytes {
                break;
            }
            text.push_str(&next);
        }
        if case == "flow" {
            text.push(']');
        }
        let started = std::time::Instant::now();
        let outcome = if var("APINATA_PROBE_RAW").is_some() {
            let (left, spent) = (Cell::new(Limits::DEFAULT.values), Cell::new(false));
            let seed = Seed {
                left: &left,
                spent: &spent,
            };
            seed.deserialize(yaml_serde::Deserializer::from_slice(text.as_bytes()))
                .map(drop)
                .map_err(|e| e.to_string())
        } else {
            load(text.as_bytes()).map(drop).map_err(|e| e.to_string())
        };
        let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        let peak = status
            .lines()
            .find(|l| l.starts_with("VmHWM"))
            .unwrap_or("VmHWM: ?");
        eprintln!(
            "{case} bytes {} {outcome:?} ms {} {peak}",
            text.len(),
            started.elapsed().as_millis()
        );
    }
}
