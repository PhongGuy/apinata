//! A field serde may leave out of a result must be optional in its schema too: every field
//! with `skip_serializing_if` is an `Option` or has `default`, or its container has
//! `#[serde(default)]`, since schemars lists any other field as required. What the line scan
//! cannot vouch for (a macro body, a `cfg_attr` serde attribute, a renamed serde import) is
//! refused; fields another crate's proc macros generate stay unseen.

use std::fs;
use std::path::{Path, PathBuf};

/// Every field in `source` that breaks the rule, as `file:line: field`.
///
/// A line scan, not a parser: attribute lines (multi-line ones read to their closing `]`)
/// gather until the next item, which is either a field, checked against them, or a struct or
/// enum, whose `#[serde(default)]` then covers the fields inside its braces.
fn scan(file: &str, source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut attrs = String::new();
    let mut open = 0i32;
    let mut depth = 0i32;
    let mut defaulted: Vec<i32> = Vec::new();
    // The current attribute's first line and text, and a macro's first line, bracket level
    // (any of `{([`), whether its body has opened and whether it was already refused.
    let mut attr = (0, String::new());
    let mut in_macro: Option<(usize, i32, bool, bool)> = None;
    for (i, raw) in source.lines().enumerate() {
        let line = raw.trim();
        let code = without_strings(line);
        if code.contains("macro_rules!") && in_macro.is_none() && open == 0 {
            in_macro = Some((i + 1, 0, false, false));
            attrs.clear();
        }
        if let Some((start, level, opened, refused)) = &mut in_macro {
            if line.contains("skip_serializing_if") && !*refused {
                *refused = true;
                out.push(format!(
                    "{file}:{start}: a macro_rules! body uses skip_serializing_if, which the \
                     scan cannot check"
                ));
            }
            *level += ['{', '(', '[']
                .iter()
                .map(|&c| count(&code, c))
                .sum::<i32>()
                - ['}', ')', ']']
                    .iter()
                    .map(|&c| count(&code, c))
                    .sum::<i32>();
            *opened |= *level > 0;
            depth += count(&code, '{') - count(&code, '}');
            if *opened && *level <= 0 {
                in_macro = None;
            }
            continue;
        }
        if open > 0 || line.starts_with("#[") {
            if open == 0 {
                attr = (i + 1, String::new());
            }
            attr.1.push_str(&code);
            attrs.push_str(line);
            attrs.push('\n');
            open += count(&code, '[') - count(&code, ']');
            if open == 0 && attr.1.contains("cfg_attr(") && attr.1.contains("serde(") {
                out.push(format!(
                    "{file}:{}: a serde attribute under cfg_attr, which the scan cannot read",
                    attr.0
                ));
            }
            continue;
        }
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if renames_serde(&code) {
            out.push(format!(
                "{file}:{}: a renamed serde import, whose attributes the scan cannot see",
                i + 1
            ));
        }
        if let Some((name, ty)) = field(line) {
            let optional = ty.starts_with("Option<") || ty.starts_with("std::option::Option<");
            if attrs.contains("skip_serializing_if")
                && !optional
                && !serde_default(&attrs)
                && !defaulted.contains(&depth)
            {
                out.push(format!(
                    "{file}:{}: field `{name}` has skip_serializing_if but is neither an Option \
                     nor defaulted",
                    i + 1
                ));
            }
        } else if (code.contains("struct ") || code.contains("enum ")) && serde_default(&attrs) {
            defaulted.push(depth + 1);
        }
        attrs.clear();
        depth += count(&code, '{') - count(&code, '}');
        defaulted.retain(|d| *d <= depth);
    }
    out
}

/// Whether `code` imports serde under another name, as `use serde as s;`,
/// `use serde::{self as s};` or `extern crate serde as s;` do.
fn renames_serde(code: &str) -> bool {
    let item = code.trim_start_matches("pub ").trim_start();
    let renamed = |name: &str| {
        item.match_indices(name)
            .any(|(at, _)| at == 0 || matches!(item.as_bytes()[at - 1], b' ' | b':' | b'{' | b','))
    };
    let self_as = item.contains("serde::{") && renamed("self as ");
    (item.starts_with("use ") || item.starts_with("extern crate "))
        && (renamed("serde as ") || self_as)
}

