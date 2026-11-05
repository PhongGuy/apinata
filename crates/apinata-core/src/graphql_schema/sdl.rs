//! The SDL reader: a schema document parsed with `apollo-parser` and lowered into the same
//! [`Schema`] an introspection answer produces, so the explorer and completion see no difference.

use std::collections::HashMap;

use apollo_parser::cst::{self, CstNode};
use apollo_parser::{Parser, SyntaxNode};

use super::SchemaError;
use super::model::{EnumValue, Field, InputValue, Schema, SchemaType, TypeKind, TypeRef};

/// The scalars every schema has, appended when the document does not declare them.
const BUILT_IN_SCALARS: [&str; 5] = ["String", "Int", "Float", "Boolean", "ID"];

/// The reason a bare `@deprecated` carries, as graphql-js reports it.
const DEFAULT_DEPRECATION: &str = "No longer supported";

/// Parses an SDL document into a [`Schema`]; see [`Schema::from_sdl`].
pub(super) fn parse(text: &str) -> Result<Schema, SchemaError> {
    let tree = Parser::new(text).parse();
    if let Some(error) = tree.errors().next() {
        return Err(at(text, error.index(), error.message()));
    }
    let mut reader = Reader {
        text,
        types: Vec::new(),
        index: HashMap::new(),
        refs: Vec::new(),
        roots: Vec::new(),
    };
    // Definitions first, then extensions, so an extension may come before the type it extends.
    let definitions: Vec<cst::Definition> = tree.document().definitions().collect();
    for def in definitions.iter().filter(|d| !d.is_extension_definition()) {
        reader.define(def)?;
    }
    for def in definitions.iter().filter(|d| d.is_extension_definition()) {
        reader.extend(def)?;
    }
    reader.finish()
}

struct Reader<'a> {
    text: &'a str,
    types: Vec<SchemaType>,
    index: HashMap<String, usize>,
    /// Every named type reference with its byte offset, checked once all types are known.
    refs: Vec<(String, usize)>,
    /// Root operations from `schema` blocks: (operation, type name, offset).
    roots: Vec<(Root, String, usize)>,
}

#[derive(Clone, Copy, PartialEq)]
enum Root {
    Query,
    Mutation,
    Subscription,
}

