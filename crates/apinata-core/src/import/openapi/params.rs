//! Parameters: a path item's and an operation's parameters become a request's params and headers.

use super::Doc;
use super::bounds::approx_size;
use super::operations::Operation;
use super::refs::RefKind;
use super::version::Version;
use crate::edit::{KeyValue, Param, ParamKind};
use crate::import::{DocumentError, ImportNote, ImportedRequest, NoteKind};
use crate::js;
use serde_json::Value;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

/// One resolved parameter with its document path.
pub(crate) type Resolved<'v> = (Cow<'v, Value>, String);

/// One param or header a parameter gives.
struct Entry {
    name: String,
    value: String,
    enabled: bool,
    description: Option<String>,
}

/// What one parameter (or one exploded property) reads its value from.
struct Fields<'p> {
    /// The parameter's document path, for refs under its `examples`.
    source: &'p str,
    example: Option<&'p Value>,
    examples: Option<&'p Value>,
    schema: Option<&'p Value>,
    required: bool,
    allow_empty: bool,
}

/// Path-item and operation parameters, merged by (name, in), into params and headers.
pub(crate) fn apply<'a: 'o, 'o>(
    doc: &mut Doc<'a>,
    op: &'o Operation<'_>,
    request: &mut ImportedRequest,
    notes: &mut Vec<ImportNote>,
) -> Result<(), DocumentError> {
    let (inherited, own) = resolve_lists(doc, op, true)?;
    notes.extend(doc.take_ref_notes());
    for (param, source) in merge(&inherited, &own) {
        let Some((name, location)) = key(param) else {
            drop(
                doc,
                notes,
                source,
                "not a parameter object, so it was not imported".into(),
            )?;
            continue;
        };
        // A header has no param kind. Swagger 2.0 bodies are left to the body import; every
        // other location, and headers the request sets itself, become notes.
        let kind = match location {
            "path" => Some(ParamKind::Path),
            "query" => Some(ParamKind::Query),
            "header" if is_reserved_header(name) => {
                doc.budget.charge_text(name)?;
                let detail = format!(
                    "the `{name}` header parameter is not imported, since the request sets that header itself"
                );
                drop(doc, notes, source, detail)?;
                continue;
            }
            "header" => None,
            "body" | "formData" if doc.version == Version::Swagger2 => continue,
            "querystring" => {
                drop(
                    doc,
                    notes,
                    source,
                    "in: querystring (OpenAPI 3.2) is not imported".into(),
                )?;
                continue;
            }
            "cookie" => {
                doc.budget.charge_text(name)?;
                let detail = format!("the cookie parameter `{name}` is not imported");
                drop(doc, notes, source, detail)?;
                continue;
            }
            other => {
                doc.budget.charge_text(name)?;
                doc.budget.charge_text(other)?;
                let detail = format!(
                    "the parameter `{name}` in `{other}` is not imported, since that location is unknown"
                );
                drop(doc, notes, source, detail)?;
                continue;
            }
        };
        for entry in entries(doc, param, name, location, source, notes)? {
            if let Some(kind) = kind {
                let mut out = Param::new(entry.name, entry.value, kind);
                out.enabled = entry.enabled;
                out.description = entry.description;
                request.params.push(out);
            } else {
                let mut out = KeyValue::new(entry.name, entry.value);
                out.enabled = entry.enabled;
                out.description = entry.description;
                request.headers.push(out);
            }
        }
        notes.extend(doc.take_ref_notes());
    }
    append_query(doc, request)
}

/// The path item's and the operation's `parameters`, each entry resolved at its document path.
/// `charge` charges one entry per occurrence; a second pass over the same lists passes false.
pub(crate) fn resolve_lists<'a: 'o, 'o>(
    doc: &mut Doc<'a>,
    op: &'o Operation<'_>,
    charge: bool,
) -> Result<(Vec<Resolved<'o>>, Vec<Resolved<'o>>), DocumentError> {
    let path = format!("paths.{}.parameters", op.path);
    let inherited = resolve_list(doc, op.item.get("parameters"), &path, charge)?;
    let own_path = format!("{}.parameters", op.source);
    let own = resolve_list(doc, op.op().get("parameters"), &own_path, charge)?;
    Ok((inherited, own))
}

