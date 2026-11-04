//! The environment grammar (`envToJson`). It lexes differently from the request grammar: a
//! multiline value opens with `'''` and a line break and closes on a line of `'''`, secrets
//! are a comma list, and `extends` takes a name or a list of names.

use super::lex::{
    Cursor, Pair, PairValue, js_slice, key, nl, optionalnl, pairannotations, space, split_lines,
    st, stnl, tagend, unescape_double_quoted,
};
use super::{BruError, annotations_json, apply_data_type, merge_object, named_rows, pairs_to_rows};
use crate::js::{is_js_whitespace, js_trim};
use serde_json::{Map, Value, json};

/// An environment `.bru`, as the reference `envToJson` returns it.
pub(crate) fn parse_environment(text: &str) -> Result<Value, BruError> {
    let mut c = Cursor::new(text);
    let mut blocks = Vec::new();
    while let Some(block) = c.attempt(|c| {
        c.skip_spaces();
        block(c)
    }) {
        blocks.push(block);
    }
    c.expect_end()?;
    if blocks.is_empty() {
        return Ok(json!({ "variables": [] }));
    }
    let mut out = Map::new();
    for block in blocks {
        if let Value::Object(map) = block {
            merge_object(&mut out, &map, true);
        }
    }
    Ok(Value::Object(out))
}

/// `vars | secretvars | externalsecrets | extends | color`, each mapped to its JSON.
fn block(c: &mut Cursor) -> Option<Value> {
    let vars = |c: &mut Cursor| {
        c.attempt(|c| {
            if !c.lit("vars") {
                return None;
            }
            let rows = rows(&dictionary(c)?);
            Some(json!({ "variables": with_secret(rows, false) }))
        })
    };
    let secrets = |c: &mut Cursor| {
        c.attempt(|c| {
            if !c.lit("vars:secret") {
                return None;
            }
            let items: Vec<Pair> = array(c)?
                .into_iter()
                .filter(|p| !p.key.is_empty())
                .collect();
            let mut rows = named_rows(&items);
            for (row, item) in rows.iter_mut().zip(&items) {
                apply_data_type(row, &item.annotations);
            }
            Some(json!({ "variables": with_secret(rows, true) }))
        })
    };
    let external = |c: &mut Cursor| {
        c.attempt(|c| {
            if !c.lit("vars:externalsecrets:") {
                return None;
            }
            let name = c.capture(|c| {
                c.plus(|c| c.not(|c| st(c) || nl(c) || c.lit("{")) && c.any());
            });
            if name.is_empty() {
                return None;
            }
            let variables: Vec<Value> = rows(&dictionary(c)?)
                .into_iter()
                .map(|row| json!({ "name": row["name"], "value": row["value"] }))
                .collect();
            Some(json!({ "externalSecrets": { "type": name, "variables": variables } }))
        })
    };
    let extends = |c: &mut Cursor| {
        c.attempt(|c| {
            if !c.lit("extends") {
                return None;
            }
            let names = extends_list(c);
            let reference = match names {
                Some(names) => validated_names(&names),
                None => {
                    let name = c.attempt(|c| c.lit(":").then(|| single_line(c)))?;
                    validated_name(&name).map(Value::String)
                }
            };
            Some(reference.map_or_else(|| json!({}), |r| json!({ "extends": r })))
        })
    };
    let color =
        |c: &mut Cursor| c.attempt(|c| c.lit("color:").then(|| json!({ "color": single_line(c) })));
    vars(c)
        .or_else(|| secrets(c))
        .or_else(|| external(c))
        .or_else(|| extends(c))
        .or_else(|| color(c))
}

/// `singlelinechar*`, trimmed.
fn single_line(c: &mut Cursor) -> String {
    js_trim(c.capture(|c| {
        c.star(|c| c.not(nl) && c.any());
    }))
    .to_owned()
}