impl Reader<'_> {
    fn error(&self, node: &SyntaxNode, detail: impl AsRef<str>) -> SchemaError {
        at(self.text, offset(node), detail.as_ref())
    }

    fn name(&self, name: Option<cst::Name>, owner: &SyntaxNode) -> Result<String, SchemaError> {
        name.map(|n| n.text().to_string())
            .ok_or_else(|| self.error(owner, "a definition has no name"))
    }

    fn define(&mut self, def: &cst::Definition) -> Result<(), SchemaError> {
        use cst::Definition as D;
        let (name_node, kind, description) = match def {
            D::OperationDefinition(_) | D::FragmentDefinition(_) => {
                return Err(self.error(def.syntax(), "a query or fragment is not part of a schema"));
            }
            D::DirectiveDefinition(_) => return Ok(()),
            D::SchemaDefinition(s) => {
                return self.schema_roots(s.root_operation_type_definitions());
            }
            D::ScalarTypeDefinition(t) => (t.name(), TypeKind::Scalar, t.description()),
            D::ObjectTypeDefinition(t) => (t.name(), TypeKind::Object, t.description()),
            D::InterfaceTypeDefinition(t) => (t.name(), TypeKind::Interface, t.description()),
            D::UnionTypeDefinition(t) => (t.name(), TypeKind::Union, t.description()),
            D::EnumTypeDefinition(t) => (t.name(), TypeKind::Enum, t.description()),
            D::InputObjectTypeDefinition(t) => (t.name(), TypeKind::InputObject, t.description()),
            _ => return Ok(()),
        };
        let name_syntax = name_node.as_ref().map(|n| n.syntax().clone());
        let name = self.name(name_node, def.syntax())?;
        if self.index.contains_key(&name) {
            let node = name_syntax.unwrap_or_else(|| def.syntax().clone());
            return Err(self.error(&node, format!("the type {name} is defined twice")));
        }
        self.index.insert(name.clone(), self.types.len());
        self.types.push(SchemaType {
            name,
            kind,
            description: description_of(description),
            fields: Vec::new(),
            input_fields: Vec::new(),
            enum_values: Vec::new(),
            interfaces: Vec::new(),
            possible_types: Vec::new(),
        });
        self.members(def)
    }

    fn extend(&mut self, def: &cst::Definition) -> Result<(), SchemaError> {
        use cst::Definition as D;
        let (name_node, kind) = match def {
            D::SchemaExtension(s) => return self.schema_roots(s.root_operation_type_definitions()),
            D::ScalarTypeExtension(t) => (t.name(), TypeKind::Scalar),
            D::ObjectTypeExtension(t) => (t.name(), TypeKind::Object),
            D::InterfaceTypeExtension(t) => (t.name(), TypeKind::Interface),
            D::UnionTypeExtension(t) => (t.name(), TypeKind::Union),
            D::EnumTypeExtension(t) => (t.name(), TypeKind::Enum),
            D::InputObjectTypeExtension(t) => (t.name(), TypeKind::InputObject),
            _ => return Ok(()),
        };
        let node = name_node
            .as_ref()
            .map_or_else(|| def.syntax().clone(), |n| n.syntax().clone());
        let name = self.name(name_node, def.syntax())?;
        match self.index.get(&name).map(|&i| self.types[i].kind) {
            None if BUILT_IN_SCALARS.contains(&name.as_str()) && kind == TypeKind::Scalar => Ok(()),
            None => Err(self.error(&node, format!("cannot extend {name}, which is not defined"))),
            Some(found) if found != kind => Err(self.error(
                &node,
                format!("cannot extend {name} as a different kind of type"),
            )),
            Some(_) => self.members(def),
        }
    }

    /// Appends the members a definition or extension declares to its already indexed type.
    fn members(&mut self, def: &cst::Definition) -> Result<(), SchemaError> {
        use cst::Definition as D;
        let name = def.name().map(|n| n.text().to_string()).unwrap_or_default();
        let Some(&slot) = self.index.get(&name) else {
            return Ok(());
        };
        let (fields, interfaces, members, values, inputs) = match def {
            D::ObjectTypeDefinition(t) => (
                t.fields_definition(),
                t.implements_interfaces(),
                None,
                None,
                None,
            ),
            D::ObjectTypeExtension(t) => (
                t.fields_definition(),
                t.implements_interfaces(),
                None,
                None,
                None,
            ),
            D::InterfaceTypeDefinition(t) => (
                t.fields_definition(),
                t.implements_interfaces(),
                None,
                None,
                None,
            ),
            D::InterfaceTypeExtension(t) => (
                t.fields_definition(),
                t.implements_interfaces(),
                None,
                None,
                None,
            ),
            D::UnionTypeDefinition(t) => (None, None, t.union_member_types(), None, None),
            D::UnionTypeExtension(t) => (None, None, t.union_member_types(), None, None),
            D::EnumTypeDefinition(t) => (None, None, None, t.enum_values_definition(), None),
            D::EnumTypeExtension(t) => (None, None, None, t.enum_values_definition(), None),
            D::InputObjectTypeDefinition(t) => {
                (None, None, None, None, t.input_fields_definition())
            }
            D::InputObjectTypeExtension(t) => (None, None, None, None, t.input_fields_definition()),
            _ => (None, None, None, None, None),
        };
        let mut new_fields = Vec::new();
        for f in fields.iter().flat_map(|f| f.field_definitions()) {
            new_fields.push(self.field(&f)?);
        }
        let mut new_inputs = Vec::new();
        for v in inputs.iter().flat_map(|i| i.input_value_definitions()) {
            new_inputs.push(self.input(&v)?);
        }
        let new_interfaces = self.named_list(interfaces.iter().flat_map(|i| i.named_types()))?;
        let new_members = self.named_list(members.iter().flat_map(|m| m.named_types()))?;
        let mut new_values = Vec::new();
        for v in values.iter().flat_map(|v| v.enum_value_definitions()) {
            let name = v
                .enum_value()
                .and_then(|e| e.name())
                .map(|n| n.text().to_string())
                .ok_or_else(|| self.error(v.syntax(), "an enum value has no name"))?;
            new_values.push(EnumValue {
                name,
                description: description_of(v.description()),
                deprecation: deprecation_of(v.directives()),
            });
        }
        let ty = &mut self.types[slot];
        ty.fields.extend(new_fields);
        ty.input_fields.extend(new_inputs);
        ty.interfaces.extend(new_interfaces);
        ty.possible_types.extend(new_members);
        ty.enum_values.extend(new_values);
        Ok(())
    }

    fn named_list(
        &mut self,
        types: impl Iterator<Item = cst::NamedType>,
    ) -> Result<Vec<String>, SchemaError> {
        let mut names = Vec::new();
        for t in types {
            let name = self.name(t.name(), t.syntax())?;
            self.refs.push((name.clone(), offset(t.syntax())));
            names.push(name);
        }
        Ok(names)
    }

    fn field(&mut self, f: &cst::FieldDefinition) -> Result<Field, SchemaError> {
        let name = self.name(f.name(), f.syntax())?;
        let mut args = Vec::new();
        for a in f
            .arguments_definition()
            .iter()
            .flat_map(|a| a.input_value_definitions())
        {
            args.push(self.input(&a)?);
        }
        Ok(Field {
            ty: self.type_ref(f.ty(), f.syntax())?,
            description: description_of(f.description()),
            deprecation: deprecation_of(f.directives()),
            name,
            args,
        })
    }

    fn input(&mut self, v: &cst::InputValueDefinition) -> Result<InputValue, SchemaError> {
        let name = self.name(v.name(), v.syntax())?;
        let default_value = v
            .default_value()
            .and_then(|d| d.value())
            .map(|value| source_of(value.syntax()));
        Ok(InputValue {
            ty: self.type_ref(v.ty(), v.syntax())?,
            description: description_of(v.description()),
            name,
            default_value,
        })
    }

    /// Reads `[User!]!` and the like inside out, recording the innermost name as a reference.
    fn type_ref(
        &mut self,
        ty: Option<cst::Type>,
        owner: &SyntaxNode,
    ) -> Result<TypeRef, SchemaError> {
        let ty = ty.ok_or_else(|| self.error(owner, "a field or argument has no type"))?;
        match ty {
            cst::Type::NamedType(n) => {
                let named = self.named_list(std::iter::once(n))?.remove(0);
                Ok(TypeRef {
                    display: named.clone(),
                    named,
                })
            }
            cst::Type::ListType(l) => {
                let inner = self.type_ref(l.ty(), l.syntax())?;
                Ok(TypeRef {
                    display: format!("[{}]", inner.display),
                    named: inner.named,
                })
            }
            cst::Type::NonNullType(n) => {
                let inner = match (n.named_type(), n.list_type()) {
                    (Some(named), _) => {
                        self.type_ref(Some(cst::Type::NamedType(named)), n.syntax())?
                    }
                    (None, Some(list)) => {
                        self.type_ref(Some(cst::Type::ListType(list)), n.syntax())?
                    }
                    (None, None) => {
                        return Err(self.error(n.syntax(), "a non-null type has no inner type"));
                    }
                };
                Ok(TypeRef {
                    display: format!("{}!", inner.display),
                    named: inner.named,
                })
            }
        }
    }

    fn schema_roots(
        &mut self,
        defs: cst::CstChildren<cst::RootOperationTypeDefinition>,
    ) -> Result<(), SchemaError> {
        for def in defs {
            let op = def.operation_type();
            let root = match op {
                Some(o) if o.query_token().is_some() => Root::Query,
                Some(o) if o.mutation_token().is_some() => Root::Mutation,
                Some(o) if o.subscription_token().is_some() => Root::Subscription,
                _ => return Err(self.error(def.syntax(), "a root operation has no kind")),
            };
            let named = def
                .named_type()
                .ok_or_else(|| self.error(def.syntax(), "a root operation has no type"))?;
            let name = self.name(named.name(), named.syntax())?;
            self.roots
                .push((root, name.clone(), offset(named.syntax())));
            self.refs.push((name, offset(named.syntax())));
        }
        Ok(())
    }

    /// Appends the undeclared built-in scalars, checks every reference, and fills in roots and
    /// the implementors of each interface.
    fn finish(mut self) -> Result<Schema, SchemaError> {
        for scalar in BUILT_IN_SCALARS {
            if !self.index.contains_key(scalar) {
                self.index.insert(scalar.to_owned(), self.types.len());
                self.types.push(SchemaType {
                    name: scalar.to_owned(),
                    kind: TypeKind::Scalar,
                    description: None,
                    fields: Vec::new(),
                    input_fields: Vec::new(),
                    enum_values: Vec::new(),
                    interfaces: Vec::new(),
                    possible_types: Vec::new(),
                });
            }
        }
        if let Some((name, at_offset)) = self.refs.iter().find(|(n, _)| !self.index.contains_key(n))
        {
            return Err(at(self.text, *at_offset, &format!("unknown type {name}")));
        }
        let implementors: Vec<(String, Vec<String>)> = self
            .types
            .iter()
            .filter(|t| t.kind == TypeKind::Object)
            .map(|t| (t.name.clone(), t.interfaces.clone()))
            .collect();
        for ty in self
            .types
            .iter_mut()
            .filter(|t| t.kind == TypeKind::Interface)
        {
            ty.possible_types = implementors
                .iter()
                .filter(|(_, ifaces)| ifaces.contains(&ty.name))
                .map(|(name, _)| name.clone())
                .collect();
        }
        let root = |which: Root, default: &str| -> Option<String> {
            if self.roots.is_empty() {
                self.index.contains_key(default).then(|| default.to_owned())
            } else {
                self.roots
                    .iter()
                    .rev()
                    .find(|(r, _, _)| *r == which)
                    .map(|(_, name, _)| name.clone())
            }
        };
        Ok(Schema {
            query_type: root(Root::Query, "Query"),
            mutation_type: root(Root::Mutation, "Mutation"),
            subscription_type: root(Root::Subscription, "Subscription"),
            types: self.types,
        })
    }
}