/// Bruno's `mergeParams`: inherited parameters the operation does not override by
/// `(name, in)`, then its own, each list deduplicated first.
pub(crate) fn merge<'l>(
    inherited: &'l [Resolved<'_>],
    own: &'l [Resolved<'_>],
) -> Vec<(&'l Value, &'l str)> {
    let own = dedupe(own);
    let overrides: HashSet<(&str, &str)> = own.iter().filter_map(|(p, _)| key(p)).collect();
    dedupe(inherited)
        .into_iter()
        .filter(|(p, _)| key(p).is_none_or(|k| !overrides.contains(&k)))
        .chain(own)
        .collect()
}

/// Each entry of a `parameters` list resolved, charging one entry per occurrence when `charge`
/// is set, so refs that later merge away still cost what they fanned out to.
fn resolve_list<'a: 'v, 'v>(
    doc: &mut Doc<'a>,
    list: Option<&'v Value>,
    source: &str,
    charge: bool,
) -> Result<Vec<Resolved<'v>>, DocumentError> {
    let mut out = Vec::new();
    for (i, raw) in list
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        if charge {
            doc.budget.charge(1)?;
        }
        let source = format!("{source}[{i}]");
        out.push((doc.resolve(raw, RefKind::Object, &source)?, source));
    }
    Ok(out)
}

/// Push a `Dropped` note about the parameter at `source`, charging it first.
fn drop(
    doc: &mut Doc<'_>,
    notes: &mut Vec<ImportNote>,
    source: &str,
    detail: String,
) -> Result<(), DocumentError> {
    doc.budget.charge(1)?;
    doc.budget
        .charge_bytes((source.len() + detail.len()) as u64)?;
    notes.push(ImportNote::new(source, NoteKind::Dropped, detail));
    Ok(())
}

/// Push a `Converted` note about the parameter at `source`, charging it like `drop`.
fn converted(
    doc: &mut Doc<'_>,
    notes: &mut Vec<ImportNote>,
    source: &str,
    detail: String,
) -> Result<(), DocumentError> {
    doc.budget.charge(1)?;
    doc.budget
        .charge_bytes((source.len() + detail.len()) as u64)?;
    notes.push(ImportNote::new(source, NoteKind::Converted, detail));
    Ok(())
}

/// Whether `apply` (or, for Swagger 2.0 `body` and `formData`, the body import) carries the
/// parameter over, rather than dropping it with a note.
pub(crate) fn is_imported(param: &Value, version: Version) -> bool {
    match key(param) {
        Some((name, "header")) => !is_reserved_header(name),
        Some((_, "path" | "query")) => true,
        Some((_, "body" | "formData")) => version == Version::Swagger2,
        _ => false,
    }
}

/// Headers the request sets from its body and auth, which a parameter must not override.
fn is_reserved_header(name: &str) -> bool {
    ["accept", "content-type", "authorization"]
        .iter()
        .any(|reserved| name.eq_ignore_ascii_case(reserved))
}

/// A parameter's `(name, in)`, when both are strings.
fn key(param: &Value) -> Option<(&str, &str)> {
    Some((param.get("name")?.as_str()?, param.get("in")?.as_str()?))
}

/// One list with a later duplicate `(name, in)` taking the earlier one's place.
fn dedupe<'l, 'v>(list: &'l [Resolved<'v>]) -> Vec<(&'l Value, &'l str)> {
    let mut out: Vec<(&Value, &str)> = Vec::new();
    let mut at: HashMap<(&str, &str), usize> = HashMap::new();
    for (param, source) in list {
        let entry = (param.as_ref(), source.as_str());
        match key(param).map(|k| (k, at.get(&k).copied())) {
            Some((_, Some(i))) => out[i] = entry,
            Some((k, None)) => {
                at.insert(k, out.len());
                out.push(entry);
            }
            None => out.push(entry),
        }
    }
    out
}

