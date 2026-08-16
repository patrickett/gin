//! Pure type inference — given an env, what type does this expression have?
//!
//! This module is the **read-only computation layer** for type inference. It defines the
//! [`TyInfer`] trait and implements it for every expression type. Each impl is a pure
//! function: borrow a [`TyInferEnv`], return a [`Ty`]. No mutation, no validation,
//! no diagnostics.

use std::collections::HashMap;

use internment::Intern;

use crate::analysis::type_surface::{
    TypeEnv, expr_is_type_surface, mangled_fn_call_name, typevars_from_receiver_expr,
};
use crate::ty::Ty;
use ast::{
    BinOp, Binary, Bind, BindValue, Expr, FnCall, HashFloat, Literal, NormalExpr, ParameterKind,
    Parameters, TagCall, WhenArm, WhenExpr,
};

/// Abstracts over different "local variable" type representations.
///
/// Implemented by `HashMap<Intern<String>, Ty>` (used by typeck) and by
/// `CodegenContext` (used by codegen) so that expression type inference works
/// everywhere without adapter functions.
pub trait LocalTypes {
    fn get_type(&self, name: &Intern<String>) -> Option<Ty>;
}

impl LocalTypes for HashMap<Intern<String>, Ty> {
    fn get_type(&self, name: &Intern<String>) -> Option<Ty> {
        self.get(name).cloned()
    }
}

/// Everything an expression needs to infer its type, bundled into one struct.
pub struct TyInferEnv<'a> {
    pub tag_types: &'a HashMap<Intern<String>, Ty>,
    pub fn_return_types: &'a HashMap<Intern<String>, Ty>,
    pub locals: &'a dyn LocalTypes,
    /// Declaration parameters for each tag. Used to fill default type arguments
    /// when fewer args are provided at the use site (e.g. `Box(Int)` → `Box(Int, LibcAllocator)`).
    /// `None` when defaults are not needed (inference, codegen).
    pub tag_params: Option<&'a HashMap<Intern<String>, Parameters>>,
}

/// Each expression type implements this to know its own type.
pub trait TyInfer {
    fn infer_ty(&self, env: &TyInferEnv) -> Ty;
}

impl TyInfer for Literal {
    fn infer_ty(&self, env: &TyInferEnv) -> Ty {
        match self {
            Literal::Int(_) | Literal::Number(_) => {
                Ty::UnresolvedLiteral(ast::ty::LiteralKind::Integer)
            }
            Literal::Float(HashFloat(f)) => Ty::Float {
                value: Some(HashFloat(*f)),
            },
            Literal::String(_) => env
                .tag_types
                .get(&Intern::<String>::from_ref("String"))
                .cloned()
                .unwrap_or(Ty::Opaque(Intern::<String>::from_ref("String"))),
        }
    }
}

#[cfg(test)]
#[path = "../../tests/analysis_infer_tests.rs"]
mod tests;

impl TyInfer for Binary {
    fn infer_ty(&self, env: &TyInferEnv) -> Ty {
        if self.op == BinOp::Equal {
            return env
                .tag_types
                .get(&Intern::from_ref("Bool"))
                .cloned()
                .unwrap_or(Ty::Opaque(Intern::from_ref("Bool")));
        }
        let lhs_ty = self.lhs.infer_ty(env);
        let rhs_ty = self.rhs.infer_ty(env);
        match (&lhs_ty, &rhs_ty) {
            (
                Ty::Float {
                    value: Some(HashFloat(a)),
                },
                Ty::Float {
                    value: Some(HashFloat(b)),
                },
            ) => {
                let folded = match self.op {
                    BinOp::Add => Some(HashFloat(a + b)),
                    BinOp::Subtract => Some(HashFloat(a - b)),
                    BinOp::Multiply => Some(HashFloat(a * b)),
                    BinOp::Divide => Some(HashFloat(a / b)),
                    _ => None,
                };
                folded
                    .map(|v| Ty::Float { value: Some(v) })
                    .unwrap_or(lhs_ty)
            }
            _ => {
                if lhs_ty.is_float() {
                    lhs_ty
                } else if rhs_ty.is_float() {
                    rhs_ty
                } else {
                    lhs_ty
                }
            }
        }
    }
}