fn description_of(description: Option<cst::Description>) -> Option<String> {
    description
        .and_then(|d| d.string_value())
        .map(|s| String::from(&s))
}

/// The `@deprecated` reason, the default one when the directive has no string reason.
fn deprecation_of(directives: Option<cst::Directives>) -> Option<String> {
    let directive = directives?
        .directives()
        .find(|d| d.name().is_some_and(|n| n.text().as_str() == "deprecated"))?;
    let reason = directive
        .arguments()
        .into_iter()
        .flat_map(|a| a.arguments())
        .find(|a| a.name().is_some_and(|n| n.text().as_str() == "reason"))
        .and_then(|a| a.value());
    Some(match reason {
        Some(cst::Value::StringValue(s)) => String::from(&s),
        _ => DEFAULT_DEPRECATION.to_owned(),
    })
}

/// A value's source text without the whitespace and commas the parser attaches around it.
fn source_of(node: &SyntaxNode) -> String {
    node.text()
        .to_string()
        .trim_matches(|c: char| c.is_whitespace() || c == ',')
        .to_owned()
}

/// The byte offset of a node's first significant token, skipping attached whitespace.
fn offset(node: &SyntaxNode) -> usize {
    let start = usize::from(node.text_range().start());
    let text = node.text().to_string();
    start
        + (text.len()
            - text
                .trim_start_matches(|c: char| c.is_whitespace() || c == ',')
                .len())
}

/// An SDL error at a byte offset, as a 1-based line and a column counted in characters.
fn at(text: &str, index: usize, detail: &str) -> SchemaError {
    let mut index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    let before = &text[..index];
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    SchemaError::Sdl {
        line: before.matches('\n').count() + 1,
        column: before[line_start..].chars().count() + 1,
        detail: detail.to_owned(),
    }
}
