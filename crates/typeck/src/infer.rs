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
//! - [`resolve_tag_from_map`] resolves a `Tag` AST node against a type map.
//!
//! For environment building, type checking, and diagnostics, see `type.rs`.

use std::collections::HashMap;

use ast::{
    BinOp, Binary, Bind, BindValue, Expr, FnCall, Literal, ParameterKind, Spanned, Tag, TagCall,
    WhenArm, WhenExpr, type_tag_as_tag,
};
use internment::Intern;

use crate::{Ty, str_record_ty};

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
            let mangled = crate::mangled_fn_call_name(self);
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

        let mangled = crate::mangled_fn_call_name(self);
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
        if let Some(sp) = &self.return_tag {
            if let Some(tag) = type_tag_as_tag(&sp.0) {
                return resolve_tag_from_map(tag, env.tag_types);
            }
        }

        let mut locals: HashMap<Intern<String>, Ty> = match self.params().as_ref() {
            None => HashMap::new(),
            Some(params) => params
                .iter()
                .map(|(name, kind)| {
                    (
                        *name,
                        resolve_parameter_kind_with(kind, env.tag_types, env.fn_return_types),
                    )
                })
                .collect(),
        };
        if let Some(recv_tag) = self.receiver_type() {
            let recv_ty = resolve_tag_from_map(recv_tag, env.tag_types);
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
            Expr::IsPattern(_) | Expr::TypeTag(_) => Ty::Unit,
        }
    }
}

impl TyInfer for Spanned<Expr> {
    fn infer_ty(&self, env: &TyInferEnv) -> Ty {
        self.0.infer_ty(env)
    }
}

/// Resolve a `Tag` to a concrete `Ty` using only the pre-built `tag_types` map.
pub(crate) fn resolve_tag_from_map(tag: &Tag, tag_types: &HashMap<Intern<String>, Ty>) -> Ty {
    match tag {
        Tag::Nominal(name, _) => tag_types.get(name).cloned().unwrap_or(Ty::Int {
            width: 64,
            signed: true,
            value: None,
        }),
        Tag::Generic(name, params, _) => match name.as_str() {
            "Ptr" | "Ref" => {
                let inner = params
                    .values()
                    .find_map(|kind| match kind {
                        ParameterKind::Tagged(t) => Some(resolve_tag_from_map(t, tag_types)),
                        _ => None,
                    })
                    .unwrap_or(Ty::Opaque(*name));
                if name.as_str() == "Ptr" {
                    Ty::Ptr {
                        inner: Box::new(inner),
                    }
                } else {
                    Ty::Ref {
                        inner: Box::new(inner),
                    }
                }
            }
            _ => tag_types.get(name).cloned().unwrap_or(Ty::Opaque(*name)),
        },
        Tag::Qualified(path) => {
            // For qualified types like Bool.True, we need to resolve them
            // to the type of the variant. The type of a variant is the union type itself.
            // E.g., Bool.True has type Bool (the union type)
            let union_name = path.root;
            tag_types
                .get(&union_name)
                .cloned()
                .unwrap_or(Ty::Opaque(union_name))
        }
    }
}

/// Resolve a `ParameterKind` to a `Ty` for use in standalone functions.
///
/// This is the non-method version of `TyEnv::resolve_parameter_kind`,
/// used when we don't have a `TyEnv` available but have the raw maps.
fn resolve_parameter_kind_with(
    kind: &ParameterKind,
    tag_types: &HashMap<Intern<String>, Ty>,
    fn_return_types: &HashMap<Intern<String>, Ty>,
) -> Ty {
    match kind {
        ParameterKind::Tagged(tag) => resolve_tag_from_map(tag, tag_types),
        ParameterKind::Generic => Ty::Int {
            width: 64,
            signed: true,
            value: None,
        },
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
