//! `JSON.parse` for the legacy readers. serde_json rejects `1e400` and a lone surrogate
//! escape, and keeps integers above 2^53 exact, all of which JavaScript accepts or rounds.
//! Numbers come out through the shared JS-number carrier. A lone surrogate becomes U+FFFD
//! because a Rust string cannot hold one.

use super::js_number;
use serde_json::{Map, Value};

/// Nesting beyond this is rejected rather than risking the stack.
const MAX_DEPTH: usize = 512;

/// Why the text is not JSON: the byte offset of the failure and what was found there.
#[derive(Debug)]
pub(crate) struct JsonError {
    pub(crate) offset: usize,
    pub(crate) message: String,
}

/// Parses the whole text as `JSON.parse` would.
pub(crate) fn parse(text: &str) -> Result<Value, JsonError> {
    let mut p = Parser { text, pos: 0 };
    p.skip_space();
    let value = p.value(0)?;
    p.skip_space();
    if p.pos < text.len() {
        return Err(p.unexpected());
    }
    Ok(value)
}

struct Parser<'a> {
    text: &'a str,
    pos: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.pos).copied()
    }

    fn unexpected(&self) -> JsonError {
        let message = match self.text[self.pos..].chars().next() {
            Some(c) => format!("unexpected {:?}", c),
            None => "unexpected end of input".to_owned(),
        };
        JsonError {
            offset: self.pos,
            message,
        }
    }

    fn skip_space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), JsonError> {
        if self.peek() == Some(byte) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.unexpected())
        }
    }

    fn literal(&mut self, word: &str, value: Value) -> Result<Value, JsonError> {
        if self.text[self.pos..].starts_with(word) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(self.unexpected())
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, JsonError> {
        if depth > MAX_DEPTH {
            return Err(JsonError {
                offset: self.pos,
                message: "nesting too deep".to_owned(),
            });
        }
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => self.string().map(Value::String),
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'n') => self.literal("null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(self.unexpected()),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, JsonError> {
        self.pos += 1;
        let mut fields = Map::new();
        self.skip_space();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Value::Object(fields));
        }
        loop {
            self.skip_space();
            if self.peek() != Some(b'"') {
                return Err(self.unexpected());
            }
            let key = self.string()?;
            self.skip_space();
            self.expect(b':')?;
            self.skip_space();
            let value = self.value(depth + 1)?;
            // A repeated key keeps its first position and takes the last value, as in JS.
            fields.insert(key, value);
            self.skip_space();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Value::Object(fields));
                }
                _ => return Err(self.unexpected()),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value, JsonError> {
        self.pos += 1;
        let mut items = Vec::new();
        self.skip_space();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Value::Array(items));
        }
        loop {
            self.skip_space();
            items.push(self.value(depth + 1)?);
            self.skip_space();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Value::Array(items));
                }
                _ => return Err(self.unexpected()),
            }
        }
    }

    /// `-?(0|[1-9]\d*)(\.\d+)?([eE][+-]?\d+)?`, rounded to the nearest f64.
    fn number(&mut self) -> Result<Value, JsonError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => self.digits(),
            _ => return Err(self.unexpected()),
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            self.some_digits()?;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            self.some_digits()?;
        }
        let n: f64 = self.text[start..self.pos].parse().map_err(|_| JsonError {
            offset: start,
            message: "invalid number".to_owned(),
        })?;
        Ok(js_number(n))
    }

    fn digits(&mut self) {
        while self.peek().is_some_and(|b| b.is_ascii_digit()) {
            self.pos += 1;
        }
    }

    fn some_digits(&mut self) -> Result<(), JsonError> {
        if !self.peek().is_some_and(|b| b.is_ascii_digit()) {
            return Err(self.unexpected());
        }
        self.digits();
        Ok(())
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        let digits = self.text.get(self.pos..self.pos + 4).unwrap_or("");
        if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(self.unexpected());
        }
        self.pos += 4;
        Ok(u32::from_str_radix(digits, 16).expect("four hex digits"))
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.pos += 1;
        let mut out = String::new();
        loop {
            let Some(c) = self.text[self.pos..].chars().next() else {
                return Err(self.unexpected());
            };
            match c {
                '"' => {
                    self.pos += 1;
                    return Ok(out);
                }
                '\\' => {
                    self.pos += 1;
                    self.escape(&mut out)?;
                }
                c if c < ' ' => return Err(self.unexpected()),
                c => {
                    out.push(c);
                    self.pos += c.len_utf8();
                }
            }
        }
    }

    fn escape(&mut self, out: &mut String) -> Result<(), JsonError> {
        let Some(byte) = self.peek() else {
            return Err(self.unexpected());
        };
        self.pos += 1;
        match byte {
            b'"' => out.push('"'),
            b'\\' => out.push('\\'),
            b'/' => out.push('/'),
            b'b' => out.push('\u{8}'),
            b'f' => out.push('\u{c}'),
            b'n' => out.push('\n'),
            b'r' => out.push('\r'),
            b't' => out.push('\t'),
            b'u' => {
                let unit = self.hex4()?;
                out.push(self.unit_char(unit)?);
            }
            _ => {
                self.pos -= 1;
                return Err(self.unexpected());
            }
        }
        Ok(())
    }

    /// A UTF-16 unit as a char: a high surrogate followed by a `\u` low one joins into the
    /// astral char, and any other surrogate is lone.
    fn unit_char(&mut self, unit: u32) -> Result<char, JsonError> {
        if (0xd800..0xdc00).contains(&unit) && self.text[self.pos..].starts_with("\\u") {
            let save = self.pos;
            self.pos += 2;
            let low = self.hex4()?;
            if (0xdc00..0xe000).contains(&low) {
                let joined = 0x10000 + ((unit - 0xd800) << 10) + (low - 0xdc00);
                return Ok(char::from_u32(joined).expect("a surrogate pair is a scalar"));
            }
            self.pos = save;
        }
        Ok(char::from_u32(unit).unwrap_or('\u{fffd}'))
    }
}

