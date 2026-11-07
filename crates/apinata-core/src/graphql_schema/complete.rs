//! Completion at a cursor in a GraphQL document: a small lexer feeds a frame stack that
//! tracks which selection, argument list or variable definitions the cursor sits in.

use super::model::{Directive, InputValue, Schema, SchemaType, TypeKind};
use crate::graphql_query::QUERY_VALIDATE_LIMIT;

/// One completion offered at a cursor.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Completion {
    label: String,
    kind: CompletionKind,
    detail: String,
    documentation: Option<String>,
}

/// What a completion inserts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CompletionKind {
    /// A field of the enclosing selection's type.
    Field,
    /// An argument of the field whose argument list holds the cursor, labelled `name:`.
    Argument,
    /// A type name, after `on` or in a variable definition.
    Type,
    /// A directive name after `@`, labelled with the bare name.
    Directive,
}

impl Completion {
    /// The text to insert, such as `user` or `id:`.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// What the completion inserts.
    pub fn kind(&self) -> CompletionKind {
        self.kind
    }

    /// The field or argument type as written in GraphQL, or a type's kind word.
    pub fn detail(&self) -> &str {
        &self.detail
    }

    /// The schema's description of the completed item.
    pub fn documentation(&self) -> Option<&str> {
        self.documentation.as_deref()
    }
}

impl Schema {
    /// The completions for the name being typed at byte `offset` of `query`, filtered by the
    /// typed prefix (ASCII case-insensitive). The offset is clamped to the text and down to
    /// a character boundary. Inside a string or comment, where nothing fits, or over
    /// [`QUERY_VALIDATE_LIMIT`], it is empty.
    pub fn completions(&self, query: &str, offset: usize) -> Vec<Completion> {
        if query.len() > QUERY_VALIDATE_LIMIT {
            return Vec::new();
        }
        let mut end = offset.min(query.len());
        while !query.is_char_boundary(end) {
            end -= 1;
        }
        let start = query[..end]
            .bytes()
            .rposition(|b| !(b.is_ascii_alphanumeric() || b == b'_'))
            .map_or(0, |i| i + 1);
        let prefix = &query[start..end];
        // The scanner would read an open `{{` placeholder as two selection sets.
        let line = &query[query[..start].rfind('\n').map_or(0, |i| i + 1)..start];
        if line.rfind("{{").is_some_and(|i| !line[i..].contains("}}")) {
            return Vec::new();
        }
        let Some(context) = Scanner::new(self).run(&query[..start]) else {
            return Vec::new();
        };
        self.offer(context)
            .into_iter()
            .filter(|c| starts_with_ignore_case(c.label.trim_end_matches(':'), prefix))
            .collect()
    }

    fn offer(&self, context: Context) -> Vec<Completion> {
        match context {
            Context::Nothing => Vec::new(),
            Context::Fields(parent) => self.field_completions(&parent),
            Context::Args { parent, field } => self.arg_completions(&parent, &field),
            Context::DirectiveArgs { directive } => self
                .directive_named(&directive)
                .map_or_else(Vec::new, |d| arg_items(d.args())),
            Context::Directives(class) => self
                .directives()
                .iter()
                .filter(|d| d.locations().iter().any(|l| class.contains(&l.as_str())))
                .map(|d| Completion {
                    label: d.name().to_owned(),
                    kind: CompletionKind::Directive,
                    detail: directive_detail(d),
                    documentation: d.description().map(str::to_owned),
                })
                .collect(),
            Context::Types(kinds) => self
                .types()
                .iter()
                .filter(|t| kinds.contains(&t.kind()) && !t.name().starts_with("__"))
                .map(|t| Completion {
                    label: t.name().to_owned(),
                    kind: CompletionKind::Type,
                    detail: kind_word(t.kind()).to_owned(),
                    documentation: t.description().map(str::to_owned),
                })
                .collect(),
        }
    }

    /// Live fields, then `__typename`, then deprecated fields.
    fn field_completions(&self, parent: &str) -> Vec<Completion> {
        let Some(ty) = self.selectable(parent) else {
            return Vec::new();
        };
        let field = |f: &super::model::Field| Completion {
            label: f.name().to_owned(),
            kind: CompletionKind::Field,
            detail: f.ty().display().to_owned(),
            documentation: f.description().map(str::to_owned),
        };
        let live = ty.fields().iter().filter(|f| f.deprecation().is_none());
        let deprecated = ty.fields().iter().filter(|f| f.deprecation().is_some());
        let typename = Completion {
            label: "__typename".to_owned(),
            kind: CompletionKind::Field,
            detail: "String!".to_owned(),
            documentation: None,
        };
        live.map(field)
            .chain(std::iter::once(typename))
            .chain(deprecated.map(field))
            .collect()
    }

