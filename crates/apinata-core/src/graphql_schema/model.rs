//! The introspection model: a walk over `serde_json::Value`, since the `ofType` chain is
//! recursive and every introspection field may be null.

use serde_json::{Map, Value};

use super::SchemaError;

/// A GraphQL schema as read from an introspection answer, types in introspection order.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Schema {
    query_type: Option<String>,
    mutation_type: Option<String>,
    subscription_type: Option<String>,
    types: Vec<SchemaType>,
}

/// The kind of a named GraphQL type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TypeKind {
    /// A scalar such as `String` or `ID`.
    Scalar,
    /// An object type with fields.
    Object,
    /// An interface implemented by objects.
    Interface,
    /// A union of object types.
    Union,
    /// An enum with named values.
    Enum,
    /// An input object used as an argument.
    InputObject,
}

/// One named type of the schema.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct SchemaType {
    name: String,
    kind: TypeKind,
    description: Option<String>,
    fields: Vec<Field>,
    input_fields: Vec<InputValue>,
    enum_values: Vec<EnumValue>,
    interfaces: Vec<String>,
    possible_types: Vec<String>,
}

/// A field of an object or interface type.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Field {
    name: String,
    description: Option<String>,
    args: Vec<InputValue>,
    ty: TypeRef,
    deprecation: Option<String>,
}

/// An argument or an input object field.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct InputValue {
    name: String,
    description: Option<String>,
    ty: TypeRef,
    default_value: Option<String>,
}

/// One value of an enum type.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct EnumValue {
    name: String,
    description: Option<String>,
    deprecation: Option<String>,
}

/// A reference to a type with its list and non-null wrappers, such as `[User!]!`.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct TypeRef {
    named: String,
    display: String,
}

impl Schema {
    /// Parses an introspection answer, with or without its `data` wrapper.
    pub fn from_json(text: &str) -> Result<Schema, SchemaError> {
        let value: Value = serde_json::from_str(text).map_err(|e| SchemaError::NotJson {
            detail: e.to_string(),
        })?;
        // A `data` object wins over anything beside it, such as `errors`.
        let root = match value.get("data") {
            Some(data @ Value::Object(_)) => data,
            _ => &value,
        };
        let Some(schema) = root.get("__schema").and_then(Value::as_object) else {
            return Err(SchemaError::NoData);
        };
        let Some(types) = schema.get("types").and_then(Value::as_array) else {
            return Err(invalid("types is not a list"));
        };
        Ok(Schema {
            query_type: root_name(schema, "queryType"),
            mutation_type: root_name(schema, "mutationType"),
            subscription_type: root_name(schema, "subscriptionType"),
            types: types.iter().map(parse_type).collect::<Result<_, _>>()?,
        })
    }

    /// The name of the query root type.
    pub fn query_type(&self) -> Option<&str> {
        self.query_type.as_deref()
    }

    /// The name of the mutation root type.
    pub fn mutation_type(&self) -> Option<&str> {
        self.mutation_type.as_deref()
    }

    /// The name of the subscription root type.
    pub fn subscription_type(&self) -> Option<&str> {
        self.subscription_type.as_deref()
    }

    /// Every named type, in introspection order.
    pub fn types(&self) -> &[SchemaType] {
        &self.types
    }

    /// The type with this exact name.
    pub fn type_named(&self, name: &str) -> Option<&SchemaType> {
        self.types.iter().find(|t| t.name == name)
    }
}

impl SchemaType {
    /// The type's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The type's kind.
    pub fn kind(&self) -> TypeKind {
        self.kind
    }

    /// The type's description.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// The fields of an object or interface; empty otherwise.
    pub fn fields(&self) -> &[Field] {
        &self.fields
    }

    /// The fields of an input object; empty otherwise.
    pub fn input_fields(&self) -> &[InputValue] {
        &self.input_fields
    }

    /// The values of an enum; empty otherwise.
    pub fn enum_values(&self) -> &[EnumValue] {
        &self.enum_values
    }

    /// The names of the interfaces this type implements.
    pub fn interfaces(&self) -> &[String] {
        &self.interfaces
    }

    /// The names of a union's members or an interface's implementors.
    pub fn possible_types(&self) -> &[String] {
        &self.possible_types
    }
}

impl Field {
    /// The field's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The field's description.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// The field's arguments.
    pub fn args(&self) -> &[InputValue] {
        &self.args
    }

    /// The field's type.
    pub fn ty(&self) -> &TypeRef {
        &self.ty
    }

    /// The deprecation reason, empty when deprecated without one; `None` when live.
    pub fn deprecation(&self) -> Option<&str> {
        self.deprecation.as_deref()
    }
}

impl InputValue {
    /// The value's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The value's description.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// The value's type.
    pub fn ty(&self) -> &TypeRef {
        &self.ty
    }

    /// The default value as GraphQL source text.
    pub fn default_value(&self) -> Option<&str> {
        self.default_value.as_deref()
    }
}

impl EnumValue {
    /// The value's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The value's description.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// The deprecation reason, empty when deprecated without one; `None` when live.
    pub fn deprecation(&self) -> Option<&str> {
        self.deprecation.as_deref()
    }
}

impl TypeRef {
    /// The innermost named type, such as `User` for `[User!]!`.
    pub fn named(&self) -> &str {
        &self.named
    }

    /// The type as written in GraphQL, such as `[User!]!`.
    pub fn display(&self) -> &str {
        &self.display
    }
}

