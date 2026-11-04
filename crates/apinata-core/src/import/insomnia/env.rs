//! Environments and folder variables. Nested data flattens to `a.b` and `arr[0]` names; a sub
//! environment is walked together with its base, so the base is never cloned per sub, and every
//! flattened node, name and value is charged to the budget because each sub repeats the base.

use super::model::{Unread, leftovers, name_or, rest_paths, sort_key, text};
use super::template::apply;
use super::{Budget, Crumbs};
use crate::edit::{VarValue, Variable};
use crate::env_file::EnvVariable;
use crate::import::{DocumentError, ImportNote, ImportedEnvironment, NoteKind};
use crate::js;
use serde_json::{Map, Value};

/// The top-level key holding Insomnia's encrypted vault values.
const VAULT: &str = "__insomnia_vault";

/// One environment as read from either version.
pub(super) struct EnvSource {
    pub(super) name: String,
    pub(super) id: String,
    pub(super) sort: f64,
    pub(super) data: Option<Value>,
    /// Members of the environment object that are not read, noted on the environment.
    pub(super) unread: Vec<Unread>,
}

impl EnvSource {
    /// A v4 resource's or v5 entry's parts; a missing or blank name takes `default`.
    pub(super) fn new(
        name: Option<&Value>,
        default: &str,
        id: String,
        sort: f64,
        data: Option<Value>,
    ) -> EnvSource {
        EnvSource {
            name: name_or(name, default),
            id,
            sort,
            data,
            unread: Vec::new(),
        }
    }
}

/// Flattened variables beside the member path each one came from.
pub(super) struct Flat {
    pub(super) variables: Vec<EnvVariable>,
    pub(super) sources: Vec<String>,
}

/// What a pending node of the walk is.
#[derive(Clone, Copy, PartialEq)]
enum Slot {
    Root,
    Value,
    /// A vault key; its value is never read.
    Secret,
}

/// One pending node: its flattened name (the bare key for a vault entry) and the base and
/// override layers at it.
struct Frame<'a> {
    name: String,
    low: Option<&'a Value>,
    high: Option<&'a Value>,
    slot: Slot,
}

/// `high` deep-merged over `low` (objects merge, everything else replaces), flattened to text
/// variables in key order with sources `<at>.<name>`. Strings pass through `template::apply`,
/// `null` becomes an empty value with a `Converted` note, and top-level vault keys become
/// valueless secrets named `vault.<key>`. Notes are sourced `trail / <at>.<name>`. The walk
/// keeps its own stack and charges each name and value before building it.
pub(super) fn flatten(
    low: Option<&Value>,
    high: Option<&Value>,
    at: &str,
    trail: &[String],
    notes: &mut Vec<ImportNote>,
    budget: &mut Budget,
) -> Result<Flat, DocumentError> {
    let mut flat = Flat {
        variables: Vec::new(),
        sources: Vec::new(),
    };
    let mut stack = vec![Frame {
        name: String::new(),
        low,
        high,
        slot: Slot::Root,
    }];
    while let Some(frame) = stack.pop() {
        let (low, high) = layers(frame.low, frame.high);
        if frame.slot == Slot::Secret {
            let source = joined(budget, &[at, ".", VAULT, ".", &frame.name])?;
            let name = joined(budget, &["vault.", &frame.name])?;
            flat.variables.push(EnvVariable::secret(name));
            flat.sources.push(source);
            continue;
        }
        let Some(value) = high.or(low) else { continue };
        match value {
            Value::Object(_) => {
                for (key, l, h) in entries(low, high).into_iter().rev() {
                    if frame.slot == Slot::Root && key == VAULT {
                        vault(l, h, at, trail, &mut stack, notes, budget)?;
                        continue;
                    }
                    if frame.slot == Slot::Root && key.is_empty() {
                        let source = joined(budget, &[at, "."])?;
                        let detail = "the variable has no name; not imported";
                        notes.push(budget.note(trail, &source, NoteKind::Dropped, detail)?);
                        continue;
                    }
                    budget.entry()?;
                    let name = if frame.slot == Slot::Root {
                        joined(budget, &[key])?
                    } else {
                        joined(budget, &[&frame.name, ".", key])?
                    };
                    stack.push(Frame {
                        name,
                        low: l,
                        high: h,
                        slot: Slot::Value,
                    });
                }
            }
            Value::Array(list) => {
                for (i, item) in list.iter().enumerate().rev() {
                    budget.entry()?;
                    let index = format!("[{i}]");
                    stack.push(Frame {
                        name: joined(budget, &[&frame.name, &index])?,
                        low: None,
                        high: Some(item),
                        slot: Slot::Value,
                    });
                }
            }
            scalar => {
                let source = joined(budget, &[at, ".", &frame.name])?;
                let value = leaf(scalar, trail, &source, notes, budget)?;
                flat.variables
                    .push(EnvVariable::new(frame.name, VarValue::Text(value)));
                flat.sources.push(source);
            }
        }
    }
    Ok(flat)
}