    fn arg_completions(&self, parent: &str, field: &str) -> Vec<Completion> {
        let Some(f) = self
            .selectable(parent)
            .and_then(|t| t.fields().iter().find(|f| f.name() == field))
        else {
            return Vec::new();
        };
        arg_items(f.args())
    }

    /// The type named `name` when a selection set can be written on it.
    pub(super) fn selectable(&self, name: &str) -> Option<&SchemaType> {
        self.type_named(name).filter(|t| {
            matches!(
                t.kind(),
                TypeKind::Object | TypeKind::Interface | TypeKind::Union
            )
        })
    }

    /// The named type of field `field` on type `parent`.
    fn field_type(&self, parent: Option<&str>, field: &str) -> Option<String> {
        let ty = self.selectable(parent?)?;
        let f = ty.fields().iter().find(|f| f.name() == field)?;
        Some(f.ty().named().to_owned())
    }
}

fn arg_items(args: &[InputValue]) -> Vec<Completion> {
    args.iter()
        .map(|a| Completion {
            label: format!("{}:", a.name()),
            kind: CompletionKind::Argument,
            detail: a.ty().display().to_owned(),
            documentation: a.description().map(str::to_owned),
        })
        .collect()
}

/// `@name(arg: Type, ...)`, or just `@name` without arguments; defaults are left out.
fn directive_detail(d: &Directive) -> String {
    if d.args().is_empty() {
        return format!("@{}", d.name());
    }
    let args: Vec<String> = d
        .args()
        .iter()
        .map(|a| format!("{}: {}", a.name(), a.ty().display()))
        .collect();
    format!("@{}({})", d.name(), args.join(", "))
}

fn kind_word(kind: TypeKind) -> &'static str {
    match kind {
        TypeKind::Object => "object",
        TypeKind::Interface => "interface",
        TypeKind::Union => "union",
        TypeKind::Enum => "enum",
        TypeKind::InputObject => "input",
        TypeKind::Scalar => "scalar",
    }
}

fn starts_with_ignore_case(name: &str, prefix: &str) -> bool {
    name.len() >= prefix.len()
        && name.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

/// What the scanner found at the cursor.
pub(super) enum Context {
    Nothing,
    Fields(String),
    Args {
        parent: String,
        field: String,
    },
    Types(&'static [TypeKind]),
    /// A directive name, offered when it has a location in this class.
    Directives(&'static [&'static str]),
    DirectiveArgs {
        directive: String,
    },
}

const SELECTION_LOCATIONS: &[&str] = &["FIELD", "FRAGMENT_SPREAD", "INLINE_FRAGMENT"];
const HEADER_LOCATIONS: &[&str] = &["QUERY", "MUTATION", "SUBSCRIPTION", "FRAGMENT_DEFINITION"];
const VARIABLE_LOCATIONS: &[&str] = &["VARIABLE_DEFINITION"];

const ON_TYPES: &[TypeKind] = &[TypeKind::Object, TypeKind::Interface, TypeKind::Union];
const INPUT_TYPES: &[TypeKind] = &[TypeKind::Scalar, TypeKind::Enum, TypeKind::InputObject];

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Name(String),
    /// A string or number literal.
    Value,
    /// `...`
    Spread,
    Punct(u8),
}

/// What a selection token does to the frame stack.
enum Step {
    Stay,
    Close,
    Open(Frame),
}

/// What a selection's next `{` would open on.
enum Pending {
    None,
    Field(String),
    Fragment(Option<String>),
}

enum Frame {
    Selection {
        parent: Option<String>,
        pending: Pending,
    },
    Args {
        parent: Option<String>,
        field: Option<String>,
    },
    DirectiveArgs {
        directive: String,
    },
    VarDefs {
        in_type: bool,
    },
    /// An object or list value inside arguments or a default value.
    Value,
}

pub(super) struct Scanner<'s> {
    schema: &'s Schema,
    stack: Vec<Frame>,
    /// The root a top-level `{` opens: the operation's root or a fragment's type condition.
    root: Option<String>,
    /// The last three tokens, newest last.
    history: [Option<Token>; 3],
}

impl<'s> Scanner<'s> {
    pub(super) fn new(schema: &'s Schema) -> Self {
        Scanner {
            schema,
            stack: Vec::new(),
            root: schema.query_type().map(str::to_owned),
            history: [None, None, None],
        }
    }

