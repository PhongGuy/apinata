//! JavaScript value semantics Bruno relies on: Number(), String(), JSON.stringify and typed variables.

use serde_json::Value;

/// ECMA-262 `StrWhiteSpaceChar`: Unicode White_Space without U+0085, plus the BOM.
pub(crate) fn is_js_whitespace(c: char) -> bool {
    (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'
}

/// Trims whitespace as JavaScript's `String.prototype.trim` and `Number()` do.
pub(crate) fn js_trim(text: &str) -> &str {
    text.trim_matches(is_js_whitespace)
}

/// The text a script setter stores for `text`: JavaScript's `String.prototype.trim`.
pub fn script_code(text: &str) -> &str {
    js_trim(text)
}

/// JavaScript `Number(text)`; `None` where JavaScript gives `NaN`.
pub(crate) fn parse_number(text: &str) -> Option<f64> {
    let text = js_trim(text);
    if text.is_empty() {
        return Some(0.0);
    }
    if let Some(n) = parse_radix(text) {
        return n;
    }
    match text {
        "Infinity" | "+Infinity" => return Some(f64::INFINITY),
        "-Infinity" => return Some(f64::NEG_INFINITY),
        _ => {}
    }
    // Rust's parser also accepts `inf`, `nan` and `infinity`, so the grammar is checked first.
    if is_decimal_literal(text) {
        text.parse::<f64>().ok()
    } else {
        None
    }
}

/// Parses an unsigned `0x`, `0o` or `0b` literal; the outer `None` means "not a radix literal".
fn parse_radix(text: &str) -> Option<Option<f64>> {
    let bytes = text.as_bytes();
    if bytes.len() < 2 || bytes[0] != b'0' {
        return None;
    }
    let radix = match bytes[1] {
        b'x' | b'X' => 16,
        b'o' | b'O' => 8,
        b'b' | b'B' => 2,
        _ => return None,
    };
    let digits = &text[2..];
    if digits.is_empty() {
        return Some(None);
    }
    // Exact in u128 while it fits; past 128 bits the f64 fold may differ from JS in the last bit.
    let mut exact: Option<u128> = Some(0);
    let mut approx = 0.0_f64;
    for c in digits.chars() {
        let Some(d) = c.to_digit(radix) else {
            return Some(None);
        };
        exact = exact.and_then(|n| n.checked_mul(u128::from(radix))?.checked_add(u128::from(d)));
        approx = approx * f64::from(radix) + f64::from(d);
    }
    Some(Some(exact.map_or(approx, |n| n as f64)))
}

/// Matches `[+-]?(digits[.digits?]|.digits)([eE][+-]?digits)?`.
fn is_decimal_literal(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut i = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
    let digits = |i: &mut usize| {
        let start = *i;
        while bytes.get(*i).is_some_and(u8::is_ascii_digit) {
            *i += 1;
        }
        *i - start
    };
    let mut mantissa = digits(&mut i);
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        mantissa += digits(&mut i);
    }
    if mantissa == 0 {
        return false;
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(bytes.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        if digits(&mut i) == 0 {
            return false;
        }
    }
    i == bytes.len()
}

/// JavaScript `String(n)`.
pub(crate) fn number_to_string(n: f64) -> String {
    ryu_js::Buffer::new().format(n).to_owned()
}

/// serde_json formatter that writes numbers as JavaScript's JSON.stringify does.
struct JsFormatter;

/// Integers up to 2^53 are exact in an f64; beyond that JavaScript has already rounded them.
const SAFE_INTEGER: u64 = 1 << 53;

impl serde_json::ser::Formatter for JsFormatter {
    fn write_f64<W: ?Sized + std::io::Write>(&mut self, w: &mut W, v: f64) -> std::io::Result<()> {
        w.write_all(ryu_js::Buffer::new().format(v).as_bytes())
    }
    fn write_u64<W: ?Sized + std::io::Write>(&mut self, w: &mut W, v: u64) -> std::io::Result<()> {
        if v <= SAFE_INTEGER {
            write!(w, "{v}")
        } else {
            self.write_f64(w, v as f64)
        }
    }
    fn write_i64<W: ?Sized + std::io::Write>(&mut self, w: &mut W, v: i64) -> std::io::Result<()> {
        if v.unsigned_abs() <= SAFE_INTEGER {
            write!(w, "{v}")
        } else {
            self.write_f64(w, v as f64)
        }
    }
}

/// JavaScript `JSON.stringify(value)`.
pub(crate) fn stringify(value: &Value) -> String {
    let mut out = Vec::new();
    let mut serializer = serde_json::Serializer::with_formatter(&mut out, JsFormatter);
    // Serializing a `Value` into memory cannot fail, so an error degrades to empty text.
    if serde::Serialize::serialize(value, &mut serializer).is_err() {
        return String::new();
    }
    String::from_utf8(out).unwrap_or_default()
}

/// Pretty layout from serde_json with numbers as [`JsFormatter`] writes them.
struct PrettyJsFormatter(serde_json::ser::PrettyFormatter<'static>);

/// Forwards each structural `Formatter` method to the pretty formatter.
macro_rules! pretty {
    ($($name:ident($($arg:ident: $ty:ty),*);)*) => {$(
        fn $name<W: ?Sized + std::io::Write>(&mut self, w: &mut W $(, $arg: $ty)*) -> std::io::Result<()> {
            self.0.$name(w $(, $arg)*)
        }
    )*};
}

impl serde_json::ser::Formatter for PrettyJsFormatter {
    pretty! {
        begin_array();
        end_array();
        begin_array_value(first: bool);
        end_array_value();
        begin_object();
        end_object();
        begin_object_key(first: bool);
        begin_object_value();
        end_object_value();
    }
    fn write_f64<W: ?Sized + std::io::Write>(&mut self, w: &mut W, v: f64) -> std::io::Result<()> {
        JsFormatter.write_f64(w, v)
    }
    fn write_u64<W: ?Sized + std::io::Write>(&mut self, w: &mut W, v: u64) -> std::io::Result<()> {
        JsFormatter.write_u64(w, v)
    }
    fn write_i64<W: ?Sized + std::io::Write>(&mut self, w: &mut W, v: i64) -> std::io::Result<()> {
        JsFormatter.write_i64(w, v)
    }
}

/// JavaScript `JSON.stringify(value, null, 2)`.
pub(crate) fn stringify_pretty(value: &Value) -> String {
    let mut out = Vec::new();
    let formatter = PrettyJsFormatter(serde_json::ser::PrettyFormatter::new());
    let mut serializer = serde_json::Serializer::with_formatter(&mut out, formatter);
    if serde::Serialize::serialize(value, &mut serializer).is_err() {
        return String::new();
    }
    String::from_utf8(out).unwrap_or_default()
}

/// Renders a value the way Bruno's interpolation turns it into text.
pub(crate) fn to_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.as_f64().map_or_else(|| n.to_string(), number_to_string),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_owned(),
        Value::Array(_) | Value::Object(_) => stringify(value),
    }
}