/// The override replaces the base unless both are objects, which merge.
fn layers<'a>(
    low: Option<&'a Value>,
    high: Option<&'a Value>,
) -> (Option<&'a Value>, Option<&'a Value>) {
    match (low, high) {
        (Some(l @ Value::Object(_)), Some(h @ Value::Object(_))) => (Some(l), Some(h)),
        (l, None) => (l, None),
        (_, h) => (None, h),
    }
}

/// The keys of the merged object: the base's in order, then the override's new ones.
fn entries<'a>(
    low: Option<&'a Value>,
    high: Option<&'a Value>,
) -> Vec<(&'a str, Option<&'a Value>, Option<&'a Value>)> {
    let low: Option<&Map<String, Value>> = low.and_then(Value::as_object);
    let high: Option<&Map<String, Value>> = high.and_then(Value::as_object);
    let mut out = Vec::new();
    for (k, v) in low.into_iter().flatten() {
        out.push((k.as_str(), Some(v), high.and_then(|h| h.get(k))));
    }
    for (k, v) in high.into_iter().flatten() {
        if !low.is_some_and(|l| l.contains_key(k)) {
            out.push((k.as_str(), None, Some(v)));
        }
    }
    out
}

/// Queue the merged vault's keys as secrets; a vault that is not an object is noted.
fn vault<'a>(
    low: Option<&'a Value>,
    high: Option<&'a Value>,
    at: &str,
    trail: &[String],
    stack: &mut Vec<Frame<'a>>,
    notes: &mut Vec<ImportNote>,
    budget: &mut Budget,
) -> Result<(), DocumentError> {
    let (low, high) = layers(low, high);
    if !high.or(low).is_some_and(Value::is_object) {
        let source = joined(budget, &[at, ".", VAULT])?;
        let detail = "the vault is not an object; not imported";
        notes.push(budget.note(trail, &source, NoteKind::Dropped, detail)?);
        return Ok(());
    }
    for (key, _, _) in entries(low, high).into_iter().rev() {
        budget.entry()?;
        stack.push(Frame {
            name: joined(budget, &[key])?,
            low: None,
            high: None,
            slot: Slot::Secret,
        });
    }
    Ok(())
}

/// A scalar as variable text.
fn leaf(
    scalar: &Value,
    trail: &[String],
    source: &str,
    notes: &mut Vec<ImportNote>,
    budget: &mut Budget,
) -> Result<String, DocumentError> {
    match scalar {
        // Insomnia renders `null` as an empty string, never as the text `null`.
        Value::Null => {
            let detail = "null value imported as empty";
            notes.push(budget.note(trail, source, NoteKind::Converted, detail)?);
            Ok(String::new())
        }
        Value::String(s) => {
            budget.charge(s.len())?;
            apply(s, trail, source, notes, budget)
        }
        other => {
            let text = js::to_text(other);
            budget.charge(text.len())?;
            Ok(text)
        }
    }
}

/// `parts` joined, charged before it is built; every node costs at least one byte.
fn joined(budget: &mut Budget, parts: &[&str]) -> Result<String, DocumentError> {
    let len: usize = parts.iter().map(|p| p.len()).sum();
    budget.charge(len + 1)?;
    Ok(parts.concat())
}

/// An environment's `data` when it is an object; `null` or missing is empty, anything else is
/// noted at `at`.
fn object(
    data: Option<Value>,
    at: &str,
    trail: &[String],
    notes: &mut Vec<ImportNote>,
    budget: &mut Budget,
) -> Result<Option<Value>, DocumentError> {
    match data {
        None | Some(Value::Null) => Ok(None),
        Some(v @ Value::Object(_)) => Ok(Some(v)),
        Some(_) => {
            let detail = "the value is not an object; not imported";
            notes.push(budget.note(trail, at, NoteKind::Dropped, detail)?);
            Ok(None)
        }
    }
}