fn invalid(detail: impl Into<String>) -> SchemaError {
    SchemaError::Invalid {
        detail: detail.into(),
    }
}

fn str_of(object: &Map<String, Value>, key: &str) -> Option<String> {
    object.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn root_name(schema: &Map<String, Value>, key: &str) -> Option<String> {
    schema
        .get(key)
        .and_then(|v| v.get("name"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// The array under `key`; absent or null means empty, anything else is malformed.
fn list<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    owner: &str,
) -> Result<&'a [Value], SchemaError> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(&[]),
        Some(Value::Array(items)) => Ok(items),
        Some(_) => Err(invalid(format!("{key} of {owner} is not a list"))),
    }
}

fn deprecation(object: &Map<String, Value>) -> Option<String> {
    let deprecated = object.get("isDeprecated").and_then(Value::as_bool) == Some(true);
    deprecated.then(|| str_of(object, "deprecationReason").unwrap_or_default())
}

fn parse_type(value: &Value) -> Result<SchemaType, SchemaError> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid("a type is not an object"))?;
    let name = str_of(object, "name").ok_or_else(|| invalid("a type has no name"))?;
    let kind = match object.get("kind").and_then(Value::as_str) {
        Some("SCALAR") => TypeKind::Scalar,
        Some("OBJECT") => TypeKind::Object,
        Some("INTERFACE") => TypeKind::Interface,
        Some("UNION") => TypeKind::Union,
        Some("ENUM") => TypeKind::Enum,
        Some("INPUT_OBJECT") => TypeKind::InputObject,
        Some(other) => return Err(invalid(format!("{name} has the unknown kind {other}"))),
        None => return Err(invalid(format!("{name} has no kind"))),
    };
    let fields = list(object, "fields", &name)?
        .iter()
        .map(|f| parse_field(f, &name))
        .collect::<Result<_, _>>()?;
    let input_fields = list(object, "inputFields", &name)?
        .iter()
        .map(|f| parse_input(f, &name))
        .collect::<Result<_, _>>()?;
    let enum_values = list(object, "enumValues", &name)?
        .iter()
        .map(|v| parse_enum_value(v, &name))
        .collect::<Result<_, _>>()?;
    let interfaces = names(list(object, "interfaces", &name)?, &name)?;
    let possible_types = names(list(object, "possibleTypes", &name)?, &name)?;
    Ok(SchemaType {
        description: str_of(object, "description"),
        name,
        kind,
        fields,
        input_fields,
        enum_values,
        interfaces,
        possible_types,
    })
}

fn names(items: &[Value], owner: &str) -> Result<Vec<String>, SchemaError> {
    items
        .iter()
        .map(|v| {
            v.get("name")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| invalid(format!("a type named by {owner} has no name")))
        })
        .collect()
}

fn parse_field(value: &Value, owner: &str) -> Result<Field, SchemaError> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid(format!("a field of {owner} is not an object")))?;
    let name =
        str_of(object, "name").ok_or_else(|| invalid(format!("a field of {owner} has no name")))?;
    let at = format!("{owner}.{name}");
    let ty = parse_type_ref(object.get("type"), &at)?;
    let args = list(object, "args", &at)?
        .iter()
        .map(|a| parse_input(a, &at))
        .collect::<Result<_, _>>()?;
    Ok(Field {
        description: str_of(object, "description"),
        deprecation: deprecation(object),
        name,
        args,
        ty,
    })
}

fn parse_input(value: &Value, owner: &str) -> Result<InputValue, SchemaError> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid(format!("an input value of {owner} is not an object")))?;
    let name = str_of(object, "name")
        .ok_or_else(|| invalid(format!("an input value of {owner} has no name")))?;
    let ty = parse_type_ref(object.get("type"), &format!("{owner}.{name}"))?;
    Ok(InputValue {
        description: str_of(object, "description"),
        default_value: str_of(object, "defaultValue"),
        name,
        ty,
    })
}

fn parse_enum_value(value: &Value, owner: &str) -> Result<EnumValue, SchemaError> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid(format!("a value of {owner} is not an object")))?;
    let name =
        str_of(object, "name").ok_or_else(|| invalid(format!("a value of {owner} has no name")))?;
    Ok(EnumValue {
        description: str_of(object, "description"),
        deprecation: deprecation(object),
        name,
    })
}

/// Unwraps the `ofType` chain from the outside in, so the display is built as
/// prefix brackets, the innermost name, then the closing suffixes in reverse.
fn parse_type_ref(value: Option<&Value>, at: &str) -> Result<TypeRef, SchemaError> {
    let mut current = value
        .filter(|v| v.is_object())
        .ok_or_else(|| invalid(format!("{at} has no type")))?;
    let mut prefix = String::new();
    let mut suffix = Vec::new();
    loop {
        match current.get("kind").and_then(Value::as_str) {
            Some("NON_NULL") => suffix.push("!"),
            Some("LIST") => {
                prefix.push('[');
                suffix.push("]");
            }
            _ => {
                let named = current
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid(format!("the type of {at} has no name")))?
                    .to_owned();
                let display = format!(
                    "{prefix}{named}{}",
                    suffix.iter().rev().copied().collect::<String>()
                );
                return Ok(TypeRef { named, display });
            }
        }
        current = current
            .get("ofType")
            .filter(|v| v.is_object())
            .ok_or_else(|| invalid(format!("the type of {at} ends in a wrapper")))?;
    }
}