impl TyInfer for FnCall {
    fn infer_ty(&self, env: &TyInferEnv) -> Ty {
        let name = self.path.root;
        if self.path.segments.is_empty() {
            if let Some(local_ty) = env.locals.get_type(&name) {
                return local_ty;
            }
            let mangled = mangled_fn_call_name(self);
            return env
                .fn_return_types
                .get(&mangled)
                .cloned()
                .unwrap_or(Ty::i64());
        }

        if let Some(mut ty) = env.locals.get_type(&name) {
            for seg in &self.path.segments {
                ty = match &ty {
                    ty if ty.pointee_ty().is_some_and(Ty::is_record) => {
                        let Some(Ty::Record { fields, .. }) = ty.pointee_ty() else {
                            unreachable!("record predicate and pointee lookup disagree")
                        };
                        fields
                            .iter()
                            .find(|(fname, _)| fname.as_str() == seg.as_str())
                            .map(|(_, fty)| (**fty).clone())
                            .unwrap_or(ty.clone())
                    }
                    Ty::Record { fields, .. } => fields
                        .iter()
                        .find(|(fname, _)| fname.as_str() == seg.as_str())
                        .map(|(_, fty)| (**fty).clone())
                        .unwrap_or(ty),
                    _ => return ty,
                };
            }
            return ty;
        }

        let mangled = mangled_fn_call_name(self);
        env.fn_return_types
            .get(&mangled)
            .cloned()
            .unwrap_or(Ty::i64())
    }
}

impl TyInfer for Bind {
    fn infer_ty(&self, env: &TyInferEnv) -> Ty {
        let subst: HashMap<Intern<String>, Ty> = self
            .receiver_type_surface()
            .map(|sp| typevars_from_receiver_expr(&sp.value))
            .unwrap_or_default();

        if let Some(sp) = &self.return_tag
            && expr_is_type_surface(&sp.value)
        {
            return TypeEnv::new(env.tag_types)
                .with_subst(&subst)
                .with_opt_tag_params(env.tag_params)
                .resolve(&sp.value);
        }

        let mut locals: HashMap<Intern<String>, Ty> = match self.params.as_ref() {
            None => HashMap::new(),
            Some(params) => params
                .iter()
                .map(|(name, kind)| {
                    (
                        *name,
                        resolve_parameter_kind_with_subst(
                            *name,
                            &kind.kind,
                            env.tag_types,
                            env.fn_return_types,
                            &subst,
                            env.tag_params,
                        ),
                    )
                })
                .collect(),
        };
        if let Some(sp) = self.receiver_type_surface()
            && expr_is_type_surface(&sp.value)
        {
            let recv_ty = TypeEnv::new(env.tag_types)
                .with_subst(&subst)
                .with_opt_tag_params(env.tag_params)
                .resolve(&sp.value);
            locals.insert(Intern::<String>::from_ref("self"), recv_ty);
        }

        let bind_env = TyInferEnv {
            tag_types: env.tag_types,
            fn_return_types: env.fn_return_types,
            locals: &locals,
            tag_params: env.tag_params,
        };
        match &self.value {
            BindValue::Expr(expr) => expr.infer_ty(&bind_env),
            BindValue::Body { ret, .. } => match &ret.value {
                Some(expr) => expr.infer_ty(&bind_env),
                None => Ty::Unit,
            },
            BindValue::Extern | BindValue::Unassigned => {
                // For unassigned binds, return unit as placeholder.
                // The type is determined by the return_tag annotation.
                Ty::Unit
            }
        }
    }
}

impl TyInfer for TagCall {
    fn infer_ty(&self, env: &TyInferEnv) -> Ty {
        if let Some(ty) = env.tag_types.values().find_map(|ty| {
            if let Ty::Union { variants, .. } = ty
                && variants.iter().any(|v| v.name == self.name)
            {
                return Some(ty.clone());
            }
            None
        }) {
            return ty;
        }
        env.tag_types
            .get(&self.name)
            .cloned()
            .unwrap_or(Ty::Opaque(self.name))
    }
}

impl TyInfer for WhenExpr {
    fn infer_ty(&self, env: &TyInferEnv) -> Ty {
        let body = self
            .arms
            .iter()
            .find_map(|a| {
                if let WhenArm::Else(b, _) = a {
                    Some(b.as_ref())
                } else {
                    None
                }
            })
            .or_else(|| {
                self.arms.first().map(|a| match a {
                    WhenArm::Cond { body, .. }
                    | WhenArm::Is { body, .. }
                    | WhenArm::Else(body, _) => body.as_ref(),
                })
            });
        match body {
            Some(b) => b.infer_ty(env),
            None => Ty::Unit,
        }
    }
}