/// The entries one parameter gives: one, one per enum value, or one per property of an
/// object schema.
fn entries<'a: 'p, 'p>(
    doc: &mut Doc<'a>,
    param: &'p Value,
    name: &str,
    location: &str,
    source: &str,
    notes: &mut Vec<ImportNote>,
) -> Result<Vec<Entry>, DocumentError> {
    let schema_source = format!("{source}.schema");
    // A Swagger 2.0 parameter other than a body carries its schema's keys itself.
    let schema = match param.get("schema") {
        Some(s) => Some(doc.resolve(s, RefKind::Schema, &schema_source)?),
        None if doc.version == Version::Swagger2 => Some(Cow::Borrowed(param)),
        None => None,
    };
    let required = param.get("required").and_then(Value::as_bool) == Some(true);
    let allow_empty = param.get("allowEmptyValue").and_then(Value::as_bool) == Some(true);
    let mut out = Vec::new();
    let style = if doc.version == Version::Swagger2 {
        Style::Plain
    } else {
        style_of(param, name, location, schema.as_deref())
    };
    if let Style::Unexpressed(detail) = &style {
        doc.budget.charge_text(name)?;
        converted(doc, notes, source, detail.clone())?;
    }
    let deep = matches!(style, Style::Deep);
    // An object schema explodes into its properties, each taking a missing example from the
    // object's example and its required flag from the object's `required`.
    if let Some(properties) = schema
        .as_deref()
        .and_then(|s| s.get("properties"))
        .and_then(Value::as_object)
    {
        let schema = schema.as_deref().unwrap_or(&Value::Null);
        let required: Vec<&str> = schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        for (prop_name, raw) in properties {
            let prop_source = format!("{schema_source}.properties.{prop_name}");
            let prop = doc.resolve(raw, RefKind::Schema, &prop_source)?;
            let parent_example = schema.get("example").and_then(|e| e.get(prop_name));
            let fields = Fields {
                source: &prop_source,
                example: None,
                examples: None,
                schema: Some(&prop),
                required: required.contains(&prop_name.as_str()),
                allow_empty,
            };
            let values = values(doc, &fields, parent_example, &prop_source)?;
            // A deepObject property is addressed as `name[prop]`; one that is itself an
            // object or array stays a single entry, with a note that it is not expanded.
            let entry_name = if deep {
                format!("{name}[{prop_name}]")
            } else {
                prop_name.clone()
            };
            if deep && is_composite(&prop) {
                doc.budget.charge_text(&entry_name)?;
                let detail = format!(
                    "the query parameter `{name}` uses style deepObject, whose property `{prop_name}` is an object or array; nested values are not expanded, and it is imported as one `{entry_name}` entry"
                );
                converted(doc, notes, source, detail)?;
            }
            push(doc, &mut out, &entry_name, description(&prop), values)?;
        }
        return Ok(out);
    }
    let fields = Fields {
        source,
        example: param.get("example"),
        examples: param.get("examples"),
        schema: schema.as_deref(),
        required,
        allow_empty,
    };
    let values = values(doc, &fields, None, &schema_source)?;
    push(doc, &mut out, name, description(param), values)?;
    Ok(out)
}

/// How a parameter's `style` and `explode` map onto the entries it gives.
enum Style {
    /// The default shape, or one that needs no note.
    Plain,
    /// A deepObject query object whose properties become `name[prop]` params.
    Deep,
    /// A shape that is not expressed, with the note detail for it.
    Unexpressed(String),
}

/// Whether a schema is an array or an object, the shapes a style can serialize differently.
fn is_composite(schema: &Value) -> bool {
    schema.get("properties").is_some()
        || matches!(
            schema.get("type").and_then(Value::as_str),
            Some("array" | "object")
        )
}

