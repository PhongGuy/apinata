//! Hover text and definition links for names in the GraphQL query editor, answered from the
//! loaded schema through core's `symbol_at`.

use std::cell::RefCell;
use std::rc::Rc;

use apinata_core::{QUERY_VALIDATE_LIMIT, Schema, SchemaSymbol, TypeKind};
use gpui_kit::component::input::{DefinitionProvider, EditorState, HoverProvider};
use gpui_kit::component::{Rope, RopeExt as _};
use gpui_kit::{App, Context, Result, SharedString, Task, Window};
use lsp_types::{Hover, HoverContents, LocationLink, MarkupContent, MarkupKind, Range, Uri};

use crate::graphql_docs::DocsView;
use crate::graphql_schema_store::{self, SchemaState};

const SCHEME: &str = "apinata-docs:";

/// Markdown hover for the name at byte `offset`, its range in LSP positions; None off a known name.
pub fn hover(schema: &Schema, text: &Rope, offset: usize) -> Option<Hover> {
    if text.len() > QUERY_VALIDATE_LIMIT {
        return None;
    }
    let source = text.to_string();
    let found = schema.symbol_at(&source, offset)?;
    let parts = match &found.symbol {
        SchemaSymbol::Type { name } => {
            let ty = schema.type_named(name)?;
            let keyword = match ty.kind() {
                TypeKind::Object => "type",
                TypeKind::Interface => "interface",
                TypeKind::Union => "union",
                TypeKind::Enum => "enum",
                TypeKind::InputObject => "input",
                TypeKind::Scalar => "scalar",
                _ => "type",
            };
            (format!("{keyword} {name}"), ty.description(), None)
        }
        SchemaSymbol::Field { owner, field } => {
            let f = schema
                .type_named(owner)?
                .fields()
                .iter()
                .find(|f| f.name() == field)?;
            let args = if f.args().is_empty() {
                String::new()
            } else {
                let list: Vec<String> = f.args().iter().map(input_signature).collect();
                format!("({})", list.join(", "))
            };
            (
                format!("{owner}.{field}{args}: {}", f.ty().display()),
                f.description(),
                f.deprecation(),
            )
        }
        SchemaSymbol::Argument { owner, field, arg } => {
            let f = schema
                .type_named(owner)?
                .fields()
                .iter()
                .find(|f| f.name() == field)?;
            let a = f.args().iter().find(|a| a.name() == arg)?;
            (input_signature(a), a.description(), None)
        }
        SchemaSymbol::Directive { name } => {
            let d = schema.directive_named(name)?;
            (directive_signature(d), d.description(), None)
        }
        SchemaSymbol::DirectiveArgument { directive, arg } => {
            let a = schema
                .directive_named(directive)?
                .args()
                .iter()
                .find(|a| a.name() == arg)?;
            (
                format!("@{directive}({})", input_signature(a)),
                a.description(),
                None,
            )
        }
        _ => return None,
    };
    let (signature, description, deprecation) = parts;
    let mut sections = vec![code_span(&signature)];
    sections.extend(description.filter(|d| !d.is_empty()).map(plain));
    if let Some(reason) = deprecation {
        sections.push(if reason.is_empty() {
            "Deprecated".to_owned()
        } else {
            format!("Deprecated: {}", plain(reason))
        });
    }
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: sections.join("\n\n"),
        }),
        range: Some(lsp_range(text, &found.range)),
    })
}

/// One link to the name's docs page: origin = the name's range, both target ranges zero.
pub fn definition(schema: &Schema, text: &Rope, offset: usize) -> Option<LocationLink> {
    if text.len() > QUERY_VALIDATE_LIMIT {
        return None;
    }
    let found = schema.symbol_at(&text.to_string(), offset)?;
    let target = match &found.symbol {
        SchemaSymbol::Type { name } => format!("{SCHEME}type/{name}"),
        SchemaSymbol::Field { owner, field } | SchemaSymbol::Argument { owner, field, .. } => {
            format!("{SCHEME}field/{owner}/{field}")
        }
        SchemaSymbol::Directive { name: directive }
        | SchemaSymbol::DirectiveArgument { directive, .. } => {
            format!("{SCHEME}directive/{directive}")
        }
        _ => return None,
    };
    Some(LocationLink {
        origin_selection_range: Some(lsp_range(text, &found.range)),
        target_uri: target.parse::<Uri>().ok()?,
        target_range: Range::default(),
        target_selection_range: Range::default(),
    })
}

