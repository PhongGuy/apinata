//! The collection grammar (`collectionBruToJson`), read for both `collection.bru` and
//! `folder.bru`: its block table in the reference's order, and each block's semantic action.

use super::auth::{AUTH_BLOCKS, auth_block, auth_block_error};
use super::lex::{BlockBody, BlockKind, Dialect, Pair, outdent, parse_blocks};
use super::request::vars;
use super::{BruError, is_truthy, merge_object, pairs_to_object, pairs_to_rows};
use serde_json::{Map, Value, json};

const BLOCKS_BEFORE_AUTH: &[(&str, BlockKind)] = &[
    ("meta", BlockKind::Dictionary),
    ("query", BlockKind::Dictionary),
    ("headers", BlockKind::Dictionary),
    ("auth", BlockKind::Dictionary),
];

const BLOCKS_AFTER_AUTH: &[(&str, BlockKind)] = &[
    ("vars:pre-request", BlockKind::Dictionary),
    ("vars:post-response", BlockKind::Dictionary),
    ("script:pre-request", BlockKind::Text),
    ("script:post-response", BlockKind::Text),
    ("tests", BlockKind::Text),
    ("docs", BlockKind::Text),
];

/// `collection.bru` and `folder.bru`, as the reference `collectionBruToJson` returns them.
pub(crate) fn parse_collection(text: &str) -> Result<Value, BruError> {
    let table = [BLOCKS_BEFORE_AUTH, AUTH_BLOCKS, BLOCKS_AFTER_AUTH].concat();
    let blocks = parse_blocks(text, &table, Dialect::Collection)?;
    if let Some(error) = blocks.iter().find_map(auth_block_error) {
        return Err(error);
    }
    let mut out = Map::new();
    for block in blocks {
        let value = match &block.body {
            BlockBody::Pairs(pairs) => pairs_block(block.keyword, pairs),
            BlockBody::Text(source) => text_block(block.keyword, outdent(source)),
        };
        if let Value::Object(map) = value {
            merge_object(&mut out, &map, true);
        }
    }
    Ok(Value::Object(out))
}

fn pairs_block(keyword: &str, pairs: &[Pair]) -> Value {
    if let Some(auth) = auth_block(keyword, pairs) {
        return auth;
    }
    match keyword {
        // Unlike a request, the meta type is always overwritten and `seq` has no default.
        "meta" => {
            let mut meta = pairs_to_object(pairs);
            meta.insert("type".into(), json!("collection"));
            json!({ "meta": meta })
        }
        "auth" => {
            let mode = pairs_to_object(pairs)
                .shift_remove("mode")
                .filter(|m| is_truthy(Some(m)))
                .unwrap_or_else(|| json!("none"));
            json!({ "auth": { "mode": mode } })
        }
        "query" => json!({ "query": pairs_to_rows(pairs) }),
        "headers" => json!({ "headers": pairs_to_rows(pairs) }),
        "vars:pre-request" => json!({ "vars": { "req": vars(pairs, true) } }),
        "vars:post-response" => json!({ "vars": { "res": vars(pairs, false) } }),
        other => unreachable!("{other} is not a collection dictionary block"),
    }
}

fn text_block(keyword: &str, text: String) -> Value {
    match keyword {
        "script:pre-request" => json!({ "script": { "req": text } }),
        "script:post-response" => json!({ "script": { "res": text } }),
        "tests" => json!({ "tests": text }),
        "docs" => json!({ "docs": text }),
        other => unreachable!("{other} is not a collection text block"),
    }
}
