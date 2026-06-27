//! Symbolic compile-time constant expressions.
//!
//! `ConstExpr` represents unevaluated constant expressions (e.g. `n + m`, `2 * n`)
//! in contrast with [`ConstValue`](crate::ConstValue) which holds already-evaluated constants.

use crate::ConstValue;
use internment::Intern;
use std::fmt;

/// A symbolic compile-time constant expression — unevaluated, in contrast with
/// [`ConstValue`]. Evaluation/normalization is handled by a separate pass.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ConstExpr {
    Value(ConstValue),
    Var(Intern<String>),
    Add(Box<ConstExpr>, Box<ConstExpr>),
    Sub(Box<ConstExpr>, Box<ConstExpr>),
    Mul(Box<ConstExpr>, Box<ConstExpr>),
}

impl ConstExpr {
    pub fn is_zero(&self) -> bool {
        *self == ConstValue::ZERO
    }

    pub fn is_one(&self) -> bool {
        *self == ConstValue::ONE
    }

    pub fn as_const_int(&self) -> Option<i128> {
        match self {
            ConstExpr::Value(ConstValue::Int(n)) => Some(*n),
            _ => None,
        }
    }
}

impl From<i128> for ConstExpr {
    fn from(n: i128) -> Self {
        ConstExpr::Value(ConstValue::Int(n))
    }
}

impl PartialEq<ConstValue> for ConstExpr {
    fn eq(&self, other: &ConstValue) -> bool {
        matches!(self, ConstExpr::Value(v) if v == other)
    }
}

impl fmt::Display for ConstExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConstExpr::Value(v) => match v {
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
                            write!(f, "{}", ConstExpr::Value(arg.clone()))?;
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
                        write!(f, "{}: {}", name.as_str(), ConstExpr::Value(val.clone()))?;
                    }
                    write!(f, " }}")
                }
                ConstValue::List(items) => {
                    write!(f, "[")?;
                    for (i, item) in items.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", ConstExpr::Value(item.clone()))?;
                    }
                    write!(f, "]")
                }
            },
            ConstExpr::Var(name) => write!(f, "{}", name.as_str()),
            ConstExpr::Add(l, r) => write!(f, "({l} + {r})"),
            ConstExpr::Sub(l, r) => write!(f, "({l} - {r})"),
            ConstExpr::Mul(l, r) => write!(f, "({l} * {r})"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ConstValue;

    #[test]
    fn value_int_displays_as_number() {
        let expr = ConstExpr::Value(ConstValue::Int(42));
        assert_eq!(expr.to_string(), "42");
    }

    #[test]
    fn var_displays_as_name() {
        let expr = ConstExpr::Var(Intern::from_ref("n"));
        assert_eq!(expr.to_string(), "n");
    }

    #[test]
    fn add_displays_parens() {
        let expr = ConstExpr::Add(
            Box::new(ConstExpr::Var(Intern::from_ref("n"))),
            Box::new(ConstExpr::Value(ConstValue::Int(1))),
        );
        assert_eq!(expr.to_string(), "(n + 1)");
    }

    #[test]
    fn sub_displays_parens() {
        let expr = ConstExpr::Sub(
            Box::new(ConstExpr::Var(Intern::from_ref("m"))),
            Box::new(ConstExpr::Value(ConstValue::Int(1))),
        );
        assert_eq!(expr.to_string(), "(m - 1)");
    }

    #[test]
    fn mul_displays_parens() {
        let expr = ConstExpr::Mul(
            Box::new(ConstExpr::Value(ConstValue::Int(2))),
            Box::new(ConstExpr::Var(Intern::from_ref("n"))),
        );
        assert_eq!(expr.to_string(), "(2 * n)");
    }

    #[test]
    fn value_float_displays() {
        let expr = ConstExpr::Value(ConstValue::Float(crate::HashFloat(3.14)));
        assert_eq!(expr.to_string(), "3.14");
    }

    #[test]
    fn value_string_displays() {
        let expr = ConstExpr::Value(ConstValue::String("hello".into()));
        assert_eq!(expr.to_string(), r#""hello""#);
    }

    #[test]
    fn value_int_round_trips() {
        let expr = ConstExpr::Value(ConstValue::Int(3));
        assert_eq!(expr.to_string(), "3");
    }

    #[test]
    fn equality_and_hash() {
        use std::collections::HashSet;
        let a = ConstExpr::Var(Intern::from_ref("n"));
        let b = ConstExpr::Var(Intern::from_ref("n"));
        let c = ConstExpr::Var(Intern::from_ref("m"));
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
    fn nested_add_mul_expression() {
        let expr = ConstExpr::Add(
            Box::new(ConstExpr::Mul(
                Box::new(ConstExpr::Value(ConstValue::Int(2))),
                Box::new(ConstExpr::Var(Intern::from_ref("n"))),
            )),
            Box::new(ConstExpr::Value(ConstValue::Int(1))),
        );
        assert_eq!(expr.to_string(), "((2 * n) + 1)");
    }
}
