//! Compile-time constant values.
//!
//! These are pure value/data types (no analysis logic). They live in `ast` so that
//! `typecheck::ty::Ty::Literal` can reference them without creating a circular dependency
//! within `typecheck`.

use crate::{HashFloat, NormalExpr};
use i256::I256;
use internment::Intern;
use std::borrow::Cow;
use std::sync::Arc;

/// A compile-time known constant value tracked through flow analysis.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ConstValue {
    Proposition(bool),
    Int(I256),
    Domain(Arc<crate::integer::IntegerDomain>),
    Float(HashFloat),
    Text(String),
    Tag {
        name: Intern<String>,
        qual_path: Option<String>,
        args: Arc<[ConstValue]>,
    },
    ResultAlternative {
        owner: crate::ResultFamilyOwner,
        label: Intern<String>,
    },
    Record {
        fields: Arc<[(Intern<String>, ConstValue)]>,
    },
    ResultRow(Arc<[ConstValue]>),
    FixedArray(Arc<[ConstValue]>),
    Sequence(Arc<[ConstValue]>),
    SequenceRepeat {
        seed: Box<ConstValue>,
        count: Box<NormalExpr>,
    },
}

impl ConstValue {
    pub fn sequence(
        values: Vec<Self>,
        names: impl IntoIterator<Item = Option<Intern<String>>>,
    ) -> Self {
        let names = names.into_iter().collect::<Option<Vec<_>>>();
        match names {
            Some(names) if names.len() == values.len() && !names.is_empty() => Self::Record {
                fields: names.into_iter().zip(values).collect::<Vec<_>>().into(),
            },
            _ => Self::ResultRow(values.into()),
        }
    }

    pub fn as_utf8(&self) -> Option<Cow<'_, str>> {
        match self {
            Self::Text(text) => Some(Cow::Borrowed(text)),
            Self::FixedArray(bytes) => {
                let bytes = bytes
                    .iter()
                    .map(|byte| u8::try_from(byte.as_const_size_int()?).ok())
                    .collect::<Option<Vec<_>>>()?;
                String::from_utf8(bytes).ok().map(Cow::Owned)
            }
            _ => None,
        }
    }

    pub const ZERO: ConstValue = ConstValue::Int(I256::from_i128(0));
    pub const ONE: ConstValue = ConstValue::Int(I256::from_i128(1));

    pub fn as_const_size_int(&self) -> Option<i128> {
        match self {
            ConstValue::Int(n)
                if *n >= I256::from_i128(i128::MIN) && *n <= I256::from_i128(i128::MAX) =>
            {
                Some(n.as_i128())
            }
            _ => None,
        }
    }

    pub fn eval_binop(&self, op: &crate::BinOp, rhs: &ConstValue) -> Option<ConstValue> {
        use crate::BinOp;
        if let Some(relation) = crate::ProofRelation::for_bin_op(op)
            && let (Self::Int(left), Self::Int(right)) = (self, rhs)
        {
            return Some(Self::Proposition(relation.holds(*left, *right)));
        }
        match (self, rhs) {
            (ConstValue::Text(a), ConstValue::Text(b)) if *op == BinOp::Add => {
                Some(ConstValue::Text(format!("{a}{b}")))
            }
            (ConstValue::Int(a), ConstValue::Int(b)) => match op {
                BinOp::Add => Some(ConstValue::Int(a.wrapping_add(*b))),
                BinOp::Subtract => Some(ConstValue::Int(a.wrapping_sub(*b))),
                BinOp::Multiply => Some(ConstValue::Int(a.wrapping_mul(*b))),
                BinOp::Divide => {
                    if *b != I256::from(0) {
                        Some(ConstValue::Int(*a / *b))
                    } else {
                        None
                    }
                }
                BinOp::Modulo => {
                    if *b != I256::from(0) {
                        Some(ConstValue::Int(*a % *b))
                    } else {
                        None
                    }
                }
                BinOp::ShiftLeft => crate::integer::to_u64(*b)
                    .and_then(|shift| u32::try_from(shift).ok())
                    .filter(|shift| *shift < 256)
                    .map(|shift| ConstValue::Int(*a << shift)),
                BinOp::ShiftRight => crate::integer::to_u64(*b)
                    .and_then(|shift| u32::try_from(shift).ok())
                    .filter(|shift| *shift < 256)
                    .map(|shift| ConstValue::Int(*a >> shift)),
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
            ConstValue::Proposition(true) => "holds".to_string(),
            ConstValue::Proposition(false) => "does not hold".to_string(),
            ConstValue::Int(i) => i.to_string(),
            ConstValue::Float(HashFloat(f)) => f.to_string(),
            ConstValue::Text(s) => format!("\"{s}\""),
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
            ConstValue::Domain(domain) => format!("domain({:?})", domain.predicate()),
            ConstValue::ResultAlternative { label, .. } => label.as_str().to_string(),
            ConstValue::Record { fields } => {
                let fields: Vec<String> = fields
                    .iter()
                    .map(|(name, val)| format!("{name}: {}", val.to_hover_string()))
                    .collect();
                format!("{{ {} }}", fields.join(", "))
            }
            ConstValue::ResultRow(items) => {
                let items: Vec<String> = items.iter().map(ConstValue::to_hover_string).collect();
                format!("{{ {} }}", items.join(", "))
            }
            ConstValue::FixedArray(items) | ConstValue::Sequence(items) => {
                let items: Vec<String> = items.iter().map(ConstValue::to_hover_string).collect();
                format!("[{}]", items.join(", "))
            }
            ConstValue::SequenceRepeat { seed, count } => {
                format!("[{}; {count}]", seed.to_hover_string())
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
            ConstValue::Proposition(_) => Intern::from_ref("proposition"),
            ConstValue::Text(s) => Intern::new(s.clone()),
            ConstValue::Int(i) => Intern::new(i.to_string()),
            ConstValue::Float(HashFloat(f)) => Intern::new(f.to_string()),
            ConstValue::Tag { name, .. } => *name,
            ConstValue::Domain(_) => Intern::from_ref("domain"),
            ConstValue::ResultAlternative { label, .. } => *label,
            ConstValue::Record { .. } => Intern::new(String::from("Record")),
            ConstValue::ResultRow(_) => Intern::new(String::from("ResultRow")),
            ConstValue::FixedArray(_) => Intern::new(String::from("FixedArray")),
            ConstValue::Sequence(_) | ConstValue::SequenceRepeat { .. } => {
                Intern::new(String::from("Sequence"))
            }
        }
    }

    /// If this is a `ConstValue::Text`, return the string content.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            ConstValue::Text(s) => Some(s.as_str()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_constants_preserve_bytes_and_reject_invalid_sequences() {
        for (bytes, expected) in [
            (vec![104, 195, 169, 0], Some("hé\0")),
            (vec![], Some("")),
            (vec![255], None),
            (vec![256], None),
            (vec![-1], None),
        ] {
            let value = ConstValue::FixedArray(
                bytes
                    .into_iter()
                    .map(|byte| ConstValue::Int(I256::from_i128(byte)))
                    .collect(),
            );
            assert_eq!(value.as_utf8().as_deref(), expected);
        }
    }
}
