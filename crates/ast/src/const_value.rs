//! Compile-time constant values and type constraint representations.
//!
//! These are pure value/data types (no analysis logic). They live in `ast` so that
//! `typecheck::ty::Ty::Literal` can reference them without creating a circular dependency
//! within `typecheck`.

use crate::HashFloat;
use internment::Intern;
use std::sync::Arc;

/// Represents a narrowed type constraint on a variable.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypeConstraint {
    IsVariant(Intern<String>, Intern<String>),
    IsNotVariant(Intern<String>, Intern<String>),
}

impl TypeConstraint {
    pub fn contradicts(&self, other: &TypeConstraint) -> bool {
        match (self, other) {
            (TypeConstraint::IsVariant(u1, v1), TypeConstraint::IsVariant(u2, v2)) => {
                u1 == u2 && v1 != v2
            }
            (TypeConstraint::IsVariant(_, v1), TypeConstraint::IsNotVariant(_, v2)) => v1 == v2,
            (TypeConstraint::IsNotVariant(_, v1), TypeConstraint::IsVariant(_, v2)) => v1 == v2,
            _ => false,
        }
    }

    /// Negate this constraint (for after-loop / else-branch reasoning).
    pub fn negate(&self) -> Self {
        match self {
            TypeConstraint::IsVariant(u, v) => TypeConstraint::IsNotVariant(*u, *v),
            TypeConstraint::IsNotVariant(u, v) => TypeConstraint::IsVariant(*u, *v),
        }
    }

    pub fn display_suffix(&self) -> String {
        match self {
            TypeConstraint::IsVariant(_, variant) => variant.as_str().to_string(),
            TypeConstraint::IsNotVariant(union, excluded) => {
                format!("not {}.{}", union.as_str(), excluded.as_str())
            }
        }
    }
}

/// A compile-time known constant value tracked through flow analysis.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ConstValue {
    Int(i128),
    Float(HashFloat),
    String(String),
    Tag {
        name: Intern<String>,
        qual_path: Option<String>,
        args: Arc<[ConstValue]>,
    },
    Record {
        fields: Arc<[(Intern<String>, ConstValue)]>,
    },
    List(Arc<[ConstValue]>),
}

impl ConstValue {
    pub const ZERO: ConstValue = ConstValue::Int(0);
    pub const ONE: ConstValue = ConstValue::Int(1);

    pub fn as_const_size_int(&self) -> Option<i128> {
        match self {
            ConstValue::Int(n) => Some(*n),
            ConstValue::Tag { name, args, .. } if name.as_str() == "Const" => {
                args.first()?.as_const_size_int()
            }
            _ => None,
        }
    }

    pub fn const_size_tag(n: i128) -> ConstValue {
        ConstValue::Tag {
            name: Intern::from_ref("Const"),
            qual_path: None,
            args: vec![ConstValue::Int(n)].into(),
        }
    }

    pub fn dynamic_size_tag() -> ConstValue {
        ConstValue::Tag {
            name: Intern::from_ref("Dynamic"),
            qual_path: None,
            args: vec![].into(),
        }
    }