impl TyInfer for Expr {
    fn infer_ty(&self, env: &TyInferEnv) -> Ty {
        match self {
            Expr::Lit(lit) => lit.infer_ty(env),
            Expr::Binary(bin) => bin.infer_ty(env),
            Expr::FnCall(call) => call.infer_ty(env),
            Expr::Bind(bind) => bind.infer_ty(env),
            Expr::TagCall(tc) => tc.infer_ty(env),
            Expr::When(w) => w.infer_ty(env),

            Expr::AnonymousTag(name) => Ty::Opaque(*name),
            Expr::FormatString(_) => Ty::Opaque(Intern::<String>::from_ref("format_string")),
            Expr::Loop(_) => Ty::Unit,
            Expr::If(_) => Ty::Unit,
            Expr::Range(_) => Ty::Opaque(Intern::<String>::from_ref("range_expr")),
            Expr::TupleSet { value, .. } => value.infer_ty(env),
            Expr::BufSet { value, .. } => value.infer_ty(env),
            Expr::Cast { ty, .. } => ty.head_name().map(Ty::Opaque).unwrap_or_else(|| match ty {
                ast::TypeReference::Unit => Ty::Unit,
                ast::TypeReference::Pointer(_) => Ty::Ptr {
                    inner: Box::new(Ty::Opaque(Intern::from_ref("<pointee>"))),
                },
                _ => unreachable!(),
            }),
            Expr::TargetQuery { kind, operand } => Ty::AnonymousInteger {
                validity: ast::integer::IntegerValidity::new(
                    ast::integer::IntegerDomain::symbolic(ast::NormalExpr::TargetQuery {
                        kind: *kind,
                        operand: operand.clone(),
                    }),
                ),
            },

            Expr::SelfRef => env
                .locals
                .get_type(&Intern::<String>::from_ref("self"))
                .unwrap_or_else(|| Ty::Opaque(Intern::<String>::from_ref("Self"))),

            Expr::TupleAlloc { init, size } => {
                let elem = init.infer_ty(env);
                let size = as_size_normal_expr_with_arithmetic(&size.value).unwrap_or_else(|| {
                    NormalExpr::Var(Intern::new(format!(
                        "_unsupported_array_size_{:?}",
                        size.span_id
                    )))
                });
                Ty::Array {
                    elem: Box::new(elem),
                    size,
                }
            }

            Expr::TupleGet { base, index } => match base.infer_ty(env) {
                Ty::Array { elem, .. } => *elem,
                Ty::Tuple(fields) => fields
                    .into_iter()
                    .nth(*index)
                    .unwrap_or(Ty::Opaque(Intern::from_ref("unresolved-tuple-element"))),
                _ => Ty::Opaque(Intern::from_ref("unresolved-array-element")),
            },

            Expr::Destructure { value, .. } => value.infer_ty(env),
            Expr::RecordSet { value, .. } => value.infer_ty(env),
            Expr::RecordGet { base, field } => match base.infer_ty(env) {
                Ty::Record { fields, .. } => fields
                    .iter()
                    .find(|(n, _)| n == field)
                    .map(|(_, t)| t.as_ref().clone())
                    .unwrap_or_else(|| Ty::Opaque(*field)),
                _ => Ty::Opaque(*field),
            },

            Expr::BufGet { buf, .. } => {
                let buffer_ty = buf.infer_ty(env);
                buffer_ty
                    .pointee_ty()
                    .cloned()
                    .or(match buffer_ty {
                        Ty::Array { elem, .. } => Some(*elem),
                        _ => None,
                    })
                    .unwrap_or(Ty::Opaque(Intern::from_ref("unresolved-array-element")))
            }

            Expr::TakePtr(inner) => Ty::Address {
                pointee: Box::new(inner.infer_ty(env)),
                address_space: 0,
            },

            Expr::Ref { inner, mutable, .. } => Ty::Ref {
                inner: Box::new(inner.infer_ty(env)),
                mutable: *mutable,
            },

            Expr::ConsumeArg(inner) => inner.infer_ty(env),

            Expr::Eat(inner) => inner.infer_ty(env),

            Expr::Deref(inner) => inner
                .infer_ty(env)
                .pointee_ty()
                .cloned()
                .unwrap_or(Ty::Unit),

            Expr::Negate(inner) => match inner.infer_ty(env) {
                Ty::Float {
                    value: Some(HashFloat(f)),
                } => Ty::Float {
                    value: Some(HashFloat(-f)),
                },
                other => other,
            },

            Expr::RecordLit(fields) => {
                let tys = fields.iter().map(|(_, e)| e.infer_ty(env)).collect();
                Ty::Tuple(tys)
            }
            Expr::TupleLit(elems) => Ty::Tuple(elems.iter().map(|e| e.infer_ty(env)).collect()),
            Expr::List(_) => Ty::Opaque(Intern::<String>::from_ref("List")),
        }
    }
}

