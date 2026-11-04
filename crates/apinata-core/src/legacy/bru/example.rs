//! The example grammars: an `example` block's content (`example/bruToJson`) and its `request`
//! and `response` sub-grammars, mapped with the shared example semantics rather than the
//! request grammar's (no annotations, and a list value has no semantic action).

use super::lex::{
    Cursor, Pair, PairValue, example_dictionary, multiline_parts, nl, outdent, outdent_by, st,
    tagend, text_block, valuechar,
};
use super::request::{file_rows, multipart, params};
use super::{BruError, is_truthy, merge_object, pair_value, pairs_to_rows};
use crate::js::js_trim;
use serde_json::{Map, Value, json};

type Fields = Map<String, Value>;

/// One `example` block's raw content as the reference returns it: the parsed example, or
/// `{error}` when any of its grammars rejects it, which never rejects the file.
pub(super) fn example_json(raw: &str) -> Value {
    match parse_example(&unindent(raw)) {
        Ok(example) => Value::Object(example),
        Err(message) => json!({ "error": message }),
    }
}

/// `parseExampleContent`'s preparation: drop the smallest indent of the non-blank lines,
/// keep blank lines as they are, and trim the whole.
fn unindent(raw: &str) -> String {
    let lines: Vec<&str> = raw.split('\n').collect();
    let blank = |line: &str| js_trim(line).is_empty();
    let min = lines
        .iter()
        .filter(|l| !blank(l))
        .map(|l| l.len() - l.trim_start_matches([' ', '\t']).len())
        .min()
        .unwrap_or(0);
    let joined = lines
        .iter()
        .map(|l| if blank(l) { *l } else { &l[min..] })
        .collect::<Vec<_>>()
        .join("\n");
    js_trim(&joined).to_owned()
}

fn syntax_error(error: BruError) -> String {
    format!("Line {}: {}", error.line, error.message)
}

const LIST_ERROR: &str = "a list is not a valid example value";

enum Item<'a> {
    Field(&'static str, String),
    Request(&'a str),
    Response(&'a str),
}

/// `ExampleFile = (name | description | request | response)*`; the sub-grammars run only
/// once the whole content has matched, as ohm's semantics do, and fold with `_.merge`.
fn parse_example(text: &str) -> Result<Fields, String> {
    let mut c = Cursor::new(text);
    let mut items = Vec::new();
    while let Some(item) = c.attempt(|c| {
        c.skip_spaces();
        name(c)
            .or_else(|| description(c))
            .or_else(|| section(c, "request").map(Item::Request))
            .or_else(|| section(c, "response").map(Item::Response))
    }) {
        items.push(item);
    }
    c.expect_end().map_err(syntax_error)?;
    let mut out = Map::new();
    for item in items {
        let mut fields = Map::new();
        match item {
            Item::Field(key, value) => fields.insert(key.into(), json!(value)),
            Item::Request(content) => {
                fields.insert("request".into(), json!(parse_request(&outdent(content))?))
            }
            Item::Response(content) => {
                fields.insert("response".into(), json!(parse_response(&outdent(content))?))
            }
        };
        merge_object(&mut out, &fields, false);
    }
    Ok(out)
}

/// `keyword st* ":" st*`, the head every example field and sub-grammar entry shares.
fn head(c: &mut Cursor, keyword: &str) -> bool {
    c.lit(keyword)
        && {
            c.star(st);
            c.lit(":")
        }
        && {
            c.star(st);
            true
        }
}

fn rest_of_line<'a>(c: &mut Cursor<'a>) -> &'a str {
    c.capture(|c| {
        c.star(valuechar);
    })
}

fn name<'a>(c: &mut Cursor<'a>) -> Option<Item<'a>> {
    c.attempt(|c| {
        if !head(c, "name") {
            return None;
        }
        let value = js_trim(rest_of_line(c)).to_owned();
        c.star(st);
        Some(Item::Field("name", value))
    })
}

/// A `'''` description is outdented by two spaces, unlike a sub-grammar's multiline value.
fn description<'a>(c: &mut Cursor<'a>) -> Option<Item<'a>> {
    c.attempt(|c| {
        if !head(c, "description") {
            return None;
        }
        let value = match multiline_parts(c) {
            Some((content, "")) => outdent(content),
            Some((content, content_type)) => format!("{} {content_type}", outdent(content)),
            None => js_trim(rest_of_line(c)).to_owned(),
        };
        c.star(st);
        Some(Item::Field("description", js_trim(&value).to_owned()))
    })
}

