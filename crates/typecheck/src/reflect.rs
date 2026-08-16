//! Compile-time reflection: map resolved [`Ty`] to the stdlib `Type` ADT as [`ConstValue`].

use internment::Intern;

use crate::representation::Repr;
use crate::{TypeRegistry, normal_expr::Normalize, ty::Ty};
use ast::expr::{Expr, Literal, TagCall, Typed};
use ast::span::SpanId;
use ast::ty::{DeclarationKey, NamedTypeInstance, PackageSourceKey, TyArg};
use ast::{ConstValue, HashFloat};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};

thread_local! {
    static REFLECT_RECURSION_DEPTH: Cell<u32> = const { Cell::new(0) };
}

struct ReflectRecursionGuard {
    entered: bool,
}

impl ReflectRecursionGuard {
    fn enter() -> Self {
        REFLECT_RECURSION_DEPTH.with(|depth| {
            depth.set(depth.get().saturating_add(1));
            if cfg!(debug_assertions) && depth.get() > 256 {
                panic!(
                    "reflection recursion overflow while reflecting: {}",
                    "depth exceeded 256"
                );
            }
        });
        Self { entered: true }
    }
}

impl Drop for ReflectRecursionGuard {
    fn drop(&mut self) {
        if self.entered {
            REFLECT_RECURSION_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct ReflectionRecursionState {
    named: HashMap<String, i128>,
    structural: HashMap<String, i128>,
}

impl ReflectionRecursionState {
    fn insert_named(&mut self, key: String) -> Option<i128> {
        if let Some(depth) = self.named.get(&key) {
            return Some(*depth);
        }
        let depth = i128::try_from(self.named.len()).unwrap_or(i128::MAX);
        self.named.insert(key, depth);
        None
    }

    fn remove_named(&mut self, key: &String) {
        self.named.remove(key);
    }

    fn insert_structural(&mut self, key: String) -> Option<i128> {
        if let Some(depth) = self.structural.get(&key) {
            return Some(*depth);
        }
        let depth = i128::try_from(self.structural.len()).unwrap_or(i128::MAX);
        self.structural.insert(key, depth);
        None
    }

    fn remove_structural(&mut self, key: &String) {
        self.structural.remove(key);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectionShapeConstructors {
    pub signed: ReflectionTag,
    pub unsigned: ReflectionTag,
    pub raw_pointer: ReflectionTag,
    pub safe_reference: ReflectionTag,
    pub record: ReflectionTag,
    pub union: ReflectionTag,
    pub tuple: ReflectionTag,
    pub array: ReflectionTag,
    pub integer: ReflectionTag,
    pub unit: ReflectionTag,
    pub opaque: ReflectionTag,
    pub unsupported: ReflectionTag,
    pub recursive_reference: ReflectionTag,
    pub observe_permission: ReflectionTag,
    pub mutate_permission: ReflectionTag,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReflectionTag {
    pub name: Intern<String>,
    pub qual_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReflectionArgumentKey {
    repr: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ReflectionTypeKey {
    declaration: Option<DeclarationKey>,
    display_name: String,
    arguments: Vec<ReflectionArgumentKey>,
}

impl ReflectionTypeKey {
    fn of_named(instance: &NamedTypeInstance, registry: Option<&TypeRegistry>) -> Self {
        let declaration = registry.and_then(|registry| registry.declaration_key(instance).cloned());
        let display_name = instance.declaration.name.as_str().to_string();
        let arguments = instance
            .arguments
            .iter()
            .map(|(_, argument)| ReflectionArgumentKey::of_ty_arg(argument, registry))
            .collect();
        Self {
            declaration,
            display_name,
            arguments,
        }
    }

    fn to_cycle_key(&self) -> String {
        if let Some(key) = &self.declaration {
            let args = if self.arguments.is_empty() {
                String::new()
            } else {
                let rendered = self
                    .arguments
                    .iter()
                    .map(|argument| argument.repr.as_str())
                    .collect::<Vec<_>>()
                    .join(",");
                format!("<{rendered}>")
            };
            return format!("{}{}", declaration_key_path(key), args);
        }
        let args = if self.arguments.is_empty() {
            String::new()
        } else {
            let rendered = self
                .arguments
                .iter()
                .map(|argument| argument.repr.as_str())
                .collect::<Vec<_>>()
                .join(",");
            format!("<{rendered}>")
        };
        format!("{}{}", self.display_name, args)
    }
}

impl ReflectionArgumentKey {
    fn of_ty_arg(arg: &TyArg, registry: Option<&TypeRegistry>) -> Self {
        match arg {
            TyArg::Type(ty) => Self {
                repr: type_argument_key_with_seen(ty, registry, &mut HashSet::new()),
            },
            TyArg::Const(expr) => Self {
                repr: expr.to_string(),
            },
        }
    }
}

fn type_argument_key_with_seen(
    ty: &Ty,
    registry: Option<&TypeRegistry>,
    seen: &mut HashSet<DeclarationKey>,
) -> String {
    match ty.named_instance() {
        Some(instance) => {
            let Some(declaration_key) =
                registry.and_then(|registry| registry.declaration_key(instance))
            else {
                return ty.format_for_hover();
            };
            if !seen.insert(declaration_key.clone()) {
                return declaration_key_path(declaration_key);
            }
            let rendered_arguments = instance
                .arguments
                .iter()
                .map(|(_, arg)| match arg {
                    TyArg::Type(arg_ty) => type_argument_key_with_seen(arg_ty, registry, seen),
                    TyArg::Const(expr) => expr.to_string(),
                })
                .collect::<Vec<_>>()
                .join(",");
            seen.remove(declaration_key);
            format!(
                "{}<{}>",
                declaration_key_path(declaration_key),
                rendered_arguments
            )
        }
        None => ty.format_for_hover(),
    }
}

fn declaration_key_path(key: &DeclarationKey) -> String {
    let source = match &key.package.source {
        PackageSourceKey::Workspace => "workspace".to_string(),
        PackageSourceKey::Registry { registry } => format!("registry:{registry}"),
        PackageSourceKey::Git { url, revision } => format!("git:{url}#{revision}"),
        PackageSourceKey::Path {
            parent_instance,
            relative_path,
        } => format!("path:{parent_instance}:{relative_path}"),
    };

    let module = if key.module.is_empty() {
        "<root>".to_string()
    } else {
        key.module
            .iter()
            .map(|part| part.as_str())
            .collect::<Vec<_>>()
            .join(".")
    };

    format!(
        "{}:{}:{}:{}:{}",
        key.package.name.as_str(),
        key.package.version.as_str(),
        source,
        module,
        key.declaration.as_str()
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReflectedRepresentation {
    Bits {
        width: u32,
    },
    Address {
        address_space: u32,
    },
    Product {
        fields: Vec<ReflectedRepresentation>,
    },
    Sum {
        variants: Vec<ReflectedRepresentation>,
    },
    Array {
        element: Box<ReflectedRepresentation>,
        length: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ReflectedIdentity {
    Named(DeclarationKey),
    Anonymous,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReflectedSemantics {
    Integer {
        validity: ast::integer::IntegerValidity,
        representation_rule: ast::integer::IntegerRepresentationRule,
        interpretation: Option<ast::integer::IntegerInterpretation>,
    },
    RawPointer {
        pointee: Box<ReflectedType>,
    },
    SafeReference {
        pointee: Box<ReflectedType>,
        mutable: bool,
    },
    Record {
        fields: Vec<(String, ReflectedType)>,
        declaration_name: String,
        declaration_key: Option<String>,
    },
    Union {
        variants: Vec<(String, Vec<(String, ReflectedType)>)>,
        declaration_name: String,
        declaration_key: Option<String>,
    },
    Tuple {
        elements: Vec<ReflectedType>,
    },
    Unit,
    Array {
        element: Box<ReflectedType>,
        length: Option<u64>,
        unresolved_length: Option<ConstValue>,
    },
    Opaque {
        name: String,
    },
    Literal {
        value: ConstValue,
    },
    Float,
    UnresolvedLiteral,
    Unsupported {
        reason: String,
    },
    RecursiveReference {
        declaration: DeclarationKey,
        arguments: Vec<ReflectionArgumentKey>,
        depth: i128,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectedType {
    pub identity: ReflectedIdentity,
    pub arguments: Vec<ReflectedArgument>,
    pub semantics: ReflectedSemantics,
    pub representation: ReflectedRepresentation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReflectedArgument {
    Type(Box<ReflectedType>),
    Const(ConstValue),
}

impl From<Repr> for ReflectedRepresentation {
    fn from(repr: Repr) -> Self {
        match repr {
            Repr::Bits { width } => Self::Bits { width },
            Repr::Address { address_space } => Self::Address { address_space },
            Repr::Product { fields } => Self::Product {
                fields: fields.into_iter().map(Self::from).collect(),
            },
            Repr::Sum { variants } => Self::Sum {
                variants: variants.into_iter().map(Self::from).collect(),
            },
            Repr::Array { element, length } => Self::Array {
                element: Box::new(Self::from(*element)),
                length,
            },
        }
    }
}

impl ReflectedType {
    pub(crate) fn to_const_value_with_schema(
        &self,
        schema: &ReflectionShapeConstructors,
    ) -> ConstValue {
        match &self.semantics {
            ReflectedSemantics::Integer {
                validity,
                representation_rule,
                interpretation: Some(interpretation),
            } => {
                let width = ast::integer::resolve_representation(validity, representation_rule)
                    .map(|representation| representation.width().get())
                    .unwrap_or(0);
                tag(
                    &schema.integer,
                    vec![
                        ConstValue::Int(width.into()),
                        integer_sign_tag(
                            matches!(interpretation, ast::integer::IntegerInterpretation::Signed),
                            &schema.signed,
                            &schema.unsigned,
                        ),
                    ],
                )
            }
            ReflectedSemantics::Integer {
                interpretation: None,
                ..
            } => tag(
                &schema.opaque,
                vec![ConstValue::String("anonymous integer".to_string())],
            ),
            ReflectedSemantics::RawPointer { pointee } => tag(
                &schema.raw_pointer,
                vec![pointee.to_const_value_with_schema(schema)],
            ),
            ReflectedSemantics::SafeReference { pointee, mutable } => tag(
                &schema.safe_reference,
                vec![
                    pointee.to_const_value_with_schema(schema),
                    reference_permission_tag(
                        *mutable,
                        &schema.observe_permission,
                        &schema.mutate_permission,
                    ),
                ],
            ),
            ReflectedSemantics::Record {
                fields,
                declaration_name,
                declaration_key: _,
            } => tag(
                &schema.record,
                vec![
                    ConstValue::String(reflected_ty_decl_name(
                        &self.identity,
                        &self.arguments,
                        declaration_name,
                    )),
                    ConstValue::List(
                        fields
                            .iter()
                            .map(|(name, field)| {
                                record_named(name, field.to_const_value_with_schema(schema))
                            })
                            .collect::<Vec<_>>()
                            .into(),
                    ),
                ],
            ),
            ReflectedSemantics::Union {
                variants,
                declaration_name,
                declaration_key: _,
            } => tag(
                &schema.union,
                vec![
                    ConstValue::String(reflected_ty_decl_name(
                        &self.identity,
                        &self.arguments,
                        declaration_name,
                    )),
                    ConstValue::List(
                        variants
                            .iter()
                            .map(|(name, fields)| {
                                record_variant(
                                    name,
                                    fields
                                        .iter()
                                        .map(|(name, field)| {
                                            record_named(
                                                name,
                                                field.to_const_value_with_schema(schema),
                                            )
                                        })
                                        .collect::<Vec<_>>(),
                                )
                            })
                            .collect::<Vec<_>>()
                            .into(),
                    ),
                ],
            ),
            ReflectedSemantics::Tuple { elements } => tag(
                &schema.tuple,
                vec![ConstValue::List(
                    elements
                        .iter()
                        .map(|element| element.to_const_value_with_schema(schema))
                        .collect::<Vec<_>>()
                        .into(),
                )],
            ),
            ReflectedSemantics::Array {
                element,
                length,
                unresolved_length,
            } => {
                let Some(length) = length.filter(|_| unresolved_length.is_none()) else {
                    return tag(
                        &schema.unsupported,
                        vec![ConstValue::String("unresolved array length".to_string())],
                    );
                };
                tag(
                    &schema.array,
                    vec![
                        element.to_const_value_with_schema(schema),
                        ConstValue::Int(length.into()),
                    ],
                )
            }
            ReflectedSemantics::Opaque { name } => {
                tag(&schema.opaque, vec![ConstValue::String(name.clone())])
            }
            ReflectedSemantics::Literal { value } => record_named("Literal", value.clone()),
            ReflectedSemantics::Float => tag(
                &schema.unsupported,
                vec![ConstValue::String("float".to_string())],
            ),
            ReflectedSemantics::UnresolvedLiteral => tag(
                &schema.unsupported,
                vec![ConstValue::String("unresolved-literal".to_string())],
            ),
            ReflectedSemantics::Unsupported { reason } => tag(
                &schema.unsupported,
                vec![ConstValue::String(reason.clone())],
            ),
            ReflectedSemantics::RecursiveReference {
                declaration,
                arguments,
                depth,
            } => tag(
                &schema.recursive_reference,
                vec![
                    ConstValue::String(recursive_reference_name(declaration, arguments)),
                    ConstValue::Int((*depth).into()),
                ],
            ),
            ReflectedSemantics::Unit => tag(&schema.unit, vec![ConstValue::List(vec![].into())]),
        }
    }
}

fn reflected_ty_decl_name(
    identity: &ReflectedIdentity,
    arguments: &[ReflectedArgument],
    name: &str,
) -> String {
    match identity {
        ReflectedIdentity::Named(declaration_key) => {
            if arguments.is_empty() {
                format!("{name}[{}]", declaration_key_path(declaration_key))
            } else {
                format!(
                    "{name}[{}]<{}>",
                    declaration_key_path(declaration_key),
                    reflected_arguments_to_display(arguments)
                )
            }
        }
        ReflectedIdentity::Anonymous => {
            if arguments.is_empty() {
                name.to_owned()
            } else {
                format!("{name}<{}>", reflected_arguments_to_display(arguments))
            }
        }
    }
}

fn reflected_arguments_to_display(arguments: &[ReflectedArgument]) -> String {
    arguments
        .iter()
        .map(reflected_argument_to_display)
        .collect::<Vec<_>>()
        .join(", ")
}

fn reflected_argument_to_display(argument: &ReflectedArgument) -> String {
    match argument {
        ReflectedArgument::Type(ty) => reflected_ty_display(ty),
        ReflectedArgument::Const(value) => const_value_display(value),
    }
}

fn reflected_ty_display(ty: &ReflectedType) -> String {
    match &ty.semantics {
        ReflectedSemantics::Integer {
            validity,
            representation_rule,
            interpretation,
        } => format!(
            "Int({:?}, {:?}, {:?})",
            validity.domain().predicate(),
            representation_rule,
            interpretation
        ),
        ReflectedSemantics::RawPointer { pointee } => {
            format!("Ptr({})", reflected_ty_display(pointee))
        }
        ReflectedSemantics::SafeReference { pointee, mutable } => {
            format!(
                "Ref({}, {})",
                reflected_ty_display(pointee),
                if *mutable { "mut" } else { "obs" }
            )
        }
        ReflectedSemantics::Record {
            declaration_name, ..
        } => declaration_name.clone(),
        ReflectedSemantics::Union {
            declaration_name, ..
        } => declaration_name.clone(),
        ReflectedSemantics::Tuple { elements } => {
            let inner = elements
                .iter()
                .map(reflected_ty_display)
                .collect::<Vec<_>>()
                .join(", ");
            format!("Tuple[{inner}]")
        }
        ReflectedSemantics::Array {
            element,
            length,
            unresolved_length,
        } => {
            let suffix = if let Some(length) = unresolved_length {
                format!("({})", const_value_display(length))
            } else if let Some(length) = length {
                format!("({length})")
            } else {
                "(<unknown>)".to_string()
            };
            let name = match length {
                Some(length) => reflected_ty_display_length(*length),
                None => "<unknown>".to_owned(),
            };
            format!("Array({}, {}){suffix}", reflected_ty_display(element), name)
        }
        ReflectedSemantics::Opaque { name } => name.clone(),
        ReflectedSemantics::Literal { value } => const_value_display(value),
        ReflectedSemantics::Float => "float".to_owned(),
        ReflectedSemantics::UnresolvedLiteral => "unresolved-literal".to_owned(),
        ReflectedSemantics::Unsupported { reason } => reason.clone(),
        ReflectedSemantics::RecursiveReference {
            declaration,
            arguments,
            depth,
        } => format!(
            "{}<{}>[{}]",
            declaration_key_path(declaration),
            arguments
                .iter()
                .map(|argument| argument.repr.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            depth
        ),
        ReflectedSemantics::Unit => "Unit".to_owned(),
    }
}

fn reflected_ty_display_length(length: u64) -> String {
    length.to_string()
}

fn const_value_display(value: &ConstValue) -> String {
    match value {
        ConstValue::ResultAlternative { label, .. } => label.as_str().to_string(),
        ConstValue::String(text) => format!("\"{text}\""),
        ConstValue::Tag {
            name,
            qual_path,
            args,
        } => {
            if args.is_empty() {
                match qual_path {
                    Some(path) => format!("{path}.{}", name.as_str()),
                    None => name.as_str().to_string(),
                }
            } else {
                let joined = args
                    .iter()
                    .map(const_value_display)
                    .collect::<Vec<_>>()
                    .join(", ");
                match qual_path {
                    Some(path) => format!("{path}.{}({joined})", name.as_str()),
                    None => format!("{}({joined})", name.as_str()),
                }
            }
        }
        ConstValue::Record { fields } => {
            let fields = fields
                .iter()
                .map(|(name, value)| format!("{}: {}", name.as_str(), const_value_display(value)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{{{fields}}}")
        }
        ConstValue::List(items) => {
            let items = items
                .iter()
                .map(const_value_display)
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{items}]")
        }
        ConstValue::Int(value) => value.to_string(),
        ConstValue::Float(HashFloat(f)) => f.to_string(),
    }
}

fn named_cycle_key(instance: &NamedTypeInstance, registry: Option<&TypeRegistry>) -> String {
    ReflectionTypeKey::of_named(instance, registry).to_cycle_key()
}

fn reflect_ty_to_reflected_type_inner(
    ty: &Ty,
    seen: &mut ReflectionRecursionState,
    registry: Option<&TypeRegistry>,
) -> ReflectedType {
    let _guard = ReflectRecursionGuard::enter();

    if let Some(reference) = registry.and_then(|registry| registry.reference_former_for_type(ty)) {
        let pointee = reflect_ty_to_reflected_type_inner(&reference.pointee, seen, registry);
        return ReflectedType {
            identity: ReflectedIdentity::Anonymous,
            arguments: Vec::new(),
            semantics: match reference.permission {
                crate::type_registry::ReferencePermission::Mutate => {
                    ReflectedSemantics::SafeReference {
                        pointee: Box::new(pointee),
                        mutable: true,
                    }
                }
                crate::type_registry::ReferencePermission::Observe => {
                    ReflectedSemantics::SafeReference {
                        pointee: Box::new(pointee),
                        mutable: false,
                    }
                }
            },
            representation: reflected_representation(ty, registry),
        };
    }

    match ty {
        Ty::Named { instance, .. } => {
            let instance = instance.clone();
            let key = named_cycle_key(&instance, registry);
            if let Some(depth) = seen.insert_named(key.clone()) {
                if let Some(declaration) = registry
                    .and_then(|registry| registry.declaration_key(&instance))
                    .cloned()
                {
                    let arguments = instance
                        .arguments
                        .iter()
                        .map(|(_, argument)| ReflectionArgumentKey::of_ty_arg(argument, registry))
                        .collect::<Vec<_>>();
                    return ReflectedType {
                        identity: ReflectedIdentity::Named(declaration.clone()),
                        arguments: Vec::new(),
                        semantics: ReflectedSemantics::RecursiveReference {
                            declaration,
                            arguments,
                            depth,
                        },
                        representation: ReflectedRepresentation::Product { fields: Vec::new() },
                    };
                }

                return ReflectedType {
                    identity: ReflectedIdentity::Anonymous,
                    arguments: Vec::new(),
                    semantics: ReflectedSemantics::Opaque {
                        name: ty.format_for_hover(),
                    },
                    representation: ReflectedRepresentation::Product { fields: Vec::new() },
                };
            }

            let Some(definition) = registry
                .and_then(|registry| registry.declaration_for_instance(&instance))
                .map(|declaration| declaration.definition.clone())
            else {
                seen.remove_named(&key);
                return ReflectedType {
                    identity: ReflectedIdentity::Anonymous,
                    arguments: Vec::new(),
                    semantics: ReflectedSemantics::Opaque {
                        name: ty.format_for_hover(),
                    },
                    representation: ReflectedRepresentation::Product { fields: Vec::new() },
                };
            };
            let identity = registry
                .and_then(|registry| registry.declaration_key(&instance))
                .cloned()
                .map(ReflectedIdentity::Named)
                .unwrap_or(ReflectedIdentity::Anonymous);

            let arguments = instance
                .arguments
                .iter()
                .map(|(_, argument)| match argument {
                    TyArg::Type(ty) => ReflectedArgument::Type(Box::new(
                        reflect_ty_to_reflected_type_inner(ty, seen, registry),
                    )),
                    TyArg::Const(expr) => {
                        let value = expr
                            .normalize_to_value()
                            .unwrap_or_else(|| ConstValue::String(expr.to_string()));
                        ReflectedArgument::Const(value)
                    }
                })
                .collect::<Vec<_>>();

            let mut reflected = reflect_ty_to_reflected_type_inner(&definition, seen, registry);
            seen.remove_named(&key);
            reflected.identity = identity;
            reflected.arguments = arguments;
            if let Some(integer) =
                registry.and_then(|registry| registry.nominal_integer_semantics_for_type(ty))
            {
                reflected.semantics = ReflectedSemantics::Integer {
                    validity: integer.validity.clone(),
                    representation_rule: integer.representation_rule.clone(),
                    interpretation: Some(integer.interpretation),
                };
            }
            reflected
        }
        Ty::AnonymousInteger { validity } => ReflectedType {
            identity: ReflectedIdentity::Anonymous,
            arguments: Vec::new(),
            semantics: ReflectedSemantics::Integer {
                validity: validity.clone(),
                representation_rule: ast::integer::IntegerRepresentationRule::InferFromValidity,
                interpretation: None,
            },
            representation: reflected_representation(ty, registry),
        },
        Ty::ResultFamily { .. } => ReflectedType {
            identity: ReflectedIdentity::Anonymous,
            arguments: Vec::new(),
            semantics: ReflectedSemantics::Opaque {
                name: ty.format_for_hover(),
            },
            representation: reflected_representation(ty, registry),
        },
        Ty::Float { .. } => ReflectedType {
            identity: ReflectedIdentity::Anonymous,
            arguments: Vec::new(),
            semantics: ReflectedSemantics::Float,
            representation: reflected_representation(ty, registry),
        },
        Ty::UnresolvedLiteral(_) => ReflectedType {
            identity: ReflectedIdentity::Anonymous,
            arguments: Vec::new(),
            semantics: ReflectedSemantics::UnresolvedLiteral,
            representation: reflected_representation(ty, registry),
        },
        Ty::Unit => ReflectedType {
            identity: ReflectedIdentity::Anonymous,
            arguments: Vec::new(),
            semantics: ReflectedSemantics::Unit,
            representation: reflected_representation(ty, registry),
        },
        Ty::Record { name, fields, .. } => {
            let key = format!("record:{name}");
            if seen.insert_structural(key.clone()).is_some() {
                return ReflectedType {
                    identity: ReflectedIdentity::Anonymous,
                    arguments: Vec::new(),
                    semantics: ReflectedSemantics::Opaque {
                        name: name.as_str().to_string(),
                    },
                    representation: reflected_representation(ty, registry),
                };
            }
            let reflected_fields = fields
                .iter()
                .map(|(fname, fty)| {
                    (
                        fname.as_str().to_string(),
                        reflect_ty_to_reflected_type_inner(fty, seen, registry),
                    )
                })
                .collect::<Vec<_>>();
            seen.remove_structural(&key);
            ReflectedType {
                identity: ReflectedIdentity::Anonymous,
                arguments: Vec::new(),
                semantics: ReflectedSemantics::Record {
                    fields: reflected_fields,
                    declaration_name: name.as_str().to_string(),
                    declaration_key: None,
                },
                representation: reflected_representation(ty, registry),
            }
        }
        Ty::Union {
            name,
            variants,
            literal_values: Some(values),
            ..
        } if !values.is_empty() => {
            let key = format!("union:{name}");
            if seen.insert_structural(key.clone()).is_some() {
                return ReflectedType {
                    identity: ReflectedIdentity::Anonymous,
                    arguments: Vec::new(),
                    semantics: ReflectedSemantics::Opaque {
                        name: name.as_str().to_string(),
                    },
                    representation: reflected_representation(ty, registry),
                };
            }
            let variants = values
                .iter()
                .map(|cv| {
                    (
                        cv.to_hover_string(),
                        vec![(
                            "value".to_string(),
                            ReflectedType {
                                identity: ReflectedIdentity::Anonymous,
                                arguments: Vec::new(),
                                semantics: ReflectedSemantics::Literal { value: cv.clone() },
                                representation: ReflectedRepresentation::Product {
                                    fields: Vec::new(),
                                },
                            },
                        )],
                    )
                })
                .collect::<Vec<_>>();
            seen.remove_structural(&key);
            ReflectedType {
                identity: ReflectedIdentity::Anonymous,
                arguments: Vec::new(),
                semantics: ReflectedSemantics::Union {
                    variants,
                    declaration_name: name.as_str().to_string(),
                    declaration_key: None,
                },
                representation: reflected_representation(ty, registry),
            }
        }
        Ty::Union { name, variants, .. } => {
            let key = format!("union:{name}");
            if seen.insert_structural(key.clone()).is_some() {
                return ReflectedType {
                    identity: ReflectedIdentity::Anonymous,
                    arguments: Vec::new(),
                    semantics: ReflectedSemantics::Opaque {
                        name: name.as_str().to_string(),
                    },
                    representation: reflected_representation(ty, registry),
                };
            }
            let variants = variants
                .iter()
                .map(|variant| {
                    (
                        variant.name.as_str().to_string(),
                        variant
                            .fields
                            .iter()
                            .map(|(fname, field)| {
                                (
                                    fname.as_str().to_string(),
                                    reflect_ty_to_reflected_type_inner(field, seen, registry),
                                )
                            })
                            .collect(),
                    )
                })
                .collect::<Vec<_>>();
            seen.remove_structural(&key);
            ReflectedType {
                identity: ReflectedIdentity::Anonymous,
                arguments: Vec::new(),
                semantics: ReflectedSemantics::Union {
                    variants,
                    declaration_name: name.as_str().to_string(),
                    declaration_key: None,
                },
                representation: reflected_representation(ty, registry),
            }
        }
        Ty::Tuple(elements) => {
            let elements = elements
                .iter()
                .map(|element| reflect_ty_to_reflected_type_inner(element, seen, registry))
                .collect();
            ReflectedType {
                identity: ReflectedIdentity::Anonymous,
                arguments: Vec::new(),
                semantics: ReflectedSemantics::Tuple { elements },
                representation: reflected_representation(ty, registry),
            }
        }
        Ty::Ptr { inner } => ReflectedType {
            identity: ReflectedIdentity::Anonymous,
            arguments: Vec::new(),
            semantics: ReflectedSemantics::RawPointer {
                pointee: Box::new(reflect_ty_to_reflected_type_inner(inner, seen, registry)),
            },
            representation: reflected_representation(ty, registry),
        },
        Ty::Address { pointee, .. } => ReflectedType {
            identity: ReflectedIdentity::Anonymous,
            arguments: Vec::new(),
            semantics: ReflectedSemantics::RawPointer {
                pointee: Box::new(reflect_ty_to_reflected_type_inner(pointee, seen, registry)),
            },
            representation: reflected_representation(ty, registry),
        },
        Ty::Ref { inner, mutable } => ReflectedType {
            identity: ReflectedIdentity::Anonymous,
            arguments: Vec::new(),
            semantics: ReflectedSemantics::SafeReference {
                pointee: Box::new(reflect_ty_to_reflected_type_inner(inner, seen, registry)),
                mutable: *mutable,
            },
            representation: reflected_representation(ty, registry),
        },
        Ty::Array { elem, size } => {
            let (length, unresolved_length) = match size.normalize_to_value() {
                Some(ConstValue::Int(value)) => ast::integer::to_u64(value)
                    .map(|length| (Some(length), None))
                    .unwrap_or((None, Some(ConstValue::Int(value)))),
                Some(value) => (None, Some(value)),
                None => (None, Some(ConstValue::String(size.to_string()))),
            };
            ReflectedType {
                identity: ReflectedIdentity::Anonymous,
                arguments: Vec::new(),
                semantics: ReflectedSemantics::Array {
                    element: Box::new(reflect_ty_to_reflected_type_inner(elem, seen, registry)),
                    length,
                    unresolved_length,
                },
                representation: reflected_representation(ty, registry),
            }
        }
        Ty::Opaque(name) => ReflectedType {
            identity: ReflectedIdentity::Anonymous,
            arguments: Vec::new(),
            semantics: ReflectedSemantics::Opaque {
                name: name.as_str().to_string(),
            },
            representation: reflected_representation(ty, registry),
        },
        Ty::Literal(cv) => ReflectedType {
            identity: ReflectedIdentity::Anonymous,
            arguments: Vec::new(),
            semantics: ReflectedSemantics::Literal { value: cv.clone() },
            representation: reflected_representation(ty, registry),
        },
    }
}

pub(crate) fn reflected_ty_to_const_value_with_schema(
    ty: &Ty,
    seen: &mut ReflectionRecursionState,
    registry: Option<&TypeRegistry>,
    schema: &ReflectionShapeConstructors,
) -> ConstValue {
    reflect_ty_to_reflected_type_inner(ty, seen, registry).to_const_value_with_schema(schema)
}

pub fn reflect_ty_to_reflected_type(ty: &Ty, registry: Option<&TypeRegistry>) -> ReflectedType {
    reflect_ty_to_reflected_type_inner(ty, &mut ReflectionRecursionState::default(), registry)
}

fn reflected_representation(ty: &Ty, registry: Option<&TypeRegistry>) -> ReflectedRepresentation {
    Repr::derive(ty, registry)
        .map(ReflectedRepresentation::from)
        .unwrap_or_else(|_| match ty {
            Ty::AnonymousInteger { .. } => ReflectedRepresentation::Bits { width: 0 },
            _ => ReflectedRepresentation::Product { fields: Vec::new() },
        })
}

fn recursive_reference_name(
    declaration: &DeclarationKey,
    arguments: &[ReflectionArgumentKey],
) -> String {
    let args = if arguments.is_empty() {
        String::new()
    } else {
        let rendered = arguments
            .iter()
            .map(|argument| argument.repr.as_str())
            .collect::<Vec<_>>()
            .join(",");
        format!("<{rendered}>")
    };
    format!("{}{}", declaration_key_path(declaration), args)
}

fn integer_sign_tag(
    signed: bool,
    signed_role: &ReflectionTag,
    unsigned_role: &ReflectionTag,
) -> ConstValue {
    if signed {
        tag(signed_role, vec![])
    } else {
        tag(unsigned_role, vec![])
    }
}

fn reference_permission_tag(
    value: bool,
    observe_permission: &ReflectionTag,
    mutate_permission: &ReflectionTag,
) -> ConstValue {
    tag(
        if value {
            mutate_permission
        } else {
            observe_permission
        },
        Vec::new(),
    )
}

fn tag(name: &ReflectionTag, args: Vec<ConstValue>) -> ConstValue {
    ConstValue::Tag {
        name: name.name,
        qual_path: name.qual_path.clone(),
        args: args.into(),
    }
}

fn record_named(name: &str, ty: ConstValue) -> ConstValue {
    ConstValue::Record {
        fields: vec![
            (
                Intern::new("name".to_string()),
                ConstValue::String(name.to_string()),
            ),
            (Intern::new("ty".to_string()), ty),
        ]
        .into(),
    }
}

fn record_variant(name: &str, fields: Vec<ConstValue>) -> ConstValue {
    ConstValue::Record {
        fields: vec![
            (
                Intern::new("name".to_string()),
                ConstValue::String(name.to_string()),
            ),
            (
                Intern::new("fields".to_string()),
                ConstValue::List(fields.into()),
            ),
        ]
        .into(),
    }
}

/// Store a compile-time [`ConstValue`] on a provided-trait field without building a full
/// mirrored [`Expr`] tree (that duplicates the whole shape and can use gigabytes for `Type`).
pub fn const_value_for_provided_trait_field(cv: ConstValue, span_id: SpanId) -> Typed<Expr> {
    Typed {
        value: Expr::Lit(Literal::Number(0)),
        ty: ast::ty_state::TyState::Infer,
        const_value: Some(cv),
        span_id,
    }
}

/// Turn a [`ConstValue`] into a [`Typed<Expr>`] for storage in [`ProvidedTrait`] fields.
pub fn const_value_to_typed_expr(cv: &ConstValue, span_id: SpanId) -> Typed<Expr> {
    Typed {
        value: const_value_to_expr(cv),
        ty: ast::ty_state::TyState::Infer,
        const_value: Some(cv.clone()),
        span_id,
    }
}

pub fn const_value_to_expr(cv: &ConstValue) -> Expr {
    match cv {
        ConstValue::ResultAlternative { label, .. } => Expr::AnonymousTag(*label),
        ConstValue::String(s) => Expr::Lit(Literal::String(s.clone())),
        ConstValue::Int(n) => Expr::Lit(Literal::Int(*n)),
        ConstValue::Float(HashFloat(f)) => Expr::Lit(Literal::Float(HashFloat(*f))),
        ConstValue::Tag { name, args, .. } if args.is_empty() => Expr::AnonymousTag(*name),
        ConstValue::Tag {
            name,
            qual_path,
            args,
        } => Expr::TagCall(TagCall {
            name: *name,
            qual_path: qual_path.as_ref().map(|s| {
                let parts: Vec<_> = s.split('.').map(Intern::from_ref).collect();
                let root = parts[0];
                let segments = parts.get(1..).unwrap_or(&[]).to_vec();
                ast::Spanned::new(ast::ModPath::new(root, segments), SpanId::INVALID)
            }),
            args: args
                .iter()
                .map(|a| const_value_to_typed_expr(a, SpanId::INVALID))
                .collect(),
        }),
        ConstValue::Record { fields } => {
            let args: Vec<Typed<Expr>> = fields
                .iter()
                .map(|(n, v)| {
                    let mut b = ast::Bind::new(*n, SpanId::INVALID, ast::BindValue::Unassigned);
                    b.value = ast::BindValue::Expr(Box::new(const_value_to_typed_expr(
                        v,
                        SpanId::INVALID,
                    )));
                    Typed {
                        value: Expr::Bind(Box::new(b)),
                        ty: ast::ty_state::TyState::Infer,
                        const_value: Some(v.clone()),
                        span_id: SpanId::INVALID,
                    }
                })
                .collect();
            Expr::TagCall(TagCall {
                name: Intern::from_ref("Record"),
                qual_path: None,
                args,
            })
        }
        ConstValue::List(items) => Expr::List(
            items
                .iter()
                .map(|i| const_value_to_typed_expr(i, SpanId::INVALID))
                .collect(),
        ),
    }
}
#[cfg(test)]
#[path = "../tests/reflect_tests.rs"]
mod tests;