fn as_size_normal_expr_with_arithmetic(expr: &Expr) -> Option<NormalExpr> {
    match expr {
        Expr::Lit(Literal::Int(n)) => Some(NormalExpr::from(*n)),
        Expr::Lit(Literal::Number(n)) => Some(NormalExpr::from(*n as i128)),
        Expr::FnCall(call) if call.args.is_none() => Some(NormalExpr::Var(call.path.value.root)),
        Expr::Bind(b) => Some(NormalExpr::Var(b.name)),
        Expr::Binary(binary) => {
            let lhs = as_size_normal_expr_with_arithmetic(&binary.lhs.value)?;
            let rhs = as_size_normal_expr_with_arithmetic(&binary.rhs.value)?;
            Some(match binary.op {
                BinOp::Add => NormalExpr::Add(Box::new(lhs), Box::new(rhs)),
                BinOp::Subtract => NormalExpr::Sub(Box::new(lhs), Box::new(rhs)),
                BinOp::Multiply => NormalExpr::Mul(Box::new(lhs), Box::new(rhs)),
                _ => return None,
            })
        }
        _ => None,
    }
}

/// Resolve a `ParameterKind` to a `Ty`, consulting a method-scoped
/// type-variable substitution map. Takes the parameter `name` so bare-id
/// (`Generic`) params can be resolved as a fresh `Ty::Opaque(name)`
/// type variable rather than the legacy `Int64` fallback.
///
/// This makes `start` and `end` in `CustomRange has start, end)` (and the
/// matching `CustomRange.new(start, end) ...` method) into independent fresh
/// type variables — call sites can then bind each to any type without
/// rejecting mixed inputs (`CustomRange.new(1, "hi")`). For shared-type
/// parameters use `start x, end x` with an explicit type-variable name.
///
/// `subst` lets `start x` and `end x` in
/// `Range[x].new(start x, end x) Range[x]: ...` both resolve to the same
/// `Ty::Opaque(x)`. Pass an empty map for non-method binds.
pub fn resolve_parameter_kind_with_subst(
    name: Intern<String>,
    kind: &ParameterKind,
    tag_types: &HashMap<Intern<String>, Ty>,
    fn_return_types: &HashMap<Intern<String>, Ty>,
    subst: &HashMap<Intern<String>, Ty>,
    tag_params: Option<&HashMap<Intern<String>, Parameters>>,
) -> Ty {
    match kind {
        ParameterKind::Tagged(sp)
        | ParameterKind::ValueParam { ty: sp }
        | ParameterKind::Inferred { ty: sp } => {
            if expr_is_type_surface(&sp.value) {
                TypeEnv::new(tag_types)
                    .with_subst(subst)
                    .with_opt_tag_params(tag_params)
                    .resolve_expr(&sp.value)
            } else {
                Ty::Opaque(Intern::<String>::from_ref("?"))
            }
        }
        ParameterKind::Generic => Ty::Opaque(name),
        ParameterKind::Default(expr) => {
            let env = TyInferEnv {
                tag_types,
                fn_return_types,
                locals: &HashMap::new(),
                tag_params,
            };
            expr.infer_ty(&env)
        }
    }
}

/// A layered local-types overlay: wraps a parent with a small Vec of new bindings.
pub struct LayeredLocals<'a> {
    parent: &'a dyn LocalTypes,
    bindings: Vec<(Intern<String>, Ty)>,
}

impl<'a> LayeredLocals<'a> {
    pub fn new(parent: &'a dyn LocalTypes) -> Self {
        Self {
            parent,
            bindings: Vec::new(),
        }
    }

    pub fn insert(&mut self, name: Intern<String>, ty: Ty) {
        self.bindings.push((name, ty));
    }

    pub fn contains_key(&self, name: &Intern<String>) -> bool {
        self.bindings.iter().rev().any(|(n, _)| n == name) || self.parent.get_type(name).is_some()
    }
}

impl LocalTypes for LayeredLocals<'_> {
    fn get_type(&self, name: &Intern<String>) -> Option<Ty> {
        self.bindings
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, t)| t.clone())
            .or_else(|| self.parent.get_type(name))
    }
}

impl TyInfer for ast::Typed<ast::Expr> {
    fn infer_ty(&self, env: &TyInferEnv) -> Ty {
        if let Some(ty) = self.current_ty() {
            return ty.clone();
        }
        self.value.infer_ty(env)
    }
}

impl TyInfer for Box<ast::Typed<ast::Expr>> {
    fn infer_ty(&self, env: &TyInferEnv) -> Ty {
        (**self).infer_ty(env)
    }
}