#[cfg(test)]
mod tests {
    use super::parse;
    use serde_json::json;

    #[test]
    fn numbers_round_like_a_javascript_double() {
        assert_eq!(
            parse("9007199254740993").unwrap(),
            json!(9007199254740992_u64)
        );
        assert_eq!(
            parse("-9007199254740993").unwrap(),
            json!(-9007199254740992_i64)
        );
        assert_eq!(parse("1.0").unwrap(), json!(1));
        assert_eq!(parse("0.1e1").unwrap(), json!(1));
        assert_eq!(parse("1e400").unwrap(), json!("Infinity"));
        assert_eq!(parse("-1e400").unwrap(), json!("-Infinity"));
        assert_eq!(parse("1e-400").unwrap(), json!(0));
        assert_eq!(parse("-0").unwrap(), json!(0));
    }

    #[test]
    fn a_lone_surrogate_escape_is_accepted_and_a_pair_joins() {
        assert_eq!(parse(r#""\ud800x""#).unwrap(), json!("\u{fffd}x"));
        assert_eq!(parse(r#""a\udc00""#).unwrap(), json!("a\u{fffd}"));
        assert_eq!(parse("\"\u{1f600}\"").unwrap(), json!("\u{1f600}"));
        assert_eq!(parse(r#""\ud83dA""#).unwrap(), json!("\u{fffd}A"));
    }

    #[test]
    fn a_repeated_key_keeps_its_first_position_and_last_value() {
        let value = parse(r#"{"a": 1, "b": 2, "a": 3}"#).unwrap();
        assert_eq!(serde_json::to_string(&value).unwrap(), r#"{"a":3,"b":2}"#);
    }

    #[test]
    fn text_javascript_rejects_is_rejected() {
        for bad in [
            "",
            "01",
            "1.",
            ".5",
            "+1",
            "[1,]",
            "{\"a\":1,}",
            "'a'",
            "\"a\nb\"",
            r#""\x""#,
            r#""\u12""#,
            "NaN",
            "Infinity",
            "[1] 2",
            "\u{feff}1",
            "{a: 1}",
            "\u{a0}1",
        ] {
            assert!(parse(bad).is_err(), "{bad:?} should be rejected");
        }
        assert!(parse(&"[".repeat(2000)).is_err());
    }

    #[test]
    fn json_whitespace_is_four_characters() {
        assert_eq!(parse(" \t\r\n[ 1 , 2 ]\n").unwrap(), json!([1, 2]));
    }
}