/// The Docs view an `apinata-docs:type/<Name>`, `apinata-docs:field/<Owner>/<field>` or
/// `apinata-docs:directive/<name>` URI names.
pub fn docs_view(uri: &str) -> Option<DocsView> {
    let rest = uri.strip_prefix(SCHEME)?;
    let mut parts = rest.split('/');
    let view = match (parts.next()?, parts.next(), parts.next(), parts.next()) {
        ("type", Some(name), None, None) => DocsView::Type(name.to_owned()),
        ("field", Some(owner), Some(field), None) => DocsView::Field {
            owner: owner.to_owned(),
            field: field.to_owned(),
        },
        ("directive", Some(name), None, None) => DocsView::Directive(name.to_owned()),
        _ => return None,
    };
    let named = match &view {
        DocsView::Type(n) => !n.is_empty(),
        DocsView::Field { owner, field } => !owner.is_empty() && !field.is_empty(),
        DocsView::Directive(n) => !n.is_empty(),
        _ => false,
    };
    named.then_some(view)
}

// Schema text shown as plain text: every ASCII punctuation character is backslash-escaped so no
// image, link or HTML renders. Line breaks become hard breaks, since the kit folds a soft one
// into a space, and leading whitespace goes, since four spaces or a tab would open a code block.
pub(crate) fn plain(text: &str) -> String {
    join_lines(text.lines().map(|line| escape(line.trim_start())).collect())
}

