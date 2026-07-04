//! Type representation and type-level operations.

use internment::Intern;
use std::collections::{HashMap, HashSet};

use crate::{ConstExpr, ConstValue, HashFloat};
use std::fmt;

/// One union variant: `(variant_name, [(field_name, field_type)])` in declaration order.
pub type UnionVariant = (Intern<String>, Vec<(Intern<String>, Box<Ty>)>);

/// A parse-level predicate on a field value, e.g. `and < n`.
/// The field value is the implicit left-hand side.
/// The right-hand side is a const expression (variable ref or literal).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PredicateExpr {
    Lt(ConstExpr),
    Gt(ConstExpr),
    Le(ConstExpr),
    Ge(ConstExpr),
    Eq(ConstExpr),
    Ne(ConstExpr),
    And(Vec<PredicateExpr>),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TyArg {
    Type(Box<Ty>),
    Const(ConstExpr),
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
    Int {
        width: u8,
        signed: bool,
        /// Known compile-time value when constant-folded, otherwise `None`.
        value: Option<i128>,
        /// Inclusive lower bound for `in N...M` / `is in N...M` types, when set.
        min: Option<i128>,
        /// Inclusive upper bound for `in N...M` / `is in N...M` types, when set.
        max: Option<i128>,
    },
    Float {
        /// Known compile-time value when constant-folded, otherwise `None`.
        value: Option<HashFloat>,
    },
    Unit,
    Record {
        name: Intern<String>,
        fields: Vec<(Intern<String>, Box<Ty>)>,
    },
    /// Unresolved / generic type — falls back to `i64` in codegen.
    Opaque(Intern<String>),
    /// Fixed-size stack-allocated array (`(T.new; N)`). The value is a `!llvm.ptr`.
    Array {
        elem: Box<Ty>,
        size: ConstExpr,
    },
    /// Raw pointer — erases `T` from layout, kept only for type checking. Maps to `!llvm.ptr`.
    Ptr {
        inner: Box<Ty>,
    },
    /// Safe reference — does not consume source. Copy semantics.
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
    },
}

impl Ty {
    pub fn is_int(&self) -> bool {
        matches!(self, Ty::Int { .. })
    }

    pub fn is_unsigned_int(&self) -> bool {
        matches!(self, Ty::Int { signed: false, .. })
    }

    pub fn is_signed_int(&self) -> bool {
        matches!(self, Ty::Int { signed: true, .. })
    }

    pub fn is_float(&self) -> bool {
        matches!(self, Ty::Float { .. })
    }

