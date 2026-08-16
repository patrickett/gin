//! Normalization for [`NormalExpr`] — reduces trees to a canonical form by folding
//! known constants and applying simple algebraic identities.

use ast::ConstValue;
use ast::NormalExpr;

/// Trait for types that can be normalized to a canonical form.
pub trait Normalize: Sized {
    fn normalize(&self) -> Self;

    fn normalize_to_value(&self) -> Option<ConstValue> {
        let _ = self;
        None
    }
}

impl Normalize for NormalExpr {
    fn normalize(&self) -> NormalExpr {
        match self {
            NormalExpr::Value(_)
            | NormalExpr::Var(_)
            | NormalExpr::Inferred(_)
            | NormalExpr::TargetQuery { .. } => self.clone(),
            NormalExpr::Add(l, r) => {
                let l = l.normalize();
                let r = r.normalize();

                if l.is_zero() {
                    return r;
                }
                if r.is_zero() {
                    return l;
                }
                if let NormalExpr::Add(inner_l, inner_r) = &r {
                    let inner_l = inner_l.normalize();
                    let inner_r = inner_r.normalize();
                    return NormalExpr::Add(
                        Box::new(NormalExpr::Add(Box::new(l), Box::new(inner_l))),
                        Box::new(inner_r),
                    );
                }
                if let NormalExpr::Add(inner_l, inner_r) = &l {
                    let inner_l = inner_l.normalize();
                    let inner_r = inner_r.normalize();
                    return NormalExpr::Add(
                        Box::new(NormalExpr::Add(Box::new(inner_l), Box::new(inner_r))),
                        Box::new(r),
                    );
                }
                if let (Some(a), Some(b)) = (l.as_const_int(), r.as_const_int()) {
                    return (a.wrapping_add(b)).into();
                }
                NormalExpr::Add(Box::new(l), Box::new(r))
            }
            NormalExpr::Sub(l, r) => {
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
                NormalExpr::Sub(Box::new(l), Box::new(r))
            }
            NormalExpr::Mul(l, r) => {
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
                NormalExpr::Mul(Box::new(l), Box::new(r))
            }
        }
    }

    fn normalize_to_value(&self) -> Option<ConstValue> {
        match self.normalize() {
            NormalExpr::Value(v) => Some(v.clone()),
            _ => None,
        }
    }
}
#[cfg(test)]
#[path = "../tests/normal_expr_tests.rs"]
mod tests;