/// The base under its own name, then each sub in sibling order with the base merged under it.
pub(super) fn environments(
    base: EnvSource,
    mut subs: Vec<EnvSource>,
    budget: &mut Budget,
) -> Result<Vec<ImportedEnvironment>, DocumentError> {
    subs.sort_by(|a, b| a.sort.total_cmp(&b.sort).then_with(|| b.id.cmp(&a.id)));
    let mut out = Vec::with_capacity(subs.len() + 1);
    let mut notes = Vec::new();
    budget.unread(&[], base.unread, &mut notes)?;
    let base_data = object(base.data, "data", &[], &mut notes, budget)?;
    let flat = flatten(base_data.as_ref(), None, "data", &[], &mut notes, budget)?;
    out.push(environment(base.name, flat, notes));
    for sub in subs {
        let mut notes = Vec::new();
        budget.unread(&[], sub.unread, &mut notes)?;
        let data = object(sub.data, "data", &[], &mut notes, budget)?;
        let flat = flatten(
            base_data.as_ref(),
            data.as_ref(),
            "data",
            &[],
            &mut notes,
            budget,
        )?;
        out.push(environment(sub.name, flat, notes));
    }
    Ok(out)
}

fn environment(name: String, flat: Flat, notes: Vec<ImportNote>) -> ImportedEnvironment {
    ImportedEnvironment {
        name,
        variables: flat.variables,
        notes,
        sources: flat.sources,
    }
}

/// A v5 `environments` object: its base and `subEnvironments`. A malformed part is noted in
/// `unwritten`.
pub(super) fn v5(
    value: Option<Value>,
    unwritten: &mut Vec<ImportNote>,
    budget: &mut Budget,
) -> Result<Vec<ImportedEnvironment>, DocumentError> {
    let mut map = match value {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Object(map)) => map,
        Some(_) => {
            let detail = "the environments are not an object; not imported";
            unwritten.push(budget.note(&[], "environments", NoteKind::Dropped, detail)?);
            return Ok(Vec::new());
        }
    };
    let sub_list = map.remove("subEnvironments");
    let base = v5_source(&mut map, "Base Environment");
    let mut subs = Vec::new();
    match sub_list {
        None | Some(Value::Null) => {}
        Some(Value::Array(list)) => {
            for (i, entry) in list.into_iter().enumerate() {
                if let Value::Object(mut entry) = entry {
                    subs.push(v5_source(&mut entry, "New Environment"));
                } else {
                    let source = format!("environments.subEnvironments[{i}]");
                    let detail = "the environment is not an object; not imported";
                    unwritten.push(budget.note(&[], &source, NoteKind::Dropped, detail)?);
                }
            }
        }
        Some(_) => {
            let source = "environments.subEnvironments";
            let detail = "the sub environments are not a list; not imported";
            unwritten.push(budget.note(&[], source, NoteKind::Dropped, detail)?);
        }
    }
    environments(base, subs, budget)
}

fn v5_source(entry: &mut Map<String, Value>, default: &str) -> EnvSource {
    let data = entry.remove("data");
    let meta = entry.get("meta");
    let id = text(meta.and_then(|m| m.get("id")));
    let sort = sort_key(meta.and_then(|m| m.get("sortKey")));
    let mut source = EnvSource::new(entry.get("name"), default, id, sort, data);
    rest_paths(entry, "", &["name", "meta"], &mut source.unread);
    leftovers(meta, "meta", &["id", "sortKey"], &mut source.unread);
    source
}

/// A folder's `environment` as folder variables, notes sourced `trail / environment.<name>`.
/// Vault keys have no folder counterpart and are noted instead.
pub(super) fn folder_vars(
    member: Option<super::model::Member>,
    trail: &[String],
    notes: &mut Vec<ImportNote>,
    budget: &mut Budget,
) -> Result<Vec<Variable>, DocumentError> {
    let Some(member) = member else {
        return Ok(Vec::new());
    };
    let data = object(Some(member.value), &member.path, trail, notes, budget)?;
    let flat = flatten(None, data.as_ref(), &member.path, trail, notes, budget)?;
    let mut vars = Vec::with_capacity(flat.variables.len());
    for (var, source) in flat.variables.into_iter().zip(flat.sources) {
        if var.secret {
            let detail = "vault values are not imported into folder variables";
            notes.push(budget.note(trail, &source, NoteKind::Dropped, detail)?);
        } else {
            vars.push(Variable::new(var.name, var.value));
        }
    }
    Ok(vars)
}
