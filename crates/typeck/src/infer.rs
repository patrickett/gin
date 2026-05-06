//! Pure type inference — given an env, what type does this expression have?
//!
//! This module is the **read-only computation layer** for type inference. It defines the
//! [`TyInfer`] trait and implements it for every expression type. Each impl is a pure
//! function: borrow a [`TyInferEnv`], return a [`Ty`]. No mutation, no validation,
//! no diagnostics.
//!
//! The supporting infrastructure lives here too:
//! - [`TyInferEnv`] bundles the read-only references needed for inference
//!   (tag types, function return types, local variable types).
//! - [`LocalTypes`] abstracts over different local-variable representations so
//!   inference works identically in typeck and codegen.
//! - [`crate::resolve::resolve_type_expr_from_map`] resolves type-surface [`Expr`] nodes against a type map.
//!
//! For environment building, type checking, and diagnostics, see `env.rs` and `check.rs`.

use std::collections::HashMap;

use ast::{
    BinOp, Binary, Bind, BindValue, Expr, FnCall, Literal, ParameterKind, Spanned, TagCall,
    WhenArm, WhenExpr,
};
use internment::Intern;

use crate::resolve::{is_type_surface, resolve_type_expr_with_subst, typevars_from_receiver};
use crate::ty::str_record_ty;
use crate::Ty;

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

/// A layered local-types overlay: wraps a parent with a small Vec of new bindings.
///
/// Avoids cloning the entire parent HashMap when entering a new scope.
pub(crate) struct LayeredLocals<'a> {
    parent: &'a dyn LocalTypes,
    bindings: Vec<(Intern<String>, Ty)>,
}

impl<'a> LayeredLocals<'a> {
    pub(crate) fn new(parent: &'a dyn LocalTypes) -> Self {
        Self {
            parent,
            bindings: Vec::new(),
        }
    }

    pub(crate) fn insert(&mut self, name: Intern<String>, ty: Ty) {
        self.bindings.push((name, ty));
    }