    /// Lexes `text` (everything before the typed prefix) and judges the cursor context;
    /// `None` when the text ends inside a string or comment.
    pub(super) fn run(mut self, text: &str) -> Option<Context> {
        let b = text.as_bytes();
        let mut i = 0;
        while i < b.len() {
            let c = b[i];
            let token = match c {
                b' ' | b'\t' | b'\r' | b'\n' | b',' => {
                    i += 1;
                    continue;
                }
                b'#' => {
                    i += b[i..].iter().position(|&x| x == b'\n')?;
                    continue;
                }
                b'"' if b[i..].starts_with(b"\"\"\"") => {
                    i += 3;
                    loop {
                        if i >= b.len() {
                            return None;
                        } else if b[i..].starts_with(b"\\\"\"\"") {
                            i += 4;
                        } else if b[i..].starts_with(b"\"\"\"") {
                            i += 3;
                            break;
                        } else {
                            i += 1;
                        }
                    }
                    Token::Value
                }
                b'"' => {
                    i += 1;
                    loop {
                        match b.get(i) {
                            None => return None,
                            Some(b'\\') => i += 2,
                            // A line break ends a broken string so later lines still scan.
                            Some(b'"' | b'\n') => {
                                i += 1;
                                break;
                            }
                            Some(_) => i += 1,
                        }
                    }
                    Token::Value
                }
                b'.' if b[i..].starts_with(b"...") => {
                    i += 3;
                    Token::Spread
                }
                b'{' | b'}' | b'(' | b')' | b'[' | b']' | b':' | b'$' | b'@' | b'=' | b'!' => {
                    i += 1;
                    Token::Punct(c)
                }
                b'-' | b'0'..=b'9' => {
                    i += 1;
                    while i < b.len()
                        && (b[i].is_ascii_alphanumeric()
                            || b[i] == b'.'
                            || (matches!(b[i], b'+' | b'-') && matches!(b[i - 1], b'e' | b'E')))
                    {
                        i += 1;
                    }
                    Token::Value
                }
                c if c.is_ascii_alphabetic() || c == b'_' => {
                    let start = i;
                    while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                        i += 1;
                    }
                    Token::Name(text[start..i].to_owned())
                }
                _ => {
                    i += 1;
                    continue;
                }
            };
            self.step(&token);
            self.history.rotate_left(1);
            self.history[2] = Some(token);
        }
        Some(self.context())
    }

    fn prev(&self, back: usize) -> Option<&Token> {
        self.history[3 - back].as_ref()
    }

    fn prev_is(&self, back: usize, punct: u8) -> bool {
        self.prev(back) == Some(&Token::Punct(punct))
    }

    fn prev_name(&self, back: usize) -> Option<&str> {
        match self.prev(back) {
            Some(Token::Name(n)) => Some(n),
            _ => None,
        }
    }

    /// The directive whose arguments a `(` here would open: the name right after `@`.
    fn directive_name(&self) -> Option<String> {
        self.prev_is(2, b'@')
            .then(|| self.prev_name(1).map(str::to_owned))
            .flatten()
    }

    /// Applies one token, with `history` still holding the tokens before it.
    fn step(&mut self, token: &Token) {
        let directive = self.directive_name();
        match self.stack.last_mut() {
            None => self.step_document(token, directive),
            Some(Frame::Selection { .. }) => {
                let Some(Frame::Selection {
                    parent,
                    mut pending,
                }) = self.stack.pop()
                else {
                    unreachable!("the top frame is a selection");
                };
                let opened = self.step_selection(token, parent.as_deref(), &mut pending, directive);
                match opened {
                    Step::Close if self.stack.is_empty() => {
                        self.root = self.schema.query_type().map(str::to_owned);
                    }
                    Step::Close => {}
                    Step::Stay => self.stack.push(Frame::Selection { parent, pending }),
                    Step::Open(child) => {
                        self.stack.push(Frame::Selection { parent, pending });
                        self.stack.push(child);
                    }
                }
            }
            Some(Frame::Args { .. }) | Some(Frame::DirectiveArgs { .. }) => match token {
                Token::Punct(b'{' | b'[') => self.stack.push(Frame::Value),
                Token::Punct(b')') => {
                    self.stack.pop();
                }
                _ => {}
            },
            Some(Frame::VarDefs { in_type }) => match token {
                Token::Punct(b'$' | b'=') => *in_type = false,
                Token::Punct(b':') => *in_type = true,
                Token::Punct(b')') => {
                    self.stack.pop();
                }
                Token::Punct(b'{') => self.stack.push(Frame::Value),
                Token::Punct(b'(') => {
                    if let Some(directive) = directive {
                        self.stack.push(Frame::DirectiveArgs { directive });
                    }
                }
                _ => {}
            },
            Some(Frame::Value) => match token {
                Token::Punct(b'{' | b'[') => self.stack.push(Frame::Value),
                Token::Punct(b'}' | b']') => {
                    self.stack.pop();
                }
                _ => {}
            },
        }
    }

    fn step_document(&mut self, token: &Token, directive: Option<String>) {
        match token {
            Token::Name(n) if !self.prev_is(1, b'@') => match n.as_str() {
                "query" => self.root = self.schema.query_type().map(str::to_owned),
                "mutation" => self.root = self.schema.mutation_type().map(str::to_owned),
                "subscription" => self.root = self.schema.subscription_type().map(str::to_owned),
                _ if self.prev_name(1) == Some("on") => self.root = Some(n.clone()),
                _ => {}
            },
            Token::Punct(b'(') => self.stack.push(match directive {
                Some(directive) => Frame::DirectiveArgs { directive },
                None => Frame::VarDefs { in_type: false },
            }),
            Token::Punct(b'{') => {
                let parent = self.root.take();
                self.stack.push(Frame::Selection {
                    parent,
                    pending: Pending::None,
                });
            }
            _ => {}
        }
    }

    /// Applies one token inside a selection set, updating what its next `{` opens on.
    fn step_selection(
        &self,
        token: &Token,
        parent: Option<&str>,
        pending: &mut Pending,
        directive: Option<String>,
    ) -> Step {
        match token {
            // A directive name never becomes the pending field.
            Token::Name(_) if self.prev_is(1, b'@') => {}
            // `...Name` is a spread; `... on` waits for the type condition.
            Token::Name(n) if self.prev(1) == Some(&Token::Spread) => {
                if n != "on" {
                    *pending = Pending::None;
                }
            }
            Token::Name(n)
                if self.prev_name(1) == Some("on") && self.prev(2) == Some(&Token::Spread) =>
            {
                *pending = Pending::Fragment(Some(n.clone()));
            }
            Token::Name(n) => *pending = Pending::Field(n.clone()),
            Token::Spread => *pending = Pending::Fragment(parent.map(str::to_owned)),
            // After an alias, the next name is the field.
            Token::Punct(b':') => *pending = Pending::None,
            Token::Punct(b'(') => {
                if let Some(directive) = directive {
                    return Step::Open(Frame::DirectiveArgs { directive });
                }
                let field = match pending {
                    Pending::Field(f) => Some(f.clone()),
                    _ => None,
                };
                return Step::Open(Frame::Args {
                    parent: parent.map(str::to_owned),
                    field,
                });
            }
            Token::Punct(b'{') => {
                let child = match std::mem::replace(pending, Pending::None) {
                    Pending::Field(f) => self.schema.field_type(parent, &f),
                    Pending::Fragment(t) => t,
                    Pending::None => None,
                };
                return Step::Open(Frame::Selection {
                    parent: child,
                    pending: Pending::None,
                });
            }
            Token::Punct(b'}') => return Step::Close,
            _ => {}
        }
        Step::Stay
    }

    fn context(&self) -> Context {
        let on_condition = self.prev_name(1) == Some("on")
            && match self.stack.last() {
                None => self.prev_name(2).is_some() && self.prev_name(3) == Some("fragment"),
                Some(Frame::Selection { .. }) => self.prev(2) == Some(&Token::Spread),
                _ => false,
            };
        if on_condition {
            return Context::Types(ON_TYPES);
        }
        if self.prev_is(1, b'@') {
            return match self.stack.last() {
                None => Context::Directives(HEADER_LOCATIONS),
                Some(Frame::Selection { .. }) => Context::Directives(SELECTION_LOCATIONS),
                Some(Frame::VarDefs { .. }) => Context::Directives(VARIABLE_LOCATIONS),
                _ => Context::Nothing,
            };
        }
        match self.stack.last() {
            Some(Frame::Selection {
                parent: Some(parent),
                ..
            }) if !(self.prev_is(1, b'@')
                || self.prev_is(1, b'$')
                || self.prev_is(1, b'=')
                || self.prev(1) == Some(&Token::Spread)) =>
            {
                Context::Fields(parent.clone())
            }
            Some(Frame::Args {
                parent: Some(parent),
                field: Some(field),
            }) if !(self.prev_is(1, b':') || self.prev_is(1, b'$')) => Context::Args {
                parent: parent.clone(),
                field: field.clone(),
            },
            Some(Frame::DirectiveArgs { directive })
                if !(self.prev_is(1, b':') || self.prev_is(1, b'$')) =>
            {
                Context::DirectiveArgs {
                    directive: directive.clone(),
                }
            }
            Some(Frame::VarDefs { in_type: true })
                if self.prev_is(1, b':') || self.prev_is(1, b'[') =>
            {
                Context::Types(INPUT_TYPES)
            }
            _ => Context::Nothing,
        }
    }
}