    pub fn eval_binop(&self, op: &crate::BinOp, rhs: &ConstValue) -> Option<ConstValue> {
        use crate::BinOp;
        // Only wrap in `Const` tag when at least one operand already is a `Const`
        // size tag. Plain `Int` values should stay as `Int` (unwrapped).
        let is_const_size = matches!(
            (self, rhs),
            (
                ConstValue::Tag { name, .. },
                ConstValue::Tag { name: rn, .. }
            ) if name.as_str() == "Const" || rn.as_str() == "Const"
        ) || matches!(self, ConstValue::Tag { name, .. } if name.as_str() == "Const")
            || matches!(rhs, ConstValue::Tag { name, .. } if name.as_str() == "Const");
        if is_const_size
            && let (Some(a), Some(b)) = (self.as_const_size_int(), rhs.as_const_size_int())
            && let Some(cv) = match op {
                BinOp::Add => Some(ConstValue::const_size_tag(a.wrapping_add(b))),
                BinOp::Subtract => Some(ConstValue::const_size_tag(a.wrapping_sub(b))),
                BinOp::Multiply => Some(ConstValue::const_size_tag(a.wrapping_mul(b))),
                BinOp::Divide if b != 0 => Some(ConstValue::const_size_tag(a / b)),
                _ => None,
            }
        {
            return Some(cv);
        }
        if matches!(
            (self, rhs),
            (
                ConstValue::Tag { name, .. },
                ConstValue::Tag { name: rhs_name, .. }
            ) if name.as_str() == "Dynamic" || rhs_name.as_str() == "Dynamic"
        ) {
            return Some(ConstValue::dynamic_size_tag());
        }
        match (self, rhs) {
            (ConstValue::Tag { name: lhs_name, .. }, ConstValue::Tag { name: rhs_name, .. })
                if matches!(lhs_name.as_str(), "True" | "False")
                    && matches!(rhs_name.as_str(), "True" | "False") =>
            {
                let lhs = lhs_name.as_str() == "True";
                let rhs = rhs_name.as_str() == "True";
                let result = match op {
                    BinOp::LogicalAnd => Some(lhs && rhs),
                    BinOp::LogicalOr => Some(lhs || rhs),
                    _ => None,
                }?;
                Some(ConstValue::Tag {
                    name: Intern::from_ref(if result { "True" } else { "False" }),
                    qual_path: None,
                    args: vec![].into(),
                })
            }
            (ConstValue::Int(a), ConstValue::Int(b)) => match op {
                BinOp::Add => Some(ConstValue::Int(a.wrapping_add(*b))),
                BinOp::Subtract => Some(ConstValue::Int(a.wrapping_sub(*b))),
                BinOp::Multiply => Some(ConstValue::Int(a.wrapping_mul(*b))),
                BinOp::Divide => {
                    if *b != 0 {
                        Some(ConstValue::Int(a / *b))
                    } else {
                        None
                    }
                }
                BinOp::Modulo => {
                    if *b != 0 {
                        Some(ConstValue::Int(a % *b))
                    } else {
                        None
                    }
                }
                _ => None,
            },
            (ConstValue::Float(HashFloat(a)), ConstValue::Float(HashFloat(b))) => match op {
                BinOp::Add => Some(ConstValue::Float(HashFloat(a + b))),
                BinOp::Subtract => Some(ConstValue::Float(HashFloat(a - b))),
                BinOp::Multiply => Some(ConstValue::Float(HashFloat(a * b))),
                BinOp::Divide => Some(ConstValue::Float(HashFloat(a / b))),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn to_hover_string(&self) -> String {
        match self {
            ConstValue::Int(i) => i.to_string(),
            ConstValue::Float(HashFloat(f)) => f.to_string(),
            ConstValue::String(s) => format!("\"{s}\""),
            ConstValue::Tag {
                name,
                qual_path,
                args,
            } => {
                let tag_name = if let Some(qp) = qual_path {
                    format!("{qp}.{}", name.as_str())
                } else {
                    name.as_str().to_string()
                };
                if args.is_empty() {
                    tag_name
                } else {
                    let args: Vec<String> = args.iter().map(|a| a.to_hover_string()).collect();
                    format!("{tag_name}({})", args.join(", "))
                }
            }
            ConstValue::Record { fields } => {
                let fields: Vec<String> = fields
                    .iter()
                    .map(|(name, val)| format!("{name}: {}", val.to_hover_string()))
                    .collect();
                format!("{{ {} }}", fields.join(", "))
            }
            ConstValue::List(items) => {
                let items: Vec<String> = items.iter().map(|i| i.to_hover_string()).collect();
                format!("[{}]", items.join(", "))
            }
        }
    }

    /// Get a named field from a `ConstValue::Record`.
    pub fn get_field(&self, name: &str) -> Option<&ConstValue> {
        match self {
            ConstValue::Record { fields } => fields
                .iter()
                .find(|(n, _)| n.as_str() == name)
                .map(|(_, v)| v),
            _ => None,
        }
    }

    /// Return a display name for this constant value (used for variant labels).
    pub fn display_name(&self) -> Intern<String> {
        match self {
            ConstValue::String(s) => Intern::new(s.clone()),
            ConstValue::Int(i) => Intern::new(i.to_string()),
            ConstValue::Float(HashFloat(f)) => Intern::new(f.to_string()),
            ConstValue::Tag { name, .. } => *name,
            ConstValue::Record { .. } => Intern::new(String::from("Record")),
            ConstValue::List(_) => Intern::new(String::from("List")),
        }
    }

    /// If this is a `ConstValue::List`, return a reference to the items.
    pub fn as_list(&self) -> Option<&[ConstValue]> {
        match self {
            ConstValue::List(items) => Some(items),
            _ => None,
        }
    }

    /// If this is a `ConstValue::String`, return the string content.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            ConstValue::String(s) => Some(s.as_str()),
            _ => None,
        }
    }
}
