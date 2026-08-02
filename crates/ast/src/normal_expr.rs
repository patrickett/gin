//! Symbolic compile-time constant expressions.
//!
//! `NormalExpr` represents unevaluated constant expressions (e.g. `n + m`, `2 * n`)
//! in contrast with [`ConstValue`](crate::ConstValue) which holds already-evaluated constants.

use crate::ConstValue;
use internment::Intern;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinderOwner {
    Definition(Intern<String>),
    Expression(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BinderId {
    pub file: u32,
    pub owner: BinderOwner,
}

impl BinderId {
    pub fn new(file: u32, owner: BinderOwner) -> Self {
        Self { file, owner }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DependentArgId {
    pub binder: BinderId,
    pub slot: u32,
}

impl DependentArgId {
    pub fn new(binder: BinderId, slot: u32) -> Self {
        Self { binder, slot }
    }
}

/// A symbolic compile-time constant expression — unevaluated, in contrast with
/// [`ConstValue`]. Evaluation/normalization is handled by a separate pass.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NormalExpr {
    Value(ConstValue),
    Var(Intern<String>),
    Inferred(DependentArgId),
    Add(Box<NormalExpr>, Box<NormalExpr>),
    Sub(Box<NormalExpr>, Box<NormalExpr>),
    Mul(Box<NormalExpr>, Box<NormalExpr>),
}

impl NormalExpr {
    pub fn is_zero(&self) -> bool {
        *self == ConstValue::ZERO
    }

    pub fn is_one(&self) -> bool {
        *self == ConstValue::ONE
    }

    pub fn as_const_int(&self) -> Option<i128> {
        match self {
            NormalExpr::Value(ConstValue::Int(n)) => Some(*n),
            _ => None,
        }
    }
}

impl From<i128> for NormalExpr {
    fn from(n: i128) -> Self {
        NormalExpr::Value(ConstValue::Int(n))
    }
}

impl TryFrom<&NormalExpr> for usize {
    type Error = ();

    fn try_from(expr: &NormalExpr) -> Result<usize, ()> {
        expr.as_const_int()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or(())
    }
}

impl PartialEq<ConstValue> for NormalExpr {
    fn eq(&self, other: &ConstValue) -> bool {
        matches!(self, NormalExpr::Value(v) if v == other)
    }
}

impl fmt::Display for NormalExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NormalExpr::Value(v) => match v {
                ConstValue::Int(i) => write!(f, "{i}"),
                ConstValue::Float(hf) => write!(f, "{hf}"),
                ConstValue::String(s) => write!(f, "\"{s}\""),
                ConstValue::Tag {
                    name,
                    qual_path,
                    args,
                } => {
                    if let Some(qp) = qual_path {
                        write!(f, "{qp}.{}", name.as_str())?;
                    } else {
                        write!(f, "{}", name.as_str())?;
                    }
                    if !args.is_empty() {
                        write!(f, "(")?;
                        for (i, arg) in args.iter().enumerate() {
                            if i > 0 {
                                write!(f, ", ")?;
                            }
                            write!(f, "{}", NormalExpr::Value(arg.clone()))?;
                        }
                        write!(f, ")")?;
                    }
                    Ok(())
                }
                ConstValue::Record { fields } => {
                    write!(f, "{{ ")?;
                    for (i, (name, val)) in fields.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}: {}", name.as_str(), NormalExpr::Value(val.clone()))?;
                    }
                    write!(f, " }}")
                }
                ConstValue::List(items) => {
                    write!(f, "[")?;
                    for (i, item) in items.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", NormalExpr::Value(item.clone()))?;
                    }
                    write!(f, "]")
                }
            },
            NormalExpr::Var(name) => write!(f, "{}", name.as_str()),
            NormalExpr::Inferred(_) => write!(f, "?"),
            NormalExpr::Add(l, r) => write!(f, "({l} + {r})"),
            NormalExpr::Sub(l, r) => write!(f, "({l} - {r})"),
            NormalExpr::Mul(l, r) => write!(f, "({l} * {r})"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ConstValue;

    #[test]
    fn value_int_displays_as_number() {
        let expr = NormalExpr::Value(ConstValue::Int(42));
        assert_eq!(expr.to_string(), "42");
    }

    #[test]
    fn var_displays_as_name() {
        let expr = NormalExpr::Var(Intern::from_ref("n"));
        assert_eq!(expr.to_string(), "n");
    }

    #[test]
    fn add_displays_parens() {
        let expr = NormalExpr::Add(
            Box::new(NormalExpr::Var(Intern::from_ref("n"))),
            Box::new(NormalExpr::Value(ConstValue::Int(1))),
        );
        assert_eq!(expr.to_string(), "(n + 1)");
    }

    #[test]
    fn sub_displays_parens() {
        let expr = NormalExpr::Sub(
            Box::new(NormalExpr::Var(Intern::from_ref("m"))),
            Box::new(NormalExpr::Value(ConstValue::Int(1))),
        );
        assert_eq!(expr.to_string(), "(m - 1)");
    }

    #[test]
    fn mul_displays_parens() {
        let expr = NormalExpr::Mul(
            Box::new(NormalExpr::Value(ConstValue::Int(2))),
            Box::new(NormalExpr::Var(Intern::from_ref("n"))),
        );
        assert_eq!(expr.to_string(), "(2 * n)");
    }

    #[test]
    fn value_float_displays() {
        let expr = NormalExpr::Value(ConstValue::Float(crate::HashFloat(314.0 / 100.0)));
        assert_eq!(expr.to_string(), "3.14");
    }

    #[test]
    fn value_string_displays() {
        let expr = NormalExpr::Value(ConstValue::String("hello".into()));
        assert_eq!(expr.to_string(), r#""hello""#);
    }

    #[test]
    fn value_int_round_trips() {
        let expr = NormalExpr::Value(ConstValue::Int(3));
        assert_eq!(expr.to_string(), "3");
    }

    #[test]
    fn equality_and_hash() {
        use std::collections::HashSet;
        let a = NormalExpr::Var(Intern::from_ref("n"));
        let b = NormalExpr::Var(Intern::from_ref("n"));
        let c = NormalExpr::Var(Intern::from_ref("m"));
        assert_eq!(a, b);
        assert_ne!(a, c);

        let mut set = HashSet::new();
        set.insert(a.clone());
        set.insert(b.clone());
        assert_eq!(set.len(), 1);
        set.insert(c);
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn inferred_identity_controls_equality_and_hash() {
        use std::collections::HashSet;

        let binder = BinderId::new(7, BinderOwner::Definition(Intern::from_ref("Vector")));
        let a = NormalExpr::Inferred(DependentArgId::new(binder, 0));
        let same = NormalExpr::Inferred(DependentArgId::new(binder, 0));
        let other_slot = NormalExpr::Inferred(DependentArgId::new(binder, 1));
        let other_owner = NormalExpr::Inferred(DependentArgId::new(
            BinderId::new(7, BinderOwner::Expression(3)),
            0,
        ));

        assert_eq!(a, same);
        assert_ne!(a, other_slot);
        assert_ne!(a, other_owner);

        let set = HashSet::from([a, same, other_slot, other_owner]);
        assert_eq!(set.len(), 3);
    }

    #[test]
    fn inferred_display_hides_identity() {
        let expr = NormalExpr::Inferred(DependentArgId::new(
            BinderId::new(19, BinderOwner::Expression(42)),
            3,
        ));
        assert_eq!(expr.to_string(), "?");
    }

    #[test]
    fn nested_add_mul_expression() {
        let expr = NormalExpr::Add(
            Box::new(NormalExpr::Mul(
                Box::new(NormalExpr::Value(ConstValue::Int(2))),
                Box::new(NormalExpr::Var(Intern::from_ref("n"))),
            )),
            Box::new(NormalExpr::Value(ConstValue::Int(1))),
        );
        assert_eq!(expr.to_string(), "((2 * n) + 1)");
    }
}