/// Resolve the effective style and explode (OpenAPI 3.1.1 section 4.8.12.3) and decide how the
/// importer treats them. Defaults, and styles on scalars that cannot differ, give no note.
fn style_of(param: &Value, name: &str, location: &str, schema: Option<&Value>) -> Style {
    let default = if matches!(location, "path" | "header") {
        "simple"
    } else {
        "form"
    };
    let style = param
        .get("style")
        .and_then(Value::as_str)
        .unwrap_or(default);
    let explicit = param.get("explode").and_then(Value::as_bool);
    let explode = explicit.unwrap_or(style == "form");
    let composite = schema.is_some_and(is_composite);
    let has_properties = schema
        .and_then(|s| s.get("properties"))
        .and_then(Value::as_object)
        .is_some_and(|p| !p.is_empty());
    if style == "deepObject" && location == "query" {
        // deepObject has no default explode, so an unset one counts as true.
        if explicit != Some(false) && has_properties {
            return Style::Deep;
        }
        let with = match explicit {
            Some(e) => format!(" with explode {e}"),
            None => String::new(),
        };
        let then = if explicit == Some(false) && has_properties {
            "one entry per property".to_owned()
        } else {
            format!("plain `{name}` entries")
        };
        let why = if explicit != Some(false) {
            ", which is not expressed without known properties"
        } else {
            ", which is not expressed"
        };
        return unexpressed(location, name, style, &with, why, &then);
    }
    let valid = match location {
        "query" => matches!(
            style,
            "form" | "spaceDelimited" | "pipeDelimited" | "deepObject"
        ),
        "path" => matches!(style, "simple" | "label" | "matrix"),
        _ => style == "simple",
    };
    let expressed = if !valid {
        false
    } else {
        match style {
            // The default shapes: exploded form, and simple for path and header.
            "simple" => true,
            "form" => explode || !composite,
            "pipeDelimited" | "spaceDelimited" => !composite,
            _ => false,
        }
    };
    if expressed {
        return Style::Plain;
    }
    let with = if matches!(style, "form" | "pipeDelimited" | "spaceDelimited") {
        format!(" with explode {explode}")
    } else {
        explicit.map_or(String::new(), |e| format!(" with explode {e}"))
    };
    let then = if has_properties && location == "query" {
        "one entry per property".to_owned()
    } else {
        format!("plain `{name}` entries")
    };
    unexpressed(
        location,
        name,
        style,
        &with,
        ", which is not expressed",
        &then,
    )
}

fn unexpressed(
    location: &str,
    name: &str,
    style: &str,
    with: &str,
    why: &str,
    then: &str,
) -> Style {
    Style::Unexpressed(format!(
        "the {location} parameter `{name}` uses style {style}{with}{why}; it is imported as {then}"
    ))
}

/// Copy `values` out as entries named `name`, charging each entry and its text first.
fn push(
    doc: &mut Doc<'_>,
    out: &mut Vec<Entry>,
    name: &str,
    description: Option<&str>,
    values: Vec<(String, bool)>,
) -> Result<(), DocumentError> {
    let description = description.filter(|d| !d.is_empty());
    for (value, enabled) in values {
        doc.budget.charge_text(name)?;
        doc.budget.charge_text(description.unwrap_or_default())?;
        out.push(Entry {
            name: name.to_owned(),
            value,
            enabled,
            description: description.map(str::to_owned),
        });
    }
    Ok(())
}

