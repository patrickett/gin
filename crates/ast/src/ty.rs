//! Type representation and type-level operations.

use internment::Intern;
use std::collections::HashMap;

use crate::{ConstValue, HashFloat, NormalExpr};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeId {
    pub file: u32,
    pub name: Intern<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PackageSourceKey {
    Workspace,
    Registry {
        registry: Intern<String>,
    },
    Git {
        url: Intern<String>,
        revision: Intern<String>,
    },
    Path {
        parent_instance: Intern<String>,
        relative_path: Intern<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PackageInstanceKey {
    pub name: Intern<String>,
    pub version: Intern<String>,
    pub source: PackageSourceKey,
    pub instance: Intern<String>,
}

impl PackageInstanceKey {
    pub fn workspace(name: &str, version: &str) -> Self {
        Self {
            name: Intern::new(name.to_string()),
            version: Intern::new(version.to_string()),
            source: PackageSourceKey::Workspace,
            instance: Intern::from_ref("root"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeclarationKey {
    pub package: PackageInstanceKey,
    pub module: Vec<Intern<String>>,
    pub declaration: Intern<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResultFamilyOwner {
    Callable(DeclarationKey),
    LocalCallable(crate::BinderId),
    Structural(crate::OperatorRole),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResultAlternative {
    pub label: Intern<String>,
    pub proposition: Option<ProofProposition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProofTerm {
    Value(i256::I256),
    Name(Intern<String>),
    Add(Box<Self>, Box<Self>),
    Sub(Box<Self>, Box<Self>),
    Mul(Box<Self>, Box<Self>),
    Remainder(Box<Self>, Box<Self>),
    PowerOfTwo(Box<Self>),
    TargetQuery {
        kind: crate::TargetQueryKind,
        operand: crate::TypeReference,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProofRelation {
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProofProposition {
    Compare {
        left: ProofTerm,
        relation: ProofRelation,
        right: ProofTerm,
    },
    InRange {
        value: ProofTerm,
        start: ProofTerm,
        end: ProofTerm,
    },
    Not(Box<Self>),
    And(Box<Self>, Box<Self>),
    Or(Box<Self>, Box<Self>),
}

impl fmt::Display for ProofTerm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value(value) => write!(f, "{value}"),
            Self::Name(name) => write!(f, "{}", name.as_str()),
            Self::Add(left, right) => write!(f, "({left} + {right})"),
            Self::Sub(left, right) => write!(f, "({left} - {right})"),
            Self::Mul(left, right) => write!(f, "({left} * {right})"),
            Self::Remainder(left, right) => write!(f, "({left} % {right})"),
            Self::PowerOfTwo(inner) => write!(f, "PowerOfTwo({inner})"),
            Self::TargetQuery { kind, operand } => {
                write!(f, "#{kind}({operand})")
            }
        }
    }
}

impl fmt::Display for ProofRelation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Equal => "=",
            Self::NotEqual => "!=",
            Self::Less => "<",
            Self::LessOrEqual => "<=",
            Self::Greater => ">",
            Self::GreaterOrEqual => ">=",
        })
    }
}

impl fmt::Display for ProofProposition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Compare {
                left,
                relation,
                right,
            } => write!(f, "{left} {relation} {right}"),
            Self::InRange { value, start, end } => write!(f, "{value} in {start}...{end}"),
            Self::Not(inner) => write!(f, "not ({inner})"),
            Self::And(left, right) => write!(f, "({left}) and ({right})"),
            Self::Or(left, right) => write!(f, "({left}) or ({right})"),
        }
    }
}

impl DeclarationKey {
    pub fn new(
        package: PackageInstanceKey,
        module: Vec<Intern<String>>,
        declaration: Intern<String>,
    ) -> Self {
        Self {
            package,
            module,
            declaration,
        }
    }

    pub fn from_qualified_name(
        package: PackageInstanceKey,
        qualified_name: Intern<String>,
    ) -> Self {
        let mut parts: Vec<_> = qualified_name
            .as_str()
            .split('.')
            .map(|part| Intern::new(part.to_string()))
            .collect();
        let declaration = parts.pop().unwrap_or(qualified_name);
        if parts.first() == Some(&package.name) {
            parts.remove(0);
        }
        Self::new(package, parts, declaration)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NamedTypeInstance {
    pub declaration: TypeId,
    pub arguments: Vec<(Intern<String>, TyArg)>,
}

impl NamedTypeInstance {
    pub fn new(declaration: TypeId) -> Self {
        Self {
            declaration,
            arguments: Vec::new(),
        }
    }

    pub fn with_arguments(mut self, arguments: Vec<(Intern<String>, TyArg)>) -> Self {
        self.arguments = arguments;
        self
    }
}

pub use crate::integer::IntegerInterpretation;

/// One union variant with an optional indexed result type.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UnionVariant {
    pub name: Intern<String>,
    pub fields: Vec<(Intern<String>, Box<Ty>)>,
    /// Result type with const-generic indices, e.g. `Vec(x, 0)` for `Nil`.
    /// `None` means the default — same as the parent union's type parameters.
    pub result_ty: Option<Ty>,
}

impl UnionVariant {
    pub fn new(name: Intern<String>, fields: Vec<(Intern<String>, Box<Ty>)>) -> Self {
        Self {
            name,
            fields,
            result_ty: None,
        }
    }
}

/// A parse-level predicate on a field value, e.g. `and < n`.
/// The field value is the implicit left-hand side.
/// The right-hand side is a const expression (variable ref or literal).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PredicateExpr {
    Lt(NormalExpr),
    Gt(NormalExpr),
    Le(NormalExpr),
    Ge(NormalExpr),
    Eq(NormalExpr),
    Ne(NormalExpr),
    And(Vec<PredicateExpr>),
    Proposition(Box<ProofProposition>),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TyArg {
    Type(Box<Ty>),
    Const(NormalExpr),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LiteralKind {
    Integer,
}

impl LiteralKind {
    pub fn resolve(name: &str) -> Option<Self> {
        match name {
            "IntegerLiteral" => Some(Self::Integer),
            _ => None,
        }
    }
}

impl fmt::Display for TyArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TyArg::Type(ty) => write!(f, "{}", ty.format_for_hover()),
            TyArg::Const(expr) => write!(f, "{expr}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ParamKind {
    Type,
    Value(Box<Ty>),
}

impl fmt::Display for ParamKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParamKind::Type => write!(f, "type"),
            ParamKind::Value(ty) => write!(f, "{}", ty.format_for_hover()),
        }
    }
}

/// Resolved type — the canonical representation after resolving declared type names against declarations.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Ty {
    Named {
        instance: NamedTypeInstance,
        name: Intern<String>,
    },
    AnonymousInteger {
        validity: crate::integer::IntegerValidity,
    },
    ResultFamily {
        owner: ResultFamilyOwner,
        alternatives: Vec<ResultAlternative>,
    },
    UnresolvedLiteral(LiteralKind),
    Float {
        /// Known compile-time value when constant-folded, otherwise `None`.
        value: Option<HashFloat>,
    },
    Unit,
    Record {
        name: Intern<String>,
        fields: Vec<(Intern<String>, Box<Ty>)>,
        resolved_params: Option<Vec<(Intern<String>, TyArg)>>,
    },
    /// Unresolved / generic type — falls back to `i64` in codegen.
    Opaque(Intern<String>),
    /// Fixed-size stack-allocated array (`(T.new; N)`). The value is a `!llvm.ptr`.
    Array {
        elem: Box<Ty>,
        size: NormalExpr,
    },
    /// Raw pointer — erases `T` from layout, kept only for type checking. Maps to `!llvm.ptr`.
    Ptr {
        inner: Box<Ty>,
    },
    /// Address-taking before an expected named raw-pointer type is selected.
    Address {
        pointee: Box<Ty>,
        address_space: u32,
    },
    /// Safe reference — does not consume its referent.
    /// `ref T` is immutable, `mut T` is mutable.
    /// Invalidation is tracked in flow analysis.
    Ref {
        inner: Box<Ty>,
        mutable: bool,
    },
    /// Positional tuple VALUE — used for tuple literals `(e1, e2, …)`. Maps to an LLVM struct.
    Tuple(Vec<Ty>),
    /// A single compile-time-known literal (`'John'`, `42`, …) from a site bind.
    Literal(ConstValue),
    /// Tagged union. When [`Self::literal_values`] is set, every variant is a unit literal
    /// (`LogLevel is 'debug' | 'info'`) and runtime layout is a small discriminant.
    Union {
        name: Intern<String>,
        variants: Vec<UnionVariant>,
        literal_values: Option<Vec<ConstValue>>,
        /// When this union was resolved from a const-generic type application
        /// (e.g. `Vec(Int, 3)`), stores the concrete type/const args as
        /// `(param_name, TyArg)` pairs, preserving declaration order.
        resolved_params: Option<Vec<(Intern<String>, TyArg)>>,
    },
}

impl Ty {
    pub fn type_id(&self) -> Option<TypeId> {
        match self {
            Ty::Named { instance, .. } => Some(instance.declaration),
            _ => None,
        }
    }

    pub fn named_instance(&self) -> Option<&NamedTypeInstance> {
        match self {
            Ty::Named { instance, .. } => Some(instance),
            _ => None,
        }
    }

    /// Return the nominal instance for a type that may be wrapped in
    /// compatibility adapters (`ref`, `mut`, and bounded refinements).
    pub fn named_instance_stripping_reference_wrappers(&self) -> Option<&NamedTypeInstance> {
        match self {
            Ty::Ref { inner, .. } => inner.named_instance_stripping_reference_wrappers(),
            Ty::Named { instance, .. } => Some(instance),
            _ => None,
        }
    }

    /// Return the referent for a type that may be wrapped in compatibility
    /// adapters (`ref`, `mut`, and bounded refinements). This is used by
    /// registry-adapter APIs that should ignore those wrappers when resolving
    /// nominal identity.
    pub fn without_reference_wrappers(&self) -> &Ty {
        match self {
            Ty::Ref { inner, .. } => inner.without_reference_wrappers(),
            ty => ty,
        }
    }

    pub fn is_int(&self) -> bool {
        matches!(self, Ty::AnonymousInteger { .. })
    }

    pub fn anonymous_integer_validity(&self) -> Option<&crate::integer::IntegerValidity> {
        match self {
            Ty::AnonymousInteger { validity } => Some(validity),
            _ => None,
        }
    }

    pub fn anonymous_operation_interpretation(&self) -> Option<IntegerInterpretation> {
        self.anonymous_integer_validity().map(|validity| {
            if validity
                .domain()
                .storage_hull()
                .is_some_and(|hull| hull.min().is_negative())
            {
                IntegerInterpretation::Signed
            } else {
                IntegerInterpretation::Unsigned
            }
        })
    }

    pub fn anonymous_integer_width(&self) -> Option<u8> {
        match self {
            Ty::AnonymousInteger { validity } => crate::integer::resolve_representation(
                validity,
                &crate::integer::IntegerRepresentationRule::InferFromValidity,
            )
            .ok()
            .and_then(|representation| u8::try_from(representation.width().get()).ok()),
            _ => None,
        }
    }

    pub fn is_float(&self) -> bool {
        matches!(self, Ty::Float { .. })
    }

    pub fn is_ptr(&self) -> bool {
        self.address_space().is_some()
    }

    pub fn address_space(&self) -> Option<u32> {
        match self {
            Ty::Ptr { .. } => Some(0),
            Ty::Address { address_space, .. } => Some(*address_space),
            _ => None,
        }
    }

    pub fn pointee_ty(&self) -> Option<&Ty> {
        match self {
            Ty::Ptr { inner } | Ty::Address { pointee: inner, .. } => Some(inner),
            _ => None,
        }
    }

    pub fn is_unresolved_address(&self) -> bool {
        matches!(self, Ty::Address { .. })
    }

    pub fn is_ref(&self) -> bool {
        matches!(self, Ty::Ref { .. })
    }

    pub fn is_mutable_ref(&self) -> bool {
        matches!(self, Ty::Ref { mutable: true, .. })
    }

    pub fn is_union(&self) -> bool {
        matches!(self, Ty::Union { .. })
    }

    /// Union declared as `'a' | 'b'` or a site [`Ty::Literal`].
    pub fn is_literal_shape(&self) -> bool {
        matches!(self, Ty::Literal(_))
            || matches!(
                self,
                Ty::Union {
                    literal_values: Some(_),
                    ..
                }
            )
    }

    /// Discriminant-order literal members for a literal-only union.
    pub fn union_literal_values(&self) -> Option<&[ConstValue]> {
        match self {
            Ty::Union {
                literal_values: Some(v),
                ..
            } => Some(v.as_slice()),
            _ => None,
        }
    }

    pub fn as_literal_const(&self) -> Option<&ConstValue> {
        match self {
            Ty::Literal(v) => Some(v),
            _ => None,
        }
    }

    pub fn union_named(name: Intern<String>, variants: Vec<UnionVariant>) -> Self {
        Ty::Union {
            name,
            resolved_params: None,
            variants,
            literal_values: None,
        }
    }

    pub fn union_of_literals(name: Intern<String>, values: Vec<ConstValue>) -> Self {
        let variants = values
            .iter()
            .map(|cv| UnionVariant::new(cv.display_name(), Vec::new()))
            .collect();
        Ty::Union {
            name,
            variants,
            literal_values: Some(values),
            resolved_params: None,
        }
    }

    pub fn is_record(&self) -> bool {
        matches!(self, Ty::Record { .. })
    }

    /// Format this type for hover display.
    pub fn format_for_hover(&self) -> String {
        match self {
            Ty::Named { instance, name, .. } => {
                if instance.arguments.is_empty() {
                    name.as_str().to_string()
                } else {
                    let arguments = instance
                        .arguments
                        .iter()
                        .map(|(_, argument)| argument.to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("{}({arguments})", name.as_str())
                }
            }
            Ty::AnonymousInteger { validity, .. } => validity
                .domain()
                .storage_hull()
                .map(|hull| format!("integer in {}...{}", hull.min(), hull.max()))
                .unwrap_or_else(|| "integer".to_string()),
            Ty::ResultFamily {
                owner,
                alternatives,
            } => format!(
                "result {:?}({})",
                owner,
                alternatives
                    .iter()
                    .map(|alternative| alternative.label.as_str())
                    .collect::<Vec<_>>()
                    .join(" or ")
            ),
            Ty::UnresolvedLiteral(LiteralKind::Integer) => "integer literal".to_string(),
            Ty::Float { value } => {
                if let Some(HashFloat(v)) = value {
                    format!("f64 = {}", v)
                } else {
                    "f64".to_string()
                }
            }
            Ty::Unit => "()".to_string(),
            Ty::Record { fields, .. } => {
                let parts: Vec<String> = fields
                    .iter()
                    .map(|(fname, fty)| format!("{}: {}", fname.as_str(), fty.format_for_hover()))
                    .collect();
                parts.join(", ")
            }
            Ty::Union { name, .. } => format!("Union({})", name.as_str()),
            Ty::Opaque(name) => name.as_str().to_string(),
            Ty::Array { elem, size } => format!("[{}; {}]", elem.format_for_hover(), size),
            Ty::Ptr { inner } => format!("*{}", inner.format_for_hover()),
            Ty::Address {
                pointee,
                address_space,
            } => format!(
                "unresolved address(space {address_space}, {})",
                pointee.format_for_hover()
            ),
            Ty::Ref { inner, mutable } => {
                let prefix = if *mutable { "mut " } else { "ref " };
                format!("{}{}", prefix, inner.format_for_hover())
            }
            Ty::Tuple(tys) => {
                let parts: Vec<String> = tys.iter().map(|t| t.format_for_hover()).collect();
                format!("({})", parts.join(", "))
            }
            Ty::Literal(cv) => match cv {
                ConstValue::String(s) => format!("'{}'", s),
                other => other.to_hover_string(),
            },
        }
    }

    /// Convenience constructor for a default-width signed integer type.
    pub fn i64() -> Self {
        Self::anonymous_integer_for_width(64, true)
    }

    /// Convenience constructor for an 8-bit unsigned integer type.
    pub fn u8() -> Self {
        Self::anonymous_integer_for_width(8, false)
    }

    pub fn anonymous_integer_for_width(width: u8, signed: bool) -> Self {
        let domain = if width == 0 || width > 128 {
            None
        } else if signed {
            let magnitude = i256::I256::from(1) << (u32::from(width) - 1);
            crate::integer::IntegerDomain::bounded(-magnitude, magnitude - i256::I256::from(1))
        } else {
            let upper = (i256::I256::from(1) << u32::from(width)) - i256::I256::from(1);
            crate::integer::IntegerDomain::bounded(i256::I256::from(0), upper)
        }
        .unwrap_or_else(crate::integer::IntegerDomain::empty);
        Ty::AnonymousInteger {
            validity: crate::integer::IntegerValidity::new(domain),
        }
    }

    pub fn is_bounded_int(&self) -> bool {
        self.anonymous_integer_validity().is_some()
    }

    /// Extract the name of a nominal type for compile-time trait lookup.
    pub fn type_name(&self) -> Option<&Intern<String>> {
        match self {
            Ty::Named { name, .. } => Some(name),
            Ty::Record { name, .. } => Some(name),
            Ty::Union { name, .. } => Some(name),
            Ty::Opaque(name) => Some(name),
            _ => None,
        }
    }

    /// Substitute type variables, replacing only [`Ty::Opaque`] leaves that match an
    /// entry in `subst`.
    pub fn substitute(&self, subst: &HashMap<Intern<String>, Ty>) -> Ty {
        match self {
            Ty::Named { instance, name } => Ty::Named {
                instance: NamedTypeInstance {
                    declaration: instance.declaration,
                    arguments: instance
                        .arguments
                        .iter()
                        .map(|(name, arg)| {
                            let arg = match arg {
                                TyArg::Type(ty) => TyArg::Type(Box::new(ty.substitute(subst))),
                                TyArg::Const(expr) => TyArg::Const(expr.clone()),
                            };
                            (*name, arg)
                        })
                        .collect(),
                },
                name: *name,
            },
            Ty::AnonymousInteger { validity } => Ty::AnonymousInteger {
                validity: validity.clone(),
            },
            Ty::Ref { inner, mutable } => Ty::Ref {
                inner: Box::new(inner.substitute(subst)),
                mutable: *mutable,
            },
            Ty::Opaque(name) => subst.get(name).cloned().unwrap_or(Ty::Opaque(*name)),
            Ty::Record {
                name,
                fields,
                resolved_params,
            } => Ty::Record {
                name: *name,
                fields: fields
                    .iter()
                    .map(|(n, t)| (*n, Box::new(t.substitute(subst))))
                    .collect(),
                resolved_params: resolved_params.as_ref().map(|params| {
                    params
                        .iter()
                        .map(|(name, arg)| {
                            let arg = match arg {
                                TyArg::Type(ty) => TyArg::Type(Box::new(ty.substitute(subst))),
                                TyArg::Const(expr) => TyArg::Const(expr.clone()),
                            };
                            (*name, arg)
                        })
                        .collect()
                }),
            },
            Ty::Union {
                name,
                variants,
                literal_values,
                resolved_params,
            } => Ty::Union {
                name: *name,
                variants: variants
                    .iter()
                    .map(|v| {
                        let new_fields = v
                            .fields
                            .iter()
                            .map(|(n, t)| (*n, Box::new(t.substitute(subst))))
                            .collect();
                        UnionVariant {
                            name: v.name,
                            fields: new_fields,
                            result_ty: v.result_ty.as_ref().map(|rt| rt.substitute(subst)),
                        }
                    })
                    .collect(),
                literal_values: literal_values.clone(),
                resolved_params: resolved_params.clone(),
            },
            Ty::Literal(v) => Ty::Literal(v.clone()),
            Ty::Tuple(elems) => Ty::Tuple(elems.iter().map(|t| t.substitute(subst)).collect()),
            Ty::Array { elem, size } => Ty::Array {
                elem: Box::new(elem.substitute(subst)),
                size: size.clone(),
            },
            Ty::Ptr { inner } => Ty::Ptr {
                inner: Box::new(inner.substitute(subst)),
            },
            Ty::Address {
                pointee,
                address_space,
            } => Ty::Address {
                pointee: Box::new(pointee.substitute(subst)),
                address_space: *address_space,
            },
            _ => self.clone(),
        }
    }

    /// If this type is an array or tuple, return the element type.
    /// Otherwise, look up the `List` type in `tag_types` and extract its `pointer` field.
    pub fn list_elem_ty(&self, tag_types: &HashMap<Intern<String>, Ty>) -> Option<Ty> {
        if let Some(instance) = self.named_instance()
            && let Some((_, TyArg::Type(elem))) = instance
                .arguments
                .iter()
                .find(|(name, _)| name.as_str() == "x")
        {
            return Some((**elem).clone());
        }
        match self {
            Ty::Array { elem, .. } => Some(*elem.clone()),
            Ty::Tuple(elems) if !elems.is_empty() => Some(elems[0].clone()),
            _ => tag_types
                .get(&Intern::from_ref("List"))
                .and_then(|list_ty| {
                    if let Ty::Record { fields, .. } = list_ty {
                        fields
                            .iter()
                            .find(|(n, _)| n.as_str() == "pointer")
                            .map(|(_, t)| match t.as_ref() {
                                Ty::Ptr { inner } => inner.as_ref().clone(),
                                other => other.clone(),
                            })
                    } else {
                        None
                    }
                }),
        }
    }
}

/// Variant map entry: (union_name, discriminant, [(field_name, field_type)]).
pub type VariantMapEntry = (Intern<String>, usize, Vec<(Intern<String>, Ty)>);

/// Map: variant_name → entries (for lookup by variant name across files).
pub type VariantMap = HashMap<Intern<String>, Vec<VariantMapEntry>>;

/// Variant lookup result: (union_name, discriminant, &[(field_name, field_type)]).
pub type VariantLookupResult<'a> = (Intern<String>, usize, &'a [(Intern<String>, Ty)]);