    pub(crate) fn contains_key(&self, name: &Intern<String>) -> bool {
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

/// Everything an expression needs to infer its type, bundled into one struct.
pub struct TyInferEnv<'a> {
    pub tag_types: &'a HashMap<Intern<String>, Ty>,
    pub fn_return_types: &'a HashMap<Intern<String>, Ty>,
    pub locals: &'a dyn LocalTypes,
}

/// Each expression type implements this to know its own type.
pub trait TyInfer {
    fn infer_ty(&self, env: &TyInferEnv) -> Ty;
}

// ---------------------------------------------------------------------------
// Per-type implementations
// ---------------------------------------------------------------------------

impl TyInfer for Literal {
    fn infer_ty(&self, _env: &TyInferEnv) -> Ty {
        match self {
            Literal::Int(n) => Ty::Int {
                width: 64,
                signed: true,
                value: Some(*n as i128),
            },
            Literal::Number(n) => Ty::Int {
                width: 64,
                signed: true,
                value: Some(*n as i128),
            },
            Literal::Float(f) => Ty::Float { value: Some(*f) },
            Literal::String(_) => str_record_ty(),
        }
    }
}

impl TyInfer for Binary {
    fn infer_ty(&self, env: &TyInferEnv) -> Ty {
        if self.op.is_comparison() {
            return Ty::Bool;
        }
        let lhs_ty = self.lhs.infer_ty(env);
        let rhs_ty = self.rhs.infer_ty(env);
        match (&lhs_ty, &rhs_ty) {
            (Ty::Int { value: Some(a), .. }, Ty::Int { value: Some(b), .. }) => {
                let folded = match self.op {
                    BinOp::Add => Some(a + b),
                    BinOp::Subtract => Some(a - b),
                    BinOp::Multiply => Some(a * b),
                    BinOp::Divide if *b != 0 => Some(a / b),
                    BinOp::Modulo if *b != 0 => Some(a % b),
                    _ => None,
                };
                folded
                    .map(|v| Ty::Int {
                        width: 64,
                        signed: true,
                        value: Some(v),
                    })
                    .unwrap_or(lhs_ty)
            }
            (Ty::Float { value: Some(a) }, Ty::Float { value: Some(b) }) => {
                let folded = match self.op {
                    BinOp::Add => Some(a + b),
                    BinOp::Subtract => Some(a - b),
                    BinOp::Multiply => Some(a * b),
                    BinOp::Divide => Some(a / b),
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
            let mangled = crate::resolve::mangled_fn_call_name(self);
            return env
                .fn_return_types
                .get(&mangled)
                .cloned()
                .unwrap_or(Ty::Int {
                    width: 64,
                    signed: true,
                    value: None,
                });
        }

        if let Some(mut ty) = env.locals.get_type(&name) {
            for seg in &self.path.segments {
                ty = match &ty {
                    Ty::Ptr { inner } | Ty::Ref { inner } if inner.is_record() => {
                        match inner.as_ref() {
                            Ty::Record { fields, .. } => fields
                                .iter()
                                .find(|(fname, _)| fname.as_str() == seg.as_str())
                                .map(|(_, fty)| (**fty).clone())
                                .unwrap_or(ty),
                            _ => return ty,
                        }
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

        let mangled = crate::resolve::mangled_fn_call_name(self);
        env.fn_return_types
            .get(&mangled)
            .cloned()
            .unwrap_or(Ty::Int {
                width: 64,
                signed: true,
                value: None,
            })
    }
}

impl TyInfer for Bind {
    fn infer_ty(&self, env: &TyInferEnv) -> Ty {
        // Build the method-scoped type-variable map. For
        // `Range[x].new(start x, end x) Range[x]: ...`, this yields
        // `{ x -> Ty::Opaque(x) }` so each occurrence of `x` in params, body,
        // and return type resolves to the same opaque tag (and unifies trivially).
        let subst: HashMap<Intern<String>, Ty> = self
            .receiver_type_surface()
            .map(|sp| typevars_from_receiver(&sp.0))
            .unwrap_or_default();

        if let Some(sp) = &self.return_tag
            && is_type_surface(&sp.0)
        {
            return resolve_type_expr_with_subst(&sp.0, env.tag_types, &subst);
        }

        let mut locals: HashMap<Intern<String>, Ty> = match self.params().as_ref() {
            None => HashMap::new(),
            Some(params) => params
                .iter()
                .map(|(name, kind)| {
                    (
                        *name,
                        resolve_parameter_kind_with_subst(
                            *name,
                            kind,
                            env.tag_types,
                            env.fn_return_types,
                            &subst,
                        ),
                    )
                })
                .collect(),
        };
        if let Some(sp) = self.receiver_type_surface()
            && is_type_surface(&sp.0)
        {
            let recv_ty = resolve_type_expr_with_subst(&sp.0, env.tag_types, &subst);
            locals.insert(Intern::<String>::from_ref("self"), recv_ty);
        }

        let bind_env = TyInferEnv {
            tag_types: env.tag_types,
            fn_return_types: env.fn_return_types,
            locals: &locals,
        };
        match self.value() {
            BindValue::Expr(expr) => expr.infer_ty(&bind_env),
            BindValue::Body { ret, .. } => match &ret.0 {
                Some(expr) => expr.infer_ty(&bind_env),
                None => Ty::Unit,
            },
            BindValue::Extern => Ty::Unit,
        }
    }
}

impl TyInfer for TagCall {
    fn infer_ty(&self, env: &TyInferEnv) -> Ty {
        // Try union variant first.
        if let Some(ty) = env.tag_types.values().find_map(|ty| {
            if let Ty::Union { variants, .. } = ty
                && variants.iter().any(|(vname, _)| *vname == self.name)
            {
                return Some(ty.clone());
            }
            None
        }) {
            return ty;
        }
        // Fall back to a record type with that name.
        env.tag_types
            .get(&self.name)
            .cloned()
            .unwrap_or(Ty::Opaque(self.name))
    }
}

impl TyInfer for WhenExpr {
    fn infer_ty(&self, env: &TyInferEnv) -> Ty {
        // Prefer the else arm; fall back to the first arm's body.
        let body = self
            .arms
            .iter()
            .find_map(|a| {
                if let WhenArm::Else(b) = a {
                    Some(b.as_ref())
                } else {
                    None
                }
            })
            .or_else(|| {
                self.arms.first().map(|a| match a {
                    WhenArm::Cond { body, .. } | WhenArm::Is { body, .. } | WhenArm::Else(body) => {
                        body.as_ref()
                    }
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
            // Delegated to per-type impls
            Expr::Lit(lit) => lit.infer_ty(env),
            Expr::Binary(bin) => bin.infer_ty(env),
            Expr::FnCall(call) => call.infer_ty(env),
            Expr::Bind(bind) => bind.infer_ty(env),
            Expr::TagCall(tc) => tc.infer_ty(env),
            Expr::When(w) => w.infer_ty(env),

            // Simple inline arms
            Expr::AnonymousTag(name, _) => Ty::Opaque(*name),
            Expr::FormatString(_) => str_record_ty(),
            Expr::Loop(_) => Ty::Unit,
            Expr::If(_) => Ty::Unit,
            Expr::Asm(_) => Ty::Int {
                width: 64,
                signed: true,
                value: None,
            },
            Expr::Range(_) => Ty::Opaque(Intern::<String>::from_ref("Range")),
            Expr::TupleSet { .. } | Expr::BufSet { .. } => Ty::Unit,
            Expr::Cast { ty, .. } => Ty::Opaque(*ty),

            // Arms with recursive sub-expression inference
            Expr::SelfRef(_) => env
                .locals
                .get_type(&Intern::<String>::from_ref("self"))
                .unwrap_or_else(|| Ty::Opaque(Intern::<String>::from_ref("Self"))),

            Expr::TupleAlloc { init, size } => {
                let elem = init.infer_ty(env);
                Ty::Array {
                    elem: Box::new(elem),
                    size: *size,
                }
            }

            Expr::TupleGet { base, index } => match base.infer_ty(env) {
                Ty::Array { elem, .. } => *elem,
                Ty::Tuple(fields) => fields.into_iter().nth(*index).unwrap_or(Ty::Int {
                    width: 64,
                    signed: true,
                    value: None,
                }),
                _ => Ty::Int {
                    width: 8,
                    signed: false,
                    value: None,
                },
            },

            Expr::BufGet { buf, .. } => match buf.infer_ty(env) {
                Ty::Array { elem, .. } => *elem,
                _ => Ty::Int {
                    width: 8,
                    signed: false,
                    value: None,
                },
            },

            Expr::TakePtr(inner) => Ty::Ptr {
                inner: Box::new(inner.infer_ty(env)),
            },

            Expr::TakeRef(inner) => Ty::Ref {
                inner: Box::new(inner.infer_ty(env)),
            },

            Expr::Deref(inner) => match inner.infer_ty(env) {
                Ty::Ptr { inner } | Ty::Ref { inner } => *inner,
                _ => Ty::Int {
                    width: 64,
                    signed: true,
                    value: None,
                },
            },

            Expr::Negate(inner) => match inner.infer_ty(env) {
                Ty::Int {
                    value: Some(n),
                    width,
                    signed,
                } => Ty::Int {
                    width,
                    signed,
                    value: Some(-n),
                },
                Ty::Float { value: Some(f) } => Ty::Float { value: Some(-f) },
                other => other,
            },

            Expr::TupleLit(elems) => Ty::Tuple(elems.iter().map(|e| e.infer_ty(env)).collect()),

            // Only stored as `if` / `when` pattern payload or bind type position, not as a value.
            Expr::TypeNominal(..) | Expr::TypeQualified(_) | Expr::TypeGeneric { .. } => Ty::Unit,
        }
    }
}

impl TyInfer for Spanned<Expr> {
    fn infer_ty(&self, env: &TyInferEnv) -> Ty {
        self.0.infer_ty(env)
    }
}

/// Resolve a `ParameterKind` to a `Ty`, consulting a method-scoped
/// type-variable substitution map. Takes the parameter `name` so bare-id
/// (`Generic`) params can be resolved as a fresh `Ty::Opaque(name)`
/// type variable rather than the legacy `Int64` fallback.
///
/// This makes `start` and `end` in `CustomRange has (start, end)` (and the
/// matching `CustomRange.new(start, end) ...` method) into independent fresh
/// type variables — call sites can then bind each to any type without
/// rejecting mixed inputs (`CustomRange.new(1, "hi")`). For shared-type
/// parameters use `start x, end x` with an explicit type-variable name.
///
/// `subst` lets `start x` and `end x` in
/// `Range[x].new(start x, end x) Range[x]: ...` both resolve to the same
/// `Ty::Opaque(x)`. Pass an empty map for non-method binds.
pub(crate) fn resolve_parameter_kind_with_subst(
    name: Intern<String>,
    kind: &ParameterKind,
    tag_types: &HashMap<Intern<String>, Ty>,
    fn_return_types: &HashMap<Intern<String>, Ty>,
    subst: &HashMap<Intern<String>, Ty>,
) -> Ty {
    match kind {
        ParameterKind::Tagged(sp) => {
            if is_type_surface(&sp.0) {
                resolve_type_expr_with_subst(&sp.0, tag_types, subst)
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
            };
            expr.infer_ty(&env)
        }
    }
}