// Like `plain`, but a backtick-quoted span becomes inline code. A run of backticks closes at the
// next run of the same length on that line, as in CommonMark; a run that never closes is text.
pub(crate) fn plain_with_code(text: &str) -> String {
    join_lines(
        text.lines()
            .map(|line| line_with_code(line.trim_start()))
            .collect(),
    )
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_ascii_punctuation() {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn line_with_code(line: &str) -> String {
    let fence_at = |from: usize| -> Option<(usize, usize)> {
        let start = from + line[from..].find('`')?;
        let len = line[start..].len() - line[start..].trim_start_matches('`').len();
        Some((start, len))
    };
    let mut out = String::with_capacity(line.len());
    let mut done = 0;
    let mut from = 0;
    while let Some((open, len)) = fence_at(from) {
        let body = open + len;
        let mut search = body;
        let close = loop {
            match fence_at(search) {
                Some((at, l)) if l == len => break Some(at),
                Some((at, l)) => search = at + l,
                None => break None,
            }
        };
        match close {
            Some(close) => {
                out.push_str(&escape(&line[done..open]));
                out.push_str(&code_span(&line[body..close]));
                done = close + len;
                from = done;
            }
            None => from = body,
        }
    }
    out.push_str(&escape(&line[done..]));
    out
}

// Escaped lines joined so each break shows: the kit folds a soft break into a space, so a line
// followed by a non-empty one ends in a hard break.
fn join_lines(lines: Vec<String>) -> String {
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        out.push_str(line);
        match lines.get(i + 1) {
            Some(next) if !line.is_empty() && !next.is_empty() => out.push_str("\\\n"),
            Some(_) => out.push('\n'),
            None => {}
        }
    }
    out
}

// An inline code span that survives backticks inside: the fence is one longer than the longest
// run, with padding when the content touches a backtick, and newlines become spaces.
pub(crate) fn code_span(content: &str) -> String {
    let content = content.replace(['\r', '\n'], " ");
    let mut longest = 0;
    let mut run = 0;
    for c in content.chars() {
        run = if c == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let fence = "`".repeat(longest + 1);
    let pad = if content.starts_with('`') || content.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{fence}{pad}{content}{pad}{fence}")
}

// `name: Type = default`, without the default when there is none.
fn input_signature(v: &apinata_core::InputValue) -> String {
    match v.default_value() {
        Some(d) => format!("{}: {} = {d}", v.name(), v.ty().display()),
        None => format!("{}: {}", v.name(), v.ty().display()),
    }
}

/// `@name(arg: Type = default, ...) repeatable on A | B`; parentheses only with arguments,
/// `repeatable` only when set.
pub(crate) fn directive_signature(d: &apinata_core::Directive) -> String {
    let mut out = format!("@{}", d.name());
    if !d.args().is_empty() {
        let args: Vec<String> = d.args().iter().map(input_signature).collect();
        out.push_str(&format!("({})", args.join(", ")));
    }
    if d.is_repeatable() {
        out.push_str(" repeatable");
    }
    out.push_str(&format!(" on {}", d.locations().join(" | ")));
    out
}

pub(crate) fn lsp_range(text: &Rope, range: &std::ops::Range<usize>) -> Range {
    Range::new(
        text.offset_to_position(range.start),
        text.offset_to_position(range.end),
    )
}

/// The live check's last diagnostics as byte ranges and messages, with the text they belong to.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct Squiggles {
    text: Rope,
    found: Vec<(std::ops::Range<usize>, SharedString)>,
}

impl Squiggles {
    /// The diagnostics of `set`, computed for `text`.
    pub fn of(text: &Rope, set: &gpui_kit::base::input::DiagnosticSet) -> Self {
        Squiggles {
            text: text.clone(),
            found: set
                .iter()
                .map(|e| (e.range.clone(), e.diagnostic.message.clone()))
                .collect(),
        }
    }
}

/// `hover` with every diagnostic over its name appended below a rule, when `squiggles` were
/// computed for this very text; a stale or empty set leaves the hover as it is.
pub fn fold_squiggles(mut answer: Hover, text: &Rope, squiggles: &Squiggles) -> Hover {
    let (Some(range), HoverContents::Markup(markup)) = (answer.range, &mut answer.contents) else {
        return answer;
    };
    if squiggles.text != *text {
        return answer;
    }
    let name = text.position_to_offset(&range.start)..text.position_to_offset(&range.end);
    let notes: Vec<String> = squiggles
        .found
        .iter()
        .filter(|(at, _)| at.start < name.end && name.start < at.end)
        .map(|(_, message)| plain_with_code(message))
        .collect();
    if !notes.is_empty() {
        markup.value = format!("{}\n\n---\n\n{}", markup.value, notes.join("\n\n"));
    }
    answer
}

/// Closes the kit's diagnostic popover while the open hover already carries that diagnostic,
/// so one popover shows. Run on every editor change; the popover returns once the hover goes.
pub fn hide_folded_diagnostic(editor: &mut EditorState, cx: &mut Context<EditorState>) {
    let (Some(open), Some(diagnostic)) = (editor.hover_popover(), editor.diagnostic_popover())
    else {
        return;
    };
    let HoverContents::Markup(markup) = &open.hover.contents else {
        return;
    };
    let name = &open.symbol_range;
    let overlaps = diagnostic.range.start < name.end && name.start < diagnostic.range.end;
    if overlaps
        && markup
            .value
            .contains(&plain_with_code(&diagnostic.diagnostic.message))
    {
        editor.clear_diagnostic_popover(cx);
    }
}

/// Hover and definitions from the store's loaded schema for the URL the cell holds.
pub struct SchemaSymbols {
    url: Rc<RefCell<String>>,
    squiggles: Rc<RefCell<Squiggles>>,
}

impl SchemaSymbols {
    /// A provider for whatever URL `url` holds when asked.
    pub fn new(url: Rc<RefCell<String>>) -> Self {
        SchemaSymbols {
            url,
            squiggles: Rc::default(),
        }
    }

    /// The same provider, folding the live check's diagnostics from `squiggles` into hovers.
    pub fn with_squiggles(mut self, squiggles: Rc<RefCell<Squiggles>>) -> Self {
        self.squiggles = squiggles;
        self
    }

    /// `hover` for the URL's loaded schema with the diagnostics over the name folded in; None
    /// while the schema is absent, loading or failed.
    pub fn hover_answer(&self, text: &Rope, offset: usize, cx: &App) -> Option<Hover> {
        match graphql_schema_store::state(cx, &self.url.borrow()) {
            Some(SchemaState::Loaded(schema)) => hover(&schema, text, offset)
                .map(|answer| fold_squiggles(answer, text, &self.squiggles.borrow())),
            _ => None,
        }
    }

    /// `definition` for the URL's loaded schema; empty while it is absent, loading or failed.
    pub fn definition_answer(&self, text: &Rope, offset: usize, cx: &App) -> Vec<LocationLink> {
        match graphql_schema_store::state(cx, &self.url.borrow()) {
            Some(SchemaState::Loaded(schema)) => {
                definition(&schema, text, offset).into_iter().collect()
            }
            _ => Vec::new(),
        }
    }
}

impl HoverProvider for SchemaSymbols {
    fn hover(
        &self,
        text: &Rope,
        offset: usize,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Option<Hover>>> {
        Task::ready(Ok(self.hover_answer(text, offset, cx)))
    }
}

impl DefinitionProvider for SchemaSymbols {
    fn definitions(
        &self,
        text: &Rope,
        offset: usize,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Vec<LocationLink>>> {
        Task::ready(Ok(self.definition_answer(text, offset, cx)))
    }
}

#[cfg(test)]
mod tests {
    use super::plain_with_code;

    #[test]
    fn a_backtick_quoted_name_becomes_a_code_span() {
        assert_eq!(
            plain_with_code("`User.legacyName` is deprecated: Use name"),
            "`User.legacyName` is deprecated\\: Use name"
        );
    }

    #[test]
    fn an_unmatched_backtick_stays_escaped() {
        assert_eq!(
            plain_with_code("`a` and `b is open"),
            "`a` and \\`b is open"
        );
    }

    #[test]
    fn markdown_outside_backticks_stays_escaped() {
        assert_eq!(
            plain_with_code("see [link](x) or <b>`<i>`</b>"),
            "see \\[link\\]\\(x\\) or \\<b\\>`<i>`\\<\\/b\\>"
        );
    }

    #[test]
    fn a_double_fence_keeps_a_backtick_inside_the_span() {
        assert_eq!(plain_with_code("x ``a`b`` y"), "x ``a`b`` y");
        assert_eq!(plain_with_code("``"), "\\`\\`");
    }
}