fn multiline_start(c: &mut Cursor) -> bool {
    c.seq(|c| c.lit("'''") && nl(c))
}

fn multiline_end(c: &mut Cursor) -> bool {
    c.seq(|c| {
        nl(c) && {
            c.star(st);
            c.lit("'''")
        }
    })
}

/// The multiline value's content with four columns dropped from every line, trimmed.
fn multiline(c: &mut Cursor) -> Option<String> {
    c.attempt(|c| {
        if !multiline_start(c) {
            return None;
        }
        let content = c.capture(|c| {
            c.star(|c| c.not(multiline_end) && c.any());
        });
        let text = split_lines(content)
            .map(|line| js_slice(line, 4))
            .collect::<Vec<_>>()
            .join("\n");
        multiline_end(c).then(|| js_trim(&text).to_owned())
    })
}

fn pair(c: &mut Cursor) -> Option<Pair> {
    c.attempt(|c| {
        c.star(st);
        let annotations = pairannotations(c);
        c.star(st);
        let key = key(c);
        c.star(st);
        if !c.lit(":") {
            return None;
        }
        c.star(st);
        let value = multiline(c).unwrap_or_else(|| {
            let source = c.capture(|c| {
                c.star(|c| c.not(|c| nl(c) || tagend(c) || multiline_start(c)) && c.any());
            });
            js_trim(source).to_owned()
        });
        c.star(st);
        Some(Pair {
            key,
            value: PairValue::Text(js_trim(&value).to_owned()),
            annotations,
        })
    })
}

/// `st* "{" pairlist? tagend`, with no spaces allowed after `{`.
fn dictionary(c: &mut Cursor) -> Option<Vec<Pair>> {
    c.attempt(|c| {
        c.star(st);
        if !c.lit("{") {
            return None;
        }
        let pairs = c
            .attempt(|c| {
                c.star(optionalnl);
                let mut pairs = vec![pair(c)?];
                while let Some(next) = c.attempt(|c| {
                    if !c.not(tagend) {
                        return None;
                    }
                    c.star(stnl);
                    pair(c)
                }) {
                    pairs.push(next);
                }
                c.star(|c| c.not(tagend) && space(c));
                Some(pairs)
            })
            .unwrap_or_default();
        tagend(c).then_some(pairs)
    })
}

/// `stnl* item stnl* ("," stnl* item)*`, where an item always matches.
fn comma_list<T>(c: &mut Cursor, mut item: impl FnMut(&mut Cursor) -> T) -> Vec<T> {
    c.star(stnl);
    let mut items = vec![item(c)];
    c.star(stnl);
    loop {
        let next = c.attempt(|c| {
            if !c.lit(",") {
                return None;
            }
            c.star(stnl);
            Some(item(c))
        });
        match next {
            Some(found) => items.push(found),
            None => return items,
        }
    }
}

/// `st* "[" stnl* list stnl* "]"`.
fn bracketed<T>(c: &mut Cursor, item: impl FnMut(&mut Cursor) -> T) -> Option<Vec<T>> {
    c.attempt(|c| {
        c.star(st);
        if !c.lit("[") {
            return None;
        }
        c.star(stnl);
        let items = comma_list(c, item);
        c.star(stnl);
        c.lit("]").then_some(items)
    })
}

/// The secret list: each entry is a name with its annotations and an empty value.
fn array(c: &mut Cursor) -> Option<Vec<Pair>> {
    bracketed(c, |c| {
        let annotations = pairannotations(c);
        c.star(st);
        let name = c.capture(|c| {
            c.star(|c| {
                c.not(|c| nl(c) || st(c) || c.lit("[") || c.lit("]") || c.lit(",")) && c.any()
            });
        });
        Pair {
            key: js_trim(name).to_owned(),
            value: PairValue::Text(String::new()),
            annotations,
        }
    })
}

