//! A textual scan of JSON members, so a body can be rewritten without re-formatting it.

use std::ops::Range;

/// What a member's value is, with the byte range a rewrite replaces.
#[derive(Debug)]
pub(super) enum Shape {
    /// The content inside the quotes.
    Str(Range<usize>),
    /// The token, from `-` or a digit through `[0-9+-.eE]`.
    Number(Range<usize>),
    /// From `{` or `[` through its matching close.
    Composite(Range<usize>),
    /// `true`, `false`, `null` or anything else.
    Literal,
    /// The text ends inside the value or before it starts.
    Unterminated,
}

/// Every `"key": value` member at any depth, ordered by key position. A key is a complete
/// string followed by only whitespace and one `:`. Linear in `text.len()`.
pub(super) fn members(text: &str) -> Vec<(Range<usize>, Shape)> {
    // One pass finds every string's content and pairs each bracket outside strings with its
    // close by a stack, so a composite's end is a lookup and never a rescan. Escapes skip two
    // bytes; `"` and `\` never occur inside a multi-byte UTF-8 sequence, so ranges stay on
    // char boundaries.
    let bytes = text.as_bytes();
    let mut strings: Vec<Range<usize>> = Vec::new();
    let mut composites: Vec<(usize, Option<usize>)> = Vec::new();
    let mut open = Vec::new();
    let mut i = 0;
    'scan: while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                let mut j = i + 1;
                loop {
                    match bytes.get(j) {
                        None => break 'scan,
                        Some(b'\\') => j += 2,
                        Some(b'"') => break,
                        Some(_) => j += 1,
                    }
                }
                strings.push(i + 1..j);
                i = j;
            }
            b'{' | b'[' => {
                open.push(composites.len());
                composites.push((i, None));
            }
            b'}' | b']' => {
                if let Some(k) = open.pop() {
                    composites[k].1 = Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    let skip_ws = |mut at: usize| {
        while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        at
    };
    let mut out = Vec::new();
    for (n, key) in strings.iter().enumerate() {
        let colon = skip_ws(key.end + 1);
        if bytes.get(colon) != Some(&b':') {
            continue;
        }
        let v = skip_ws(colon + 1);
        let shape = match bytes.get(v) {
            None => Shape::Unterminated,
            // A quote outside every string opens the next string, or the unterminated one.
            Some(b'"') => match strings.get(n + 1) {
                Some(next) if next.start == v + 1 => Shape::Str(next.clone()),
                _ => Shape::Unterminated,
            },
            Some(b'{' | b'[') => {
                let k = composites.partition_point(|c| c.0 < v);
                match composites.get(k) {
                    Some(&(_, Some(close))) => Shape::Composite(v..close + 1),
                    _ => Shape::Unterminated,
                }
            }
            Some(b'-' | b'0'..=b'9') => {
                let len = bytes[v..]
                    .iter()
                    .take_while(|b| matches!(b, b'0'..=b'9' | b'+' | b'-' | b'.' | b'e' | b'E'))
                    .count();
                Shape::Number(v..v + len)
            }
            Some(_) => Shape::Literal,
        };
        out.push((key.clone(), shape));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shapes(text: &str) -> Vec<String> {
        members(text)
            .into_iter()
            .map(|(k, s)| {
                let v = match s {
                    Shape::Str(r) => format!("str {}", &text[r]),
                    Shape::Number(r) => format!("num {}", &text[r]),
                    Shape::Composite(r) => format!("comp {}", &text[r]),
                    Shape::Literal => "lit".to_owned(),
                    Shape::Unterminated => "cut".to_owned(),
                };
                format!("{}={v}", &text[k])
            })
            .collect()
    }

    #[test]
    fn members_pair_each_key_with_the_shape_of_its_value() {
        let text = r#"{"a":"1", "b" : "2","c":-3.5e2,"d":{"e":"x\"y"},"f":["g","h"],"i":"j\\","t":true,"z":null}"#;
        assert_eq!(
            shapes(text),
            [
                "a=str 1",
                "b=str 2",
                "c=num -3.5e2",
                r#"d=comp {"e":"x\"y"}"#,
                r#"e=str x\"y"#,
                r#"f=comp ["g","h"]"#,
                r"i=str j\\",
                "t=lit",
                "z=lit",
            ]
        );
    }

    #[test]
    fn brackets_inside_strings_do_not_close_a_composite() {
        assert_eq!(
            shapes(r#"{"a":["]",{"b":"}"}],"c":1}"#),
            [r#"a=comp ["]",{"b":"}"}]"#, "b=str }", "c=num 1"]
        );
    }

    #[test]
    fn multibyte_text_keeps_char_boundaries() {
        assert_eq!(shapes(r#"{"é":"ü","k":"v"}"#), ["é=str ü", "k=str v"]);
    }

    #[test]
    fn an_escaped_quote_never_starts_a_key() {
        assert_eq!(
            shapes(r#"{"esc":"a\"password\":\"x"}"#),
            [r#"esc=str a\"password\":\"x"#]
        );
    }

    #[test]
    fn a_value_the_text_ends_inside_is_unterminated() {
        assert_eq!(shapes(r#"{"a":"1","b":"2"#), ["a=str 1", "b=cut"]);
        assert_eq!(shapes(r#"{"a":{"b":1"#), ["a=cut", "b=num 1"]);
        assert_eq!(shapes(r#"{"p":"#), ["p=cut"]);
        assert_eq!(shapes("{\"p\":  "), ["p=cut"]);
    }

    #[test]
    fn deep_nesting_stays_linear() {
        let depth = 200_000;
        let text = format!("{}1{}", r#"{"a":"#.repeat(depth), "}".repeat(depth));
        let start = std::time::Instant::now();
        assert_eq!(members(&text).len(), depth);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(2),
            "{:?}",
            start.elapsed()
        );
    }
}