    pub fn is_ptr(&self) -> bool {
        matches!(self, Ty::Ptr { .. })
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
            variants,
            literal_values: None,
        }
    }

    pub fn union_of_literals(name: Intern<String>, values: Vec<ConstValue>) -> Self {
        let variants = values
            .iter()
            .map(|cv| (cv.display_name(), Vec::new()))
            .collect();
        Ty::Union {
            name,
            variants,
            literal_values: Some(values),
        }
    }

    pub fn is_record(&self) -> bool {
        matches!(self, Ty::Record { .. })
    }

    /// Format this type for hover display.
    pub fn format_for_hover(&self) -> String {
        match self {
            Ty::Int {
                width,
                signed,
                value,
                min,
                max,
            } => {
                if let (Some(lo), Some(hi)) = (min, max) {
                    if let Some(v) = value {
                        format!("in {lo}...{hi} (= {v})")
                    } else {
                        format!("in {lo}...{hi}")
                    }
                } else {
                    let prefix = if *signed { "i" } else { "u" };
                    if let Some(v) = value {
                        format!("{}{} = {}", prefix, width, v)
                    } else {
                        format!("{}{}", prefix, width)
                    }
                }
            }
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

    /// Build a `ConstValue` describing this type for the synthesized `Reflectable.shape` field.
    pub fn to_const_value(&self) -> ConstValue {
        let mut seen: HashSet<Intern<String>> = HashSet::new();
        self.to_const_value_inner(&mut seen)
    }

    fn to_const_value_inner(&self, seen: &mut HashSet<Intern<String>>) -> ConstValue {
        match self {
            Ty::Int { width, signed, .. } => Self::tag(
                "Primitive",
                vec![
                    ConstValue::Int(i128::from(*width)),
                    ConstValue::Tag {
                        name: Intern::from_ref(if *signed { "True" } else { "False" }),
                        qual_path: None,
                        args: vec![].into(),
                    },
                ],
            ),
            Ty::Float { .. } => Self::tag(
                "Primitive",
                vec![
                    ConstValue::Int(64),
                    ConstValue::Tag {
                        name: Intern::from_ref("True"),
                        qual_path: None,
                        args: vec![].into(),
                    },
                ],
            ),
            Ty::Unit => Self::tag("Tuple", vec![ConstValue::List(vec![].into())]),
            Ty::Record { name, fields } => {
                if !seen.insert(*name) {
                    return Self::tag(
                        "Opaque",
                        vec![ConstValue::String(name.as_str().to_string())],
                    );
                }
                let named: Vec<ConstValue> = fields
                    .iter()
                    .map(|(fname, fty)| {
                        Self::record_named(fname.as_str(), fty.to_const_value_inner(seen))
                    })
                    .collect();
                seen.remove(name);
                Self::tag(
                    "Record",
                    vec![
                        ConstValue::String(name.as_str().to_string()),
                        ConstValue::List(named.into()),
                    ],
                )
            }
            Ty::Union {
                name,
                variants,
                literal_values: Some(values),
                ..
            } if !values.is_empty() => {
                if !seen.insert(*name) {
                    return Self::tag(
                        "Opaque",
                        vec![ConstValue::String(name.as_str().to_string())],
                    );
                }
                let variant_shapes: Vec<ConstValue> = values
                    .iter()
                    .map(|cv| {
                        let vname = cv.to_hover_string();
                        Self::record_variant(&vname, vec![Self::record_named("value", cv.clone())])
                    })
                    .collect();
                seen.remove(name);
                Self::tag(
                    "Union",
                    vec![
                        ConstValue::String(name.as_str().to_string()),
                        ConstValue::List(variant_shapes.into()),
                    ],
                )
            }
            Ty::Union { name, variants, .. } => {
                if !seen.insert(*name) {
                    return Self::tag(
                        "Opaque",
                        vec![ConstValue::String(name.as_str().to_string())],
                    );
                }
                let variant_shapes: Vec<ConstValue> = variants
                    .iter()
                    .map(|(vname, vfields)| {
                        let fields: Vec<ConstValue> = vfields
                            .iter()
                            .map(|(fname, fty)| {
                                Self::record_named(fname.as_str(), fty.to_const_value_inner(seen))
                            })
                            .collect();
                        Self::record_variant(vname.as_str(), fields)
                    })
                    .collect();
                seen.remove(name);
                Self::tag(
                    "Union",
                    vec![
                        ConstValue::String(name.as_str().to_string()),
                        ConstValue::List(variant_shapes.into()),
                    ],
                )
            }
            Ty::Tuple(elems) => {
                let items: Vec<ConstValue> =
                    elems.iter().map(|t| t.to_const_value_inner(seen)).collect();
                Self::tag("Tuple", vec![ConstValue::List(items.into())])
            }
            Ty::Ptr { inner } => Self::tag("Ptr", vec![inner.to_const_value_inner(seen)]),
            Ty::Ref { inner, mutable } => Self::tag(
                "Ref",
                vec![
                    inner.to_const_value_inner(seen),
                    ConstValue::Tag {
                        name: Intern::from_ref(if *mutable { "True" } else { "False" }),
                        qual_path: None,
                        args: vec![].into(),
                    },
                ],
            ),
            Ty::Array { elem, size } => {
                let size_val = match size {
                    ConstExpr::Value(v) => v.clone(),
                    _ => ConstValue::Int(0),
                };
                Self::tag("Array", vec![elem.to_const_value_inner(seen), size_val])
            }
            Ty::Opaque(name) => Self::tag(
                "Opaque",
                vec![ConstValue::String(name.as_str().to_string())],
            ),
            Ty::Literal(cv) => Self::record_named("Literal", cv.clone()),
        }
    }

    fn tag(name: &str, args: Vec<ConstValue>) -> ConstValue {
        ConstValue::Tag {
            name: Intern::new(name.to_string()),
            qual_path: None,
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

    /// Convenience constructor for a default-width signed integer type.
    pub fn i64() -> Self {
        Ty::Int {
            width: 64,
            signed: true,
            value: None,
            min: None,
            max: None,
        }
    }

    /// Convenience constructor for an 8-bit unsigned integer type.
    pub fn u8() -> Self {
        Ty::Int {
            width: 8,
            signed: false,
            value: None,
            min: None,
            max: None,
        }
    }

    pub fn is_bounded_int(&self) -> bool {
        matches!(
            self,
            Ty::Int {
                min: Some(_),
                max: Some(_),
                ..
            }
        )
    }

    /// Extract the name of a nominal type for compile-time trait lookup.
    pub fn type_name(&self) -> Option<&Intern<String>> {
        match self {
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
            Ty::Opaque(name) => subst.get(name).cloned().unwrap_or(Ty::Opaque(*name)),
            Ty::Record { name, fields } => Ty::Record {
                name: *name,
                fields: fields
                    .iter()
                    .map(|(n, t)| (*n, Box::new(t.substitute(subst))))
                    .collect(),
            },
            Ty::Union {
                name,
                variants,
                literal_values,
            } => Ty::Union {
                name: *name,
                variants: variants
                    .iter()
                    .map(|(vn, fields)| {
                        let new_fields = fields
                            .iter()
                            .map(|(n, t)| (*n, Box::new(t.substitute(subst))))
                            .collect();
                        (*vn, new_fields)
                    })
                    .collect(),
                literal_values: literal_values.clone(),
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
            _ => self.clone(),
        }
    }

    /// If this type is an array or tuple, return the element type.
    /// Otherwise, look up the `List` type in `tag_types` and extract its `pointer` field.
    pub fn list_elem_ty(&self, tag_types: &HashMap<Intern<String>, Ty>) -> Option<Ty> {
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