/// `name` and type of a named field line such as `pub(crate) name: Type,`.
fn field(line: &str) -> Option<(&str, &str)> {
    let rest = ["pub(crate) ", "pub(super) ", "pub "]
        .iter()
        .find_map(|p| line.strip_prefix(p))
        .unwrap_or(line);
    let (name, ty) = rest.split_once(':')?;
    let ident = !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_');
    (ident && !ty.starts_with(':')).then(|| (name, ty.trim()))
}

/// Whether a `#[serde(...)]` among `attrs` names `default`, outside any string.
fn serde_default(attrs: &str) -> bool {
    attrs
        .split("#[")
        .filter(|a| a.trim_start().starts_with("serde"))
        .any(|a| {
            without_strings(a)
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .any(|token| token == "default")
        })
}

/// `line` with the contents of every string literal removed, so brackets and words in
/// strings are not counted.
fn without_strings(line: &str) -> String {
    let mut out = String::new();
    let (mut quoted, mut escaped) = (false, false);
    for c in line.chars() {
        match (quoted, escaped, c) {
            (true, true, _) => escaped = false,
            (true, false, '\\') => escaped = true,
            (true, false, '"') => quoted = false,
            (true, false, _) => {}
            (false, _, '"') => quoted = true,
            (false, _, c) => out.push(c),
        }
    }
    out
}

fn count(text: &str, c: char) -> i32 {
    text.chars().filter(|&x| x == c).count() as i32
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read src") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn every_skipped_field_in_src_is_optional_or_defaulted() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    files.sort();
    assert!(files.len() > 10, "found the sources: {files:?}");
    let mut broken = Vec::new();
    for path in &files {
        let label = path
            .strip_prefix(&src)
            .unwrap_or(path)
            .display()
            .to_string();
        broken.extend(scan(
            &format!("src/{label}"),
            &fs::read_to_string(path).expect("read"),
        ));
    }
    assert!(
        broken.is_empty(),
        "fields skipped when serialized but required by the schema:\n{}",
        broken.join("\n")
    );
}

#[test]
fn a_skipped_field_without_option_or_default_is_named_with_its_line() {
    let source = "struct A {\n    #[serde(skip_serializing_if = \"Vec::is_empty\")]\n    pub hits: Vec<String>,\n}\n";
    assert_eq!(
        scan("src/x.rs", source),
        vec![
            "src/x.rs:3: field `hits` has skip_serializing_if but is neither an Option nor defaulted"
        ]
    );
}

#[test]
fn option_and_defaulted_fields_pass() {
    let source = concat!(
        "struct A {\n",
        "    #[serde(skip_serializing_if = \"Option::is_none\")]\n",
        "    pub a: Option<u8>,\n",
        "    #[serde(default, skip_serializing_if = \"Vec::is_empty\")]\n",
        "    b: Vec<u8>,\n",
        "    #[serde(skip_serializing_if = \"is_default\", default = \"zero\")]\n",
        "    pub(crate) c: u8,\n",
        "}\n",
    );
    assert_eq!(scan("src/x.rs", source), Vec::<String>::new());
}

#[test]
fn a_default_only_inside_a_string_does_not_count() {
    let source =
        "struct A {\n    #[serde(skip_serializing_if = \"default_skip\")]\n    n: u8,\n}\n";
    assert_eq!(scan("src/x.rs", source).len(), 1);
}

#[test]
fn multi_line_attributes_and_doc_comments_are_read_whole() {
    let source = concat!(
        "struct A {\n",
        "    /// Documented.\n",
        "    #[serde(\n",
        "        rename = \"t\",\n",
        "        skip_serializing_if = \"Vec::is_empty\"\n",
        "    )]\n",
        "    /// More.\n",
        "    #[schemars(description = \"x\")]\n",
        "    pub broken: Vec<u8>,\n",
        "    #[serde(\n",
        "        default,\n",
        "        skip_serializing_if = \"Vec::is_empty\"\n",
        "    )]\n",
        "    pub fine: Vec<u8>,\n",
        "}\n",
    );
    assert_eq!(
        scan("src/x.rs", source),
        vec![
            "src/x.rs:9: field `broken` has skip_serializing_if but is neither an Option nor defaulted"
        ]
    );
}