/// Port of bruno-common `parseValueByDataType`: the typed value, or the raw string on failure.
pub(crate) fn typed_value(kind: &str, data: &str) -> Value {
    let raw = || Value::String(data.to_owned());
    match kind {
        "number" if js_trim(data).is_empty() => raw(),
        "number" => match parse_number(data) {
            None => raw(),
            // JSON cannot hold Infinity, so it is kept as the text JavaScript would print.
            Some(n) => serde_json::Number::from_f64(n)
                .map_or_else(|| Value::String(number_to_string(n)), Value::Number),
        },
        "boolean" => match data {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => raw(),
        },
        "object" => match serde_json::from_str::<Value>(js_trim(data)) {
            Ok(parsed @ (Value::Object(_) | Value::Array(_))) => parsed,
            _ => raw(),
        },
        _ => raw(),
    }
}

/// Port of the filestore's `ensureString` for scalars; mappings and sequences are `None`.
pub(crate) fn yaml_to_string(value: &yaml_serde::Value) -> Option<String> {
    match value {
        yaml_serde::Value::Null => Some(String::new()),
        yaml_serde::Value::Bool(b) => Some(b.to_string()),
        yaml_serde::Value::Number(n) => n.as_f64().map(number_to_string),
        yaml_serde::Value::String(s) => Some(s.clone()),
        yaml_serde::Value::Tagged(tagged) => yaml_to_string(&tagged.value),
        yaml_serde::Value::Sequence(_) | yaml_serde::Value::Mapping(_) => None,
    }
}

/// JavaScript `String(v)` for a YAML value that is present and not null: a list joins its
/// items with `,` and a mapping is `[object Object]`.
pub(crate) fn yaml_js_string(value: &yaml_serde::Value) -> Option<String> {
    match value {
        yaml_serde::Value::Null => None,
        yaml_serde::Value::Mapping(_) => Some("[object Object]".to_owned()),
        yaml_serde::Value::Sequence(items) => Some(
            items
                .iter()
                .map(|item| yaml_js_string(item).unwrap_or_default())
                .collect::<Vec<_>>()
                .join(","),
        ),
        yaml_serde::Value::Tagged(tagged) => yaml_js_string(&tagged.value),
        scalar => yaml_to_string(scalar),
    }
}

