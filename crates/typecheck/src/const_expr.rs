//! Normalization for [`ConstExpr`] — reduces trees to a canonical form by folding
//! known constants and applying simple algebraic identities.

use ast::ConstExpr;
use ast::ConstValue;

/// Trait for types that can be normalized to a canonical form.
pub trait Normalize: Sized {
    fn normalize(&self) -> Self;

    fn normalize_to_value(&self) -> Option<ConstValue> {
        let _ = self;
        None
    }
}

impl Normalize for ConstExpr {
    fn normalize(&self) -> ConstExpr {
        match self {
            ConstExpr::Value(_) | ConstExpr::Var(_) | ConstExpr::Inferred(_) => self.clone(),
            ConstExpr::Add(l, r) => {
                let l = l.normalize();
                let r = r.normalize();

                if l.is_zero() {
                    return r;
                }
                if r.is_zero() {
                    return l;
                }
                if let ConstExpr::Add(inner_l, inner_r) = &r {
                    let inner_l = inner_l.normalize();
                    let inner_r = inner_r.normalize();
                    return ConstExpr::Add(
                        Box::new(ConstExpr::Add(Box::new(l), Box::new(inner_l))),
                        Box::new(inner_r),
                    );
                }
                if let ConstExpr::Add(inner_l, inner_r) = &l {
                    let inner_l = inner_l.normalize();
                    let inner_r = inner_r.normalize();
                    return ConstExpr::Add(
                        Box::new(ConstExpr::Add(Box::new(inner_l), Box::new(inner_r))),
                        Box::new(r),
                    );
                }
                if let (Some(a), Some(b)) = (l.as_const_int(), r.as_const_int()) {
                    return (a.wrapping_add(b)).into();
                }
                ConstExpr::Add(Box::new(l), Box::new(r))
            }
            ConstExpr::Sub(l, r) => {
                let l = l.normalize();
                let r = r.normalize();

                if r.is_zero() {
                    return l;
                }
                if l == r {
                    return 0.into();
                }
                if let (Some(a), Some(b)) = (l.as_const_int(), r.as_const_int()) {
                    return (a.wrapping_sub(b)).into();
                }
                ConstExpr::Sub(Box::new(l), Box::new(r))
            }
            ConstExpr::Mul(l, r) => {
                let l = l.normalize();
                let r = r.normalize();

                if l.is_zero() || r.is_zero() {
                    return 0.into();
                }
                if l.is_one() {
                    return r;
                }
                if r.is_one() {
                    return l;
                }
                if let (Some(a), Some(b)) = (l.as_const_int(), r.as_const_int()) {
                    return (a.wrapping_mul(b)).into();
                }
                ConstExpr::Mul(Box::new(l), Box::new(r))
            }
        }
    }