/// Bruno's `getParameterEntries`: each value with whether it is enabled. `schema_example`
/// stands in for a missing `schema.example`, as an exploded property inherits it.
fn values<'a: 'p, 'p>(
    doc: &mut Doc<'a>,
    fields: &Fields<'p>,
    schema_example: Option<&'p Value>,
    schema_source: &str,
) -> Result<Vec<(String, bool)>, DocumentError> {
    let null = Value::Null;
    let schema = fields.schema.unwrap_or(&null);
    let get = |key: &str| schema.get(key);
    let is_array = get("type").and_then(Value::as_str) == Some("array");
    let items_source = format!("{schema_source}.items");
    let items = get("items")
        .map(|i| doc.resolve(i, RefKind::Schema, &items_source))
        .transpose()?;
    let items = items.as_deref();
    if doc.version == Version::Swagger2 {
        return swagger2_values(doc, schema, items.filter(|_| is_array), fields.required);
    }
    // An enum, or an array of enum items, gives one entry per value; only the default is
    // enabled, else the first when the parameter is required. An array default of enum items
    // is one enabled entry instead.
    let item_enum = items
        .filter(|_| is_array)
        .and_then(|i| non_empty(i.get("enum")));
    let enumerated = match (non_empty(get("enum")), item_enum) {
        (Some(values), _) => Some((values, get("default"))),
        (None, Some(_)) if get("default").is_some_and(Value::is_array) => {
            doc.budget.charge(1)?;
            let default = text(doc, get("default").unwrap_or(&null))?;
            return Ok(vec![(default, true)]);
        }
        (None, Some(values)) => Some((values, items.and_then(|i| i.get("default")))),
        _ => None,
    };
    if let Some((values, default)) = enumerated {
        return enum_entries(doc, values, default, fields.required);
    }
    doc.budget.charge(1)?;
    let mut value = String::new();
    let mut enabled = fields.required;
    let first_example = fields.examples.and_then(|e| match e {
        Value::Object(map) => map
            .iter()
            .next()
            .map(|(k, v)| (format!(".examples.{k}"), v)),
        Value::Array(list) => list.first().map(|v| (".examples[0]".to_owned(), v)),
        _ => None,
    });
    let first_example = first_example
        .map(|(path, raw)| {
            let source = format!("{}{path}", fields.source);
            doc.resolve(raw, RefKind::Object, &source)
        })
        .transpose()?;
    let found = fields
        .example
        .or_else(|| first_example.as_deref().and_then(|e| e.get("value")));
    // Each later source is consulted only while the value is still empty text.
    if let Some(v) = found {
        value = text(doc, v)?;
        enabled = true;
    }
    for candidate in [get("default"), get("example").or(schema_example)] {
        if let Some(v) = candidate.filter(|_| value.is_empty()) {
            value = text(doc, v)?;
            enabled = true;
        }
    }
    if value.is_empty()
        && is_array
        && let Some(items) = items
    {
        let item = items
            .get("example")
            .or_else(|| non_empty(items.get("enum")).map(|e| &e[0]))
            .or_else(|| items.get("default"));
        value = match item {
            Some(v) => text(doc, v)?,
            None => "[]".to_owned(),
        };
        enabled = fields.required;
    }
    if let Some(v) = non_empty(get("examples")).filter(|_| value.is_empty()) {
        value = text(doc, &v[0])?;
        enabled = true;
    }
    if let Some(v) = get("minimum").filter(|_| value.is_empty()) {
        value = text(doc, v)?;
        enabled = fields.required;
    }
    let nullable = get("nullable").and_then(Value::as_bool) == Some(true);
    if value.is_empty() && !fields.required && (nullable || fields.allow_empty) {
        enabled = false;
    }
    Ok(vec![(value, enabled)])
}

/// Bruno's Swagger 2.0 `getParameterEntries`: the parameter's `enum`, then an array of `enum`
/// items as one entry joined by its `collectionFormat` (one per value for `multi`), then
/// `example`, `default` or nothing. Item values and bounds are ignored; an array default is
/// joined by its `collectionFormat` even without enum items.
fn swagger2_values(
    doc: &mut Doc<'_>,
    param: &Value,
    items: Option<&Value>,
    required: bool,
) -> Result<Vec<(String, bool)>, DocumentError> {
    let get = |key: &str| param.get(key);
    let format = get("collectionFormat");
    let default_list = get("default").and_then(Value::as_array);
    if let Some(values) = non_empty(get("enum")) {
        return enum_entries(doc, values, get("default"), required);
    }
    if let Some(item) = items
        && let Some(values) = non_empty(item.get("enum"))
    {
        return match (default_list, format.and_then(Value::as_str)) {
            (Some(list), _) => collection_default(doc, list, format, true),
            (None, Some("multi")) => enum_entries(doc, values, item.get("default"), required),
            (None, _) => collection_default(doc, values, format, required),
        };
    }
    if get("example").is_none()
        && let Some(list) = default_list
    {
        return collection_default(doc, list, format, true);
    }
    doc.budget.charge(1)?;
    let value = match get("example").or_else(|| get("default")) {
        Some(v) => text(doc, v)?,
        None => String::new(),
    };
    let enabled = required || !value.is_empty();
    Ok(vec![(value, enabled)])
}