/// `request` or `response`: `keyword st* ":" st* "{" nl* content nl* "}" nl*`, where the
/// content runs to the first line that starts with `}`.
fn section<'a>(c: &mut Cursor<'a>, keyword: &str) -> Option<&'a str> {
    c.attempt(|c| {
        c.star(nl);
        if !(head(c, keyword) && c.lit("{")) {
            return None;
        }
        c.star(nl);
        let content = c.capture(|c| {
            c.star(|c| c.not(tagend) && c.any());
        });
        if content.is_empty() {
            return None;
        }
        c.star(nl);
        if !c.lit("}") {
            return None;
        }
        c.star(nl);
        Some(content)
    })
}

/// A sub-grammar's start rule: `entry*` with space skipping, then end of input.
fn entries<'a, T>(
    text: &'a str,
    mut entry: impl FnMut(&mut Cursor<'a>) -> Option<T>,
) -> Result<Vec<T>, String> {
    let mut c = Cursor::new(text);
    let mut out = Vec::new();
    while let Some(found) = c.attempt(|c| {
        c.skip_spaces();
        entry(c)
    }) {
        out.push(found);
    }
    c.expect_end().map_err(syntax_error)?;
    Ok(out)
}

#[derive(Clone, Copy)]
enum Shape {
    Line,
    Dictionary,
    Text,
}

/// The request sub-grammar's entries in its ordered-choice order.
const REQUEST_ENTRIES: &[(&str, Shape)] = &[
    ("url", Shape::Line),
    ("method", Shape::Line),
    ("mode", Shape::Line),
    ("params:path", Shape::Dictionary),
    ("params:query", Shape::Dictionary),
    ("headers", Shape::Dictionary),
    ("body:json", Shape::Text),
    ("body:text", Shape::Text),
    ("body:xml", Shape::Text),
    ("body:sparql", Shape::Text),
    ("body:graphql", Shape::Text),
    ("body:graphql:vars", Shape::Text),
    ("body:form-urlencoded", Shape::Dictionary),
    ("body:multipart-form", Shape::Dictionary),
    ("body:file", Shape::Dictionary),
];

enum Entry {
    Line(String),
    Pairs(Vec<Pair>),
    Text(String),
}

fn parse_request(text: &str) -> Result<Fields, String> {
    let found = entries(text, |c| {
        REQUEST_ENTRIES.iter().find_map(|&(keyword, shape)| {
            c.attempt(|c| {
                if !head(c, keyword) {
                    return None;
                }
                let entry = match shape {
                    Shape::Line => Entry::Line(js_trim(rest_of_line(c)).to_owned()),
                    Shape::Dictionary => Entry::Pairs(example_dictionary(c)?),
                    Shape::Text => Entry::Text(outdent(&text_block(c)?)),
                };
                Some((keyword, entry))
            })
        })
    })?;
    let mut out = Map::new();
    for (keyword, entry) in found {
        merge_object(&mut out, &request_entry(keyword, entry)?, true);
    }
    Ok(out)
}

fn request_entry(keyword: &str, entry: Entry) -> Result<Fields, String> {
    let value = match (keyword, entry) {
        ("mode", Entry::Line(mode)) => {
            let mode = if mode.is_empty() { "none".into() } else { mode };
            json!({ "body": { "mode": mode } })
        }
        (key, Entry::Line(value)) => json!({ key: value }),
        (key, Entry::Text(text)) => {
            let (mode, field) = match key {
                "body:graphql" => ("graphql", json!({ "graphql": { "query": text } })),
                "body:graphql:vars" => ("graphql", json!({ "graphql": { "variables": text } })),
                other => {
                    let mode = other.trim_start_matches("body:");
                    (mode, json!({ mode: text }))
                }
            };
            let mut body = field;
            body["mode"] = json!(mode);
            json!({ "body": body })
        }
        (key, Entry::Pairs(pairs)) => {
            if pairs.iter().any(|p| matches!(p.value, PairValue::List(_))) {
                return Err(LIST_ERROR.into());
            }
            match key {
                "params:path" => json!({ "params": params(&pairs, "path") }),
                "params:query" => json!({ "params": params(&pairs, "query") }),
                "headers" => json!({ "headers": pairs_to_rows(&pairs) }),
                "body:form-urlencoded" => json!({ "body": {
                    "mode": "formUrlEncoded", "formUrlEncoded": pairs_to_rows(&pairs),
                } }),
                "body:multipart-form" => json!({ "body": {
                    "mode": "multipartForm", "multipartForm": multipart(&pairs, false),
                } }),
                "body:file" => json!({ "body": { "mode": "file", "file": file_rows(&pairs) } }),
                other => unreachable!("{other} is not a dictionary entry"),
            }
        }
    };
    match value {
        Value::Object(fields) => Ok(fields),
        _ => unreachable!("every entry maps to an object"),
    }
}