/// `extendslist`: quoted names unescape, bare names trim, and empty names drop out.
fn extends_list(c: &mut Cursor) -> Option<Vec<String>> {
    let names = bracketed(c, |c| {
        let quoted = c.attempt(|c| {
            if !c.lit("\"") {
                return None;
            }
            let source = c.capture(|c| {
                c.star(|c| {
                    c.seq(|c| c.lit("\\") && c.any())
                        || (c.not(|c| c.lit("\"") || nl(c)) && c.any())
                });
            });
            c.lit("\"").then(|| unescape_double_quoted(source))
        });
        quoted.unwrap_or_else(|| {
            let source = c.capture(|c| {
                c.star(|c| c.not(|c| nl(c) || c.lit("[") || c.lit("]") || c.lit(",")) && c.any());
            });
            js_trim(source).to_owned()
        })
    })?;
    Some(names.into_iter().filter(|n| !n.is_empty()).collect())
}

/// `validatedEnvironmentExtendsFrom` for a list: every name must be valid, or none is kept.
fn validated_names(names: &[String]) -> Option<Value> {
    if names.is_empty() {
        return None;
    }
    names
        .iter()
        .map(|n| validated_name(n).map(Value::String))
        .collect::<Option<Vec<_>>>()
        .map(Value::Array)
}

/// `validatedEnvironmentName`: the trimmed name when it could name a file on any platform.
fn validated_name(reference: &str) -> Option<String> {
    let name = js_trim(reference);
    let reserved = ["CON", "PRN", "AUX", "NUL"]
        .iter()
        .any(|r| name.eq_ignore_ascii_case(r))
        || (name.len() == 4
            && ["COM", "LPT"]
                .iter()
                .any(|p| name.is_char_boundary(3) && name[..3].eq_ignore_ascii_case(p))
            && name.as_bytes()[3].is_ascii_digit());
    let invalid_char = name
        .chars()
        .any(|ch| "<>:\"/\\|?*".contains(ch) || ch <= '\u{1f}');
    let valid = !name.is_empty()
        && name.encode_utf16().count() <= 255
        && !reserved
        && !invalid_char
        && !name.starts_with(|ch| ch == '-' || is_js_whitespace(ch))
        && !name.ends_with(|ch| ch == '.' || is_js_whitespace(ch));
    valid.then(|| name.to_owned())
}

/// `mapPairListToKeyValPairs`: rows with typed values, each `@description` beyond the last
/// split out as a description-only row ahead of its variable.
fn rows(pairs: &[Pair]) -> Vec<Map<String, Value>> {
    let mut out = Vec::new();
    for (mut row, pair) in pairs_to_rows(pairs).into_iter().zip(pairs) {
        apply_data_type(&mut row, &pair.annotations);
        let (descriptions, others): (Vec<_>, Vec<_>) = pair
            .annotations
            .iter()
            .cloned()
            .partition(|a| a.name == "description");
        let Some((last, orphans)) = descriptions.split_last().filter(|(_, o)| !o.is_empty()) else {
            out.push(row);
            continue;
        };
        for orphan in orphans {
            let mut extra = Map::new();
            extra.insert("name".into(), json!(""));
            extra.insert("value".into(), json!(""));
            extra.insert("enabled".into(), row["enabled"].clone());
            extra.insert(
                "annotations".into(),
                annotations_json(std::slice::from_ref(orphan)),
            );
            extra.insert(
                "description".into(),
                json!(orphan.value.as_deref().unwrap_or("")),
            );
            out.push(extra);
        }
        let mut kept = others;
        kept.push(last.clone());
        row.insert("annotations".into(), annotations_json(&kept));
        row.insert(
            "description".into(),
            json!(last.value.as_deref().unwrap_or("")),
        );
        out.push(row);
    }
    out
}

fn with_secret(rows: Vec<Map<String, Value>>, secret: bool) -> Vec<Map<String, Value>> {
    rows.into_iter()
        .map(|mut row| {
            row.insert("secret".into(), json!(secret));
            row
        })
        .collect()
}
