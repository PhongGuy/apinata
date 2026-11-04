//! Reading and replacing one phase's script in a file's `scripts` list.

use super::ScriptPhase;
use crate::js::script_code;
use yaml_serde::{Mapping, Value};

const PHASES: [ScriptPhase; 3] = [
    ScriptPhase::PreRequest,
    ScriptPhase::PostResponse,
    ScriptPhase::Tests,
];

fn entry_type(item: &Value) -> Option<&str> {
    item.get("type").and_then(Value::as_str)
}

fn position(items: &[Value], phase: ScriptPhase) -> Option<usize> {
    items
        .iter()
        .position(|i| entry_type(i) == Some(phase.script_type()))
}

/// The code of the first entry of `phase`'s type in `list`; empty when there is none.
pub(crate) fn script_in(list: Option<&Value>, phase: ScriptPhase) -> String {
    let items = list.and_then(Value::as_sequence).map(Vec::as_slice);
    items
        .and_then(|items| Some(&items[position(items, phase)?]))
        .and_then(|item| item.get("code"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// `list` with the first entry of `phase` holding `script_code(code)`, or without it when that
/// is blank. Other entries keep their place; a new entry goes after those of earlier phases.
pub(crate) fn with_script(list: Option<&Value>, phase: ScriptPhase, code: &str) -> Vec<Value> {
    let mut items = list
        .and_then(Value::as_sequence)
        .cloned()
        .unwrap_or_default();
    let code = script_code(code);
    match (position(&items, phase), code.is_empty()) {
        (Some(at), true) => {
            items.remove(at);
        }
        (Some(at), false) => {
            if let Value::Mapping(map) = &mut items[at] {
                map.insert(Value::from("code"), Value::from(code));
            }
        }
        (None, true) => {}
        (None, false) => {
            let earlier: Vec<&str> = PHASES
                .iter()
                .take_while(|p| **p != phase)
                .map(|p| p.script_type())
                .collect();
            let at = items
                .iter()
                .rposition(|i| entry_type(i).is_some_and(|t| earlier.contains(&t)))
                .map_or(0, |i| i + 1);
            let mut map = Mapping::new();
            map.insert(Value::from("type"), Value::from(phase.script_type()));
            map.insert(Value::from("code"), Value::from(code));
            items.insert(at, Value::Mapping(map));
        }
    }
    items
}