enum ResponseEntry {
    Headers(Vec<Pair>),
    Status(Vec<Pair>),
    Body(Vec<(&'static str, String)>),
}

fn parse_response(text: &str) -> Result<Fields, String> {
    let found = entries(text, |c| {
        dictionary_entry(c, "headers")
            .map(ResponseEntry::Headers)
            .or_else(|| dictionary_entry(c, "status").map(ResponseEntry::Status))
            .or_else(|| response_body(c).map(ResponseEntry::Body))
    })?;
    let mut out = Map::new();
    for entry in found {
        let fields = match entry {
            ResponseEntry::Headers(pairs) | ResponseEntry::Status(pairs)
                if pairs.iter().any(|p| matches!(p.value, PairValue::List(_))) =>
            {
                return Err(LIST_ERROR.into());
            }
            ResponseEntry::Headers(pairs) => json!({ "headers": pairs_to_rows(&pairs) }),
            ResponseEntry::Status(pairs) => {
                // The first `code` or `text` pair wins, and an empty one falls back.
                let first = |name: &str| {
                    pairs
                        .iter()
                        .find(|p| p.key == name)
                        .map(|p| pair_value(&p.value))
                        .filter(|v| is_truthy(Some(v)))
                };
                json!({
                    "status": first("code").unwrap_or(json!(200)),
                    "statusText": first("text").unwrap_or(json!("OK")),
                })
            }
            ResponseEntry::Body(fields) => {
                let mut body = Map::new();
                for (key, value) in fields {
                    body.insert(key.into(), json!(value));
                }
                json!({ "body": body })
            }
        };
        if let Value::Object(fields) = fields {
            merge_object(&mut out, &fields, false);
        }
    }
    Ok(out)
}

/// `keyword st* ":" st* dictionary nl*`.
fn dictionary_entry(c: &mut Cursor, keyword: &str) -> Option<Vec<Pair>> {
    c.attempt(|c| {
        if !head(c, keyword) {
            return None;
        }
        let pairs = example_dictionary(c)?;
        c.star(nl);
        Some(pairs)
    })
}

/// `"body" st* ":" st* "{" nl* (type | content)* tagend`; a `type` line swallows the line
/// breaks after it, so a body whose last field is `type` never finds its closing brace.
fn response_body(c: &mut Cursor) -> Option<Vec<(&'static str, String)>> {
    c.attempt(|c| {
        if !(head(c, "body") && c.lit("{")) {
            return None;
        }
        c.star(nl);
        let mut fields = Vec::new();
        while let Some(field) = body_type(c).or_else(|| body_content(c)) {
            fields.push(field);
        }
        tagend(c).then_some(fields)
    })
}

fn body_type(c: &mut Cursor) -> Option<(&'static str, String)> {
    c.attempt(|c| {
        c.star(st);
        if !head(c, "type") {
            return None;
        }
        let value = js_trim(rest_of_line(c)).to_owned();
        c.star(nl);
        Some(("type", value))
    })
}

/// The content's source with its delimiters, one trailing two-space indent and one edge
/// line break removed, then outdented by four spaces.
fn body_content(c: &mut Cursor) -> Option<(&'static str, String)> {
    c.attempt(|c| {
        c.star(st);
        if !head(c, "content") {
            return None;
        }
        let mut matched = false;
        let source = c.capture(|c| matched = multiline_parts(c).is_some());
        if !matched {
            return None;
        }
        let text = source.strip_prefix("'''").unwrap_or(source);
        let text = text.strip_suffix("'''").unwrap_or(text);
        let text = text.strip_suffix("  ").unwrap_or(text);
        let text = text.strip_prefix('\n').unwrap_or(text);
        let text = text.strip_suffix('\n').unwrap_or(text);
        Some(("content", outdent_by(text, "    ")))
    })
}