/// One entry per enum value: only the default is enabled, else the first when required.
fn enum_entries(
    doc: &mut Doc<'_>,
    values: &[Value],
    default: Option<&Value>,
    required: bool,
) -> Result<Vec<(String, bool)>, DocumentError> {
    let default = default.map(|d| text(doc, d)).transpose()?;
    let mut out = Vec::with_capacity(values.len());
    for (i, value) in values.iter().enumerate() {
        doc.budget.charge(1)?;
        let text = text(doc, value)?;
        let enabled = match &default {
            Some(d) => *d == text,
            None => i == 0 && required,
        };
        out.push((text, enabled));
    }
    Ok(out)
}

/// A Swagger 2.0 array joined by its `collectionFormat` (`csv` when absent or unknown) into
/// one entry enabled as given, or one enabled entry per value for `multi`.
fn collection_default(
    doc: &mut Doc<'_>,
    list: &[Value],
    format: Option<&Value>,
    enabled: bool,
) -> Result<Vec<(String, bool)>, DocumentError> {
    let separator = match format.and_then(Value::as_str) {
        Some("multi") => {
            let mut out = Vec::with_capacity(list.len());
            for value in list {
                doc.budget.charge(1)?;
                out.push((text(doc, value)?, true));
            }
            return Ok(out);
        }
        Some("ssv") => " ",
        Some("tsv") => "\t",
        Some("pipes") => "|",
        _ => ",",
    };
    doc.budget.charge(1)?;
    let mut parts = Vec::with_capacity(list.len());
    for value in list {
        doc.budget.charge_bytes(1)?;
        parts.push(text(doc, value)?);
    }
    Ok(vec![(parts.join(separator), enabled)])
}

/// A node's `description` text.
fn description(value: &Value) -> Option<&str> {
    value.get("description").and_then(Value::as_str)
}

/// `value` when it is a non-empty array.
fn non_empty(value: Option<&Value>) -> Option<&Vec<Value>> {
    value.and_then(Value::as_array).filter(|a| !a.is_empty())
}

/// JavaScript `String(value)` (arrays and objects as JSON), charged before it is copied.
pub(crate) fn text(doc: &mut Doc<'_>, value: &Value) -> Result<String, DocumentError> {
    match value {
        Value::String(s) => doc.budget.charge_text(s)?,
        other => doc.budget.charge_bytes(approx_size(other))?,
    }
    Ok(js::to_text(value))
}

/// Write the enabled query params into the URL, since the executor sends the URL's query
/// and not the params list. Delimiters are escaped in the URL only.
fn append_query(doc: &mut Doc<'_>, request: &mut ImportedRequest) -> Result<(), DocumentError> {
    let query: Vec<&Param> = request
        .params
        .iter()
        .filter(|p| p.enabled && p.kind == ParamKind::Query)
        .collect();
    if query.is_empty() {
        return Ok(());
    }
    let size: usize = query
        .iter()
        .map(|p| 3 * (p.name.len() + p.value.len()) + 2)
        .sum();
    doc.budget.charge_bytes(size as u64)?;
    let mut url = std::mem::take(&mut request.url);
    let mut separator = if url.contains('?') { '&' } else { '?' };
    for param in query {
        url.push(separator);
        separator = '&';
        escape(&mut url, &param.name, false);
        url.push('=');
        escape(&mut url, &param.value, true);
    }
    request.url = url;
    Ok(())
}

/// Append `text` with `%`, `&`, `#`, `+` and space percent-encoded, and `=` too in a value.
fn escape(out: &mut String, text: &str, value: bool) {
    for c in text.chars() {
        match c {
            '%' | '&' | '#' | '+' | ' ' => out.push_str(&format!("%{:02X}", c as u8)),
            '=' if value => out.push_str("%3D"),
            c => out.push(c),
        }
    }
}