/// Whether a list row is switched off with `disabled: true`.
pub(crate) fn is_disabled(row: &yaml_serde::Value) -> bool {
    row.get("disabled").and_then(yaml_serde::Value::as_bool) == Some(true)
}

/// Reads a variable row's `value`: plain scalars as text, `{type, data}` mappings as typed values.
pub(crate) fn variable_value(value: Option<&yaml_serde::Value>) -> Option<Value> {
    let Some(value) = value else {
        return Some(Value::String(String::new()));
    };
    if let yaml_serde::Value::Mapping(map) = value {
        let kind = yaml_to_string(map.get("type")?)?;
        let data = yaml_to_string(map.get("data")?)?;
        return Some(typed_value(&kind, &data));
    }
    yaml_to_string(value).map(Value::String)
}

/// Whether `text` is an array index as JavaScript spells it: `0`, or digits without a leading zero.
pub(crate) fn is_canonical_index(text: &str) -> bool {
    !text.is_empty()
        && text.bytes().all(|b| b.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_number_follows_js_number() {
        let cases: &[(&str, Option<f64>)] = &[
            (" 42 ", Some(42.0)),
            ("", Some(0.0)),
            ("  ", Some(0.0)),
            ("0x1F", Some(31.0)),
            ("0X1f", Some(31.0)),
            ("0b101", Some(5.0)),
            ("0o17", Some(15.0)),
            ("1e3", Some(1000.0)),
            (".5", Some(0.5)),
            ("5.", Some(5.0)),
            ("+7", Some(7.0)),
            ("-2.5e-3", Some(-0.0025)),
            ("-0", Some(0.0)),
            ("Infinity", Some(f64::INFINITY)),
            ("-Infinity", Some(f64::NEG_INFINITY)),
            ("1e400", Some(f64::INFINITY)),
            ("9007199254740993", Some(9_007_199_254_740_992.0)),
            ("\u{a0}42\u{feff}", Some(42.0)),
            ("-0x10", None),
            ("1_000", None),
            ("inf", None),
            ("NaN", None),
            ("12abc", None),
            ("\u{85}1", None),
            ("1e", None),
            ("e5", None),
            (".", None),
            ("0x", None),
        ];
        for (text, expected) in cases {
            assert_eq!(parse_number(text), *expected, "{text:?}");
        }
    }

    #[test]
    fn number_to_string_matches_js_string() {
        let cases = [
            (300.0, "300"),
            (-0.0, "0"),
            (1e21, "1e+21"),
            (0.1 + 0.2, "0.30000000000000004"),
            (1e-7, "1e-7"),
            (123e-20, "1.23e-18"),
            (f64::INFINITY, "Infinity"),
            (f64::NEG_INFINITY, "-Infinity"),
            (18_446_744_073_709_551_616.0, "18446744073709552000"),
        ];
        for (n, expected) in cases {
            assert_eq!(number_to_string(n), expected, "{n}");
        }
    }

    #[test]
    fn stringify_matches_json_stringify() {
        let input = r#"{"integer":123,"negativeInteger":-99,"zero":0,"float":2.718,"negativeFloat":-1.618,"largeDouble":12345.678901234567,"smallDouble":9.876e-12,"booleanTrue":true,"booleanFalse":false,"neg0":-0,"neg0f":-0.0,"big":18446744073709551615,"e21":1e21,"one":1.0,"x":1e-7,"z":{"b":1,"a":[1,2.50]}}"#;
        let node = r#"{"integer":123,"negativeInteger":-99,"zero":0,"float":2.718,"negativeFloat":-1.618,"largeDouble":12345.678901234567,"smallDouble":9.876e-12,"booleanTrue":true,"booleanFalse":false,"neg0":0,"neg0f":0,"big":18446744073709552000,"e21":1e+21,"one":1,"x":1e-7,"z":{"b":1,"a":[1,2.5]}}"#;
        let value: serde_json::Value = serde_json::from_str(input).expect("json");
        assert_eq!(stringify(&value), node);
        let escapes = json!({"s": "a\"b\\c\u{1}\u{1f}\n/\u{2028}"});
        let expected = format!(r#"{{"s":"a\"b\\c\u0001\u001f\n/{}"}}"#, '\u{2028}');
        assert_eq!(stringify(&escapes), expected);
    }

    #[test]
    fn stringify_pretty_matches_two_space_json_stringify() {
        let value: serde_json::Value =
            serde_json::from_str(r#"{"a":1.0,"b":[true,null,1e21],"c":{},"d":[]}"#).expect("json");
        let node = "{\n  \"a\": 1,\n  \"b\": [\n    true,\n    null,\n    1e+21\n  ],\n  \"c\": {},\n  \"d\": []\n}";
        assert_eq!(stringify_pretty(&value), node);
        assert_eq!(stringify_pretty(&json!(-0.0)), "0");
    }

    #[test]
    fn typed_values_parse_like_parse_value_by_data_type() {
        assert_eq!(typed_value("number", "42").as_f64(), Some(42.0));
        assert_eq!(typed_value("number", " 300 ").as_f64(), Some(300.0));
        assert_eq!(typed_value("number", "0x10").as_f64(), Some(16.0));
        assert_eq!(typed_value("number", ""), json!(""));
        assert_eq!(typed_value("number", "not-a-number"), json!("not-a-number"));
        assert_eq!(typed_value("number", "1e400"), json!("Infinity"));
        assert_eq!(typed_value("boolean", "true"), json!(true));
        assert_eq!(typed_value("boolean", "false"), json!(false));
        assert_eq!(typed_value("boolean", "TRUE"), json!("TRUE"));
        assert_eq!(typed_value("boolean", " true"), json!(" true"));
        assert_eq!(
            typed_value("object", r#"{"scope":"env"}"#),
            json!({"scope": "env"})
        );
        assert_eq!(typed_value("object", "  [1, 2]  "), json!([1, 2]));
        assert_eq!(typed_value("object", "5"), json!("5"));
        assert_eq!(typed_value("object", "null"), json!("null"));
        assert_eq!(typed_value("object", "{oops"), json!("{oops"));
        assert_eq!(typed_value("string", "x"), json!("x"));
        assert_eq!(typed_value("null", "x"), json!("x"));
        assert_eq!(typed_value("uuid", "x"), json!("x"));
    }

    #[test]
    fn to_text_renders_like_bruno_interpolation() {
        assert_eq!(to_text(&json!("plain")), "plain");
        assert_eq!(to_text(&json!(true)), "true");
        assert_eq!(to_text(&json!(null)), "null");
        assert_eq!(to_text(&typed_value("number", "42")), "42");
        assert_eq!(
            to_text(&json!({"a": 1.0, "b": [true, null]})),
            r#"{"a":1,"b":[true,null]}"#
        );
    }

    fn yaml(text: &str) -> yaml_serde::Value {
        yaml_serde::from_str(text).expect("yaml")
    }

    #[test]
    fn yaml_scalars_stringify_like_ensure_string() {
        assert_eq!(yaml_to_string(&yaml("abc")), Some("abc".to_owned()));
        assert_eq!(yaml_to_string(&yaml("5")), Some("5".to_owned()));
        assert_eq!(yaml_to_string(&yaml("-7")), Some("-7".to_owned()));
        assert_eq!(yaml_to_string(&yaml("1.50")), Some("1.5".to_owned()));
        assert_eq!(yaml_to_string(&yaml("true")), Some("true".to_owned()));
        assert_eq!(yaml_to_string(&yaml("~")), Some(String::new()));
        assert_eq!(yaml_to_string(&yaml("a: 1")), None);
        assert_eq!(yaml_to_string(&yaml("- 1")), None);
    }

    #[test]
    fn variable_values_read_plain_and_typed_rows() {
        assert_eq!(variable_value(None), Some(json!("")));
        assert_eq!(variable_value(Some(&yaml("~"))), Some(json!("")));
        assert_eq!(variable_value(Some(&yaml("5"))), Some(json!("5")));
        let typed = |text: &str| variable_value(Some(&yaml(text)));
        assert_eq!(
            typed("type: number\ndata: \"42\"").and_then(|v| v.as_f64()),
            Some(42.0)
        );
        assert_eq!(
            typed("type: number\ndata: 42").and_then(|v| v.as_f64()),
            Some(42.0)
        );
        assert_eq!(
            typed("type: object\ndata: '{\"k\":1}'"),
            Some(json!({"k": 1}))
        );
        assert_eq!(typed("type: weird\ndata: x"), Some(json!("x")));
        assert_eq!(typed("type: number"), None);
        assert_eq!(typed("- title: a\n  value: x"), None);
    }
}