    fn normalize_to_value(&self) -> Option<ConstValue> {
        match self.normalize() {
            ConstExpr::Value(v) => Some(v.clone()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ast::{BinderId, BinderOwner, ConstExpr, ConstValue, DependentArgId};
    use internment::Intern;

    #[test]
    fn value_stays_as_is() {
        let expr = ConstExpr::Value(ConstValue::Int(42));
        assert_eq!(expr.normalize(), expr);
    }

    #[test]
    fn var_stays_as_is() {
        let expr = ConstExpr::Var(Intern::from_ref("n"));
        assert_eq!(expr.normalize(), expr);
    }

    #[test]
    fn inferred_stays_as_is() {
        let expr = ConstExpr::Inferred(DependentArgId::new(
            BinderId::new(5, BinderOwner::Expression(12)),
            2,
        ));
        assert_eq!(expr.normalize(), expr);
        assert_eq!(expr.normalize_to_value(), None);
    }

    #[test]
    fn add_zero_left() {
        let expr = ConstExpr::Add(
            Box::new(ConstExpr::Value(ConstValue::Int(0))),
            Box::new(ConstExpr::Var(Intern::from_ref("n"))),
        );
        assert_eq!(expr.normalize(), ConstExpr::Var(Intern::from_ref("n")));
    }

    #[test]
    fn add_zero_right() {
        let expr = ConstExpr::Add(
            Box::new(ConstExpr::Var(Intern::from_ref("n"))),
            Box::new(ConstExpr::Value(ConstValue::Int(0))),
        );
        assert_eq!(expr.normalize(), ConstExpr::Var(Intern::from_ref("n")));
    }

    #[test]
    fn add_two_concrete_values() {
        let expr = ConstExpr::Add(
            Box::new(ConstExpr::Value(ConstValue::Int(2))),
            Box::new(ConstExpr::Value(ConstValue::Int(3))),
        );
        assert_eq!(expr.normalize(), ConstExpr::Value(ConstValue::Int(5)));
    }

    #[test]
    fn sub_zero_right() {
        let expr = ConstExpr::Sub(
            Box::new(ConstExpr::Var(Intern::from_ref("n"))),
            Box::new(ConstExpr::Value(ConstValue::Int(0))),
        );
        assert_eq!(expr.normalize(), ConstExpr::Var(Intern::from_ref("n")));
    }

    #[test]
    fn sub_same_var() {
        let n = ConstExpr::Var(Intern::from_ref("n"));
        let expr = ConstExpr::Sub(Box::new(n.clone()), Box::new(n));
        assert_eq!(expr.normalize(), ConstExpr::Value(ConstValue::Int(0)));
    }

    #[test]
    fn sub_two_concrete_values() {
        let expr = ConstExpr::Sub(
            Box::new(ConstExpr::Value(ConstValue::Int(5))),
            Box::new(ConstExpr::Value(ConstValue::Int(3))),
        );
        assert_eq!(expr.normalize(), ConstExpr::Value(ConstValue::Int(2)));
    }

    #[test]
    fn mul_by_one_left() {
        let expr = ConstExpr::Mul(
            Box::new(ConstExpr::Value(ConstValue::Int(1))),
            Box::new(ConstExpr::Var(Intern::from_ref("n"))),
        );
        assert_eq!(expr.normalize(), ConstExpr::Var(Intern::from_ref("n")));
    }

    #[test]
    fn mul_by_one_right() {
        let expr = ConstExpr::Mul(
            Box::new(ConstExpr::Var(Intern::from_ref("n"))),
            Box::new(ConstExpr::Value(ConstValue::Int(1))),
        );
        assert_eq!(expr.normalize(), ConstExpr::Var(Intern::from_ref("n")));
    }

    #[test]
    fn mul_by_zero_left() {
        let expr = ConstExpr::Mul(
            Box::new(ConstExpr::Value(ConstValue::Int(0))),
            Box::new(ConstExpr::Var(Intern::from_ref("n"))),
        );
        assert_eq!(expr.normalize(), ConstExpr::Value(ConstValue::Int(0)));
    }

    #[test]
    fn mul_by_zero_right() {
        let expr = ConstExpr::Mul(
            Box::new(ConstExpr::Var(Intern::from_ref("n"))),
            Box::new(ConstExpr::Value(ConstValue::Int(0))),
        );
        assert_eq!(expr.normalize(), ConstExpr::Value(ConstValue::Int(0)));
    }

    #[test]
    fn mul_two_concrete_values() {
        let expr = ConstExpr::Mul(
            Box::new(ConstExpr::Value(ConstValue::Int(3))),
            Box::new(ConstExpr::Value(ConstValue::Int(4))),
        );
        assert_eq!(expr.normalize(), ConstExpr::Value(ConstValue::Int(12)));
    }

    #[test]
    fn add_flatten_right_associative() {
        let a = ConstExpr::Var(Intern::from_ref("a"));
        let b = ConstExpr::Var(Intern::from_ref("b"));
        let c = ConstExpr::Var(Intern::from_ref("c"));
        let inner = ConstExpr::Add(Box::new(a.clone()), Box::new(b.clone()));
        let outer = ConstExpr::Add(Box::new(inner), Box::new(c.clone()));
        let result = outer.normalize();
        assert_eq!(
            result,
            ConstExpr::Add(
                Box::new(ConstExpr::Add(Box::new(a), Box::new(b))),
                Box::new(c),
            )
        );
    }

    #[test]
    fn add_flatten_left_associative() {
        let a = ConstExpr::Var(Intern::from_ref("a"));
        let b = ConstExpr::Var(Intern::from_ref("b"));
        let c = ConstExpr::Var(Intern::from_ref("c"));
        let inner = ConstExpr::Add(Box::new(b.clone()), Box::new(c.clone()));
        let outer = ConstExpr::Add(Box::new(a.clone()), Box::new(inner));
        let result = outer.normalize();
        let expected = ConstExpr::Add(
            Box::new(ConstExpr::Add(Box::new(a), Box::new(b))),
            Box::new(c),
        );
        assert_eq!(result, expected);
    }

    #[test]
    fn normalize_to_value_concrete_value() {
        let expr = ConstExpr::Value(ConstValue::Int(42));
        assert_eq!(expr.normalize_to_value(), Some(ConstValue::Int(42)));
    }

    #[test]
    fn normalize_to_value_variable_returns_none() {
        let expr = ConstExpr::Var(Intern::from_ref("n"));
        assert_eq!(expr.normalize_to_value(), None);
    }

    #[test]
    fn normalize_to_value_folds_arithmetic() {
        let expr = ConstExpr::Add(
            Box::new(ConstExpr::Value(ConstValue::Int(2))),
            Box::new(ConstExpr::Value(ConstValue::Int(3))),
        );
        assert_eq!(expr.normalize_to_value(), Some(ConstValue::Int(5)));
    }

    #[test]
    fn normalize_to_value_variable_identity_returns_none() {
        let expr = ConstExpr::Add(
            Box::new(ConstExpr::Var(Intern::from_ref("n"))),
            Box::new(ConstExpr::Value(ConstValue::Int(0))),
        );
        assert_eq!(expr.normalize_to_value(), None);
    }
}