#[test]
fn a_container_default_covers_its_fields_only() {
    let source = concat!(
        "#[derive(Serialize)]\n",
        "#[serde(default, rename_all = \"snake_case\")]\n",
        "struct A {\n",
        "    #[serde(skip_serializing_if = \"Vec::is_empty\")]\n",
        "    a: Vec<u8>,\n",
        "}\n",
        "struct B {\n",
        "    #[serde(skip_serializing_if = \"Vec::is_empty\")]\n",
        "    b: Vec<u8>,\n",
        "}\n",
    );
    assert_eq!(
        scan("src/x.rs", source),
        vec![
            "src/x.rs:9: field `b` has skip_serializing_if but is neither an Option nor defaulted"
        ]
    );
}

#[test]
fn a_macro_body_with_skip_serializing_if_is_refused() {
    let source = concat!(
        "macro_rules! doc {\n",
        "    ($name:ident) => {\n",
        "        struct $name {\n",
        "            #[serde(skip_serializing_if = \"Vec::is_empty\")]\n",
        "            hits: Vec<u8>,\n",
        "        }\n",
        "    };\n",
        "}\n",
    );
    let found = scan("src/x.rs", source);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].starts_with("src/x.rs:1: "), "{found:?}");
    assert!(found[0].contains("macro_rules!"), "{found:?}");
}

#[test]
fn a_serde_attribute_under_cfg_attr_is_refused() {
    let source = concat!(
        "struct A {\n",
        "    #[cfg_attr(feature = \"x\", serde(skip_serializing_if = \"Vec::is_empty\"))]\n",
        "    pub a: Option<Vec<u8>>,\n",
        "}\n",
    );
    let found = scan("src/x.rs", source);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].starts_with("src/x.rs:2: "), "{found:?}");
    assert!(found[0].contains("cfg_attr"), "{found:?}");
}

#[test]
fn a_renamed_serde_import_is_refused() {
    for (line, source) in [
        (1, "use serde as s;\n"),
        (2, "use std::fmt;\nextern crate serde as s;\n"),
        (1, "use ::serde as s;\n"),
    ] {
        let found = scan("src/x.rs", source);
        assert_eq!(found.len(), 1, "{source}: {found:?}");
        assert!(
            found[0].starts_with(&format!("src/x.rs:{line}: ")),
            "{found:?}"
        );
        assert!(found[0].contains("renamed"), "{found:?}");
    }
}

#[test]
fn ordinary_macros_cfg_attrs_and_imports_pass() {
    let source = concat!(
        "use serde::{Deserialize, Serialize};\n",
        "macro_rules! twice { ($e:expr) => { $e + $e }; }\n",
        "#[cfg_attr(test, derive(Debug))]\n",
        "struct A {\n",
        "    a: u8,\n",
        "}\n",
    );
    assert_eq!(scan("src/x.rs", source), Vec::<String>::new());
}

#[test]
fn a_self_rename_inside_a_serde_use_tree_is_refused() {
    for source in [
        "use serde::{self as s};\n",
        "use serde::{Deserialize, self as s};\n",
    ] {
        let found = scan("src/x.rs", source);
        assert_eq!(found.len(), 1, "{source}: {found:?}");
        assert!(found[0].contains("renamed"), "{found:?}");
    }
}

#[test]
fn a_macro_body_in_any_delimiter_or_opened_later_is_refused() {
    let body = concat!(
        "    ($name:ident) => {\n",
        "        struct $name {\n",
        "            #[serde(skip_serializing_if = \"Vec::is_empty\")]\n",
        "            hits: Vec<u8>,\n",
        "        }\n",
        "    };\n",
    );
    for (open, close) in [("\n{", "}"), (" (", ");"), (" [", "];"), ("\n(", ");")] {
        let source = format!("macro_rules! doc{open}\n{body}{close}\nstruct After {{ a: u8 }}\n");
        let found = scan("src/x.rs", &source);
        assert_eq!(found.len(), 1, "{source}: {found:?}");
        assert!(found[0].starts_with("src/x.rs:1: "), "{found:?}");
        assert!(found[0].contains("macro_rules!"), "{found:?}");
    }
}
