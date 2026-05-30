use crate::parameter::ParameterKind;
use crate::path::ModPath;
use crate::span::SpanId;
use crate::ty::Ty;
use crate::ty_state::TyState;
use internment::Intern;

use std::ops::{Deref, DerefMut};

use crate::span::Spanned;

/// A typed AST node — pairs an inner expression `T` with its resolved type,
/// optional compile-time constant value, and source span.
///
/// This is the post-typecheck representation of an expression. Before
/// typechecking, the `ty` field is [`TyState::Infer`]; after typechecking it
/// is [`TyState::Resolved`] or [`TyState::Narrowed`].
///
/// `Typed<T>` subsumes [`Spanned<T>`]: it carries a span *and* type information.
/// Conversions between the two are provided via [`From`] impls.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Typed<T> {
    /// The inner AST node.
    pub value: T,
    /// The resolved (or inferred) type of this expression.
    ///
    /// * [`TyState::Infer`] — not yet typechecked.
    /// * [`TyState::Explicit`] — user wrote a type annotation.
    /// * [`TyState::Resolved`] — concrete type after inference.
    /// * [`TyState::Narrowed`] — control-flow-refined type (e.g., inside
    ///   `if x is Some(v)` the type is narrowed to the `Some` variant).
    pub ty: TyState,
    /// Compile-time constant value, when this expression can be folded to a
    /// known value (e.g., a literal, or a constant-folded binary op).
    pub const_value: Option<crate::ConstValue>,
    /// Source location for diagnostics and LSP.
    pub span_id: SpanId,
}

impl<T> Typed<T> {
    /// Create a new typed node with a given inner node, type, and span.
    pub fn new(value: T, ty: TyState, span_id: SpanId) -> Self {
        Self {
            value,
            ty,
            const_value: None,
            span_id,
        }
    }

    /// Create a new typed node with a known constant value.
    pub fn with_value(value: T, ty: TyState, cv: crate::ConstValue, span_id: SpanId) -> Self {
        Self {
            value,
            ty,
            const_value: Some(cv),
            span_id,
        }
    }

    /// Create a typed node in the [`TyState::Infer`] state (pre-typecheck).
    /// This is the typical starting state for parser output.
    pub fn infer(value: T, span_id: SpanId) -> Self {
        Self {
            value,
            ty: TyState::Infer,
            const_value: None,
            span_id,
        }
    }

    /// Create a typed node with a resolved concrete type.
    pub fn resolved(value: T, ty: Ty, span_id: SpanId) -> Self {
        Self {
            value,
            ty: TyState::Resolved(ty),
            const_value: None,
            span_id,
        }
    }

    /// Create a typed node with a narrowed type (control-flow refinement).
    /// `current` is the narrowed type; `original` is the type before narrowing.
    pub fn narrowed(value: T, current: Ty, original: Ty, span_id: SpanId) -> Self {
        Self {
            value,
            ty: TyState::Narrowed {
                current,
                original: Box::new(original),
            },
            const_value: None,
            span_id,
        }
    }

    /// Returns `true` if this node has been type-resolved (not [`TyState::Infer`]).
    pub fn is_resolved(&self) -> bool {
        !matches!(self.ty, TyState::Infer)
    }

    /// Returns the concrete type if resolved, stripping any narrowing.
    ///
    /// For [`TyState::Resolved(ty)`] returns `Some(ty)`.
    /// For [`TyState::Narrowed { original, .. }`] returns `Some(original)`.
    /// For [`TyState::Infer`] or [`TyState::Explicit`] returns `None`.
    pub fn resolved_ty(&self) -> Option<&Ty> {
        self.ty.resolved_ty()
    }

    /// Returns the current (possibly narrowed) type if resolved.
    ///
    /// For [`TyState::Resolved(ty)`] returns `Some(ty)`.
    /// For [`TyState::Narrowed { current, .. }`] returns `Some(current)`.
    /// For [`TyState::Infer`] or [`TyState::Explicit`] returns `None`.
    pub fn current_ty(&self) -> Option<&Ty> {
        self.ty.current_ty()
    }

    /// Map the inner node from `T` to `U`, preserving type info and span.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Typed<U> {
        Typed {
            value: f(self.value),
            ty: self.ty,
            const_value: self.const_value,
            span_id: self.span_id,
        }
    }

    /// Convert to a [`Spanned<T>`], discarding type information.
    pub fn into_spanned(self) -> Spanned<T> {
        Spanned {
            value: self.value,
            span_id: self.span_id,
        }
    }
}

impl<T> Deref for Typed<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T> DerefMut for Typed<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

impl<T> From<Typed<T>> for Spanned<T> {
    fn from(typed: Typed<T>) -> Self {
        typed.into_spanned()
    }
}

impl<T> From<Spanned<T>> for Typed<T> {
    /// Convert a spanned node into a typed node in the [`TyState::Infer`] state.
    fn from(spanned: Spanned<T>) -> Self {
        Typed::infer(spanned.value, spanned.span_id)
    }
}

impl<T> crate::span::HasSpanId for Typed<T> {
    fn span_id(&self) -> SpanId {
        self.span_id
    }
}

// TODO: Closure capture syntax and lambda expressions
//   1. Procedure calls — add labels + code forms (jump to known labels with args on stack).
//   2. Closures — close over free variables by storing them alongside the code pointer.
//      Requires free-variable analysis to annotate each lambda with its captured set.
//   3. Proper tail calls — use jmp instead of call in tail position for constant-space recursion.
// The Expr enum should eventually include a Lambda/Closure variant.

mod bind;
pub use bind::*;
mod asm;
pub use asm::*;
pub mod format_string;
pub use format_string::*;
pub mod literal;
pub use literal::*;
mod import;
pub use import::*;
mod fn_call;
pub use fn_call::*;
mod tag_call;
pub use tag_call::*;
mod binary;
pub use binary::*;
pub mod r#loop;
pub use r#loop::{Loop as LoopEnum, *};
pub mod r#if;
pub use r#if::*;
pub mod range;
pub use range::*;
pub mod r#return;
pub use r#return::*;
pub mod when;
pub use when::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Expr {
    Loop(Loop),
    Binary(Binary),
    FnCall(FnCall),
    Lit(Literal),
    FormatString(FormatString),
    Range(Range),
    Bind(Box<Bind>),
    When(WhenExpr),
    If(IfExpr),
    SelfRef,
    /// A capitalized variant constructor with arguments, e.g. `Some(5)`.
    TagCall(TagCall),
    /// A bare capitalized tag in expression position, e.g. `None`, `True`.
    AnonymousTag(Intern<String>),
    /// Type position: bare `Tag` (e.g. `Str` in `(x Str)`).
    TypeNominal(Intern<String>),
    /// Type position: `in 0...10` or `in WeekRange`.
    TypeInRange(crate::InRangeBounds),
    /// Type position: qualified path `Tag.Tag…`.
    TypeQualified(Spanned<ModPath>),
    /// Type position: `Tag(...)` with generic / named parameters.
    /// Stored as a vector (declaration order) so [`Hash`] can be derived despite the
    /// `ParameterKind` ↔ `Expr` recursion.
    TypeGeneric {
        name: Intern<String>,
        params: Vec<(Intern<String>, ParameterKind)>,
    },
    /// Type position: `ref T` or `mut T`.
    TypeRef {
        inner: Box<Expr>,
        mutable: bool,
    },
    /// Stack-allocate an array: `(init_expr; N)` — emits `llvm.alloca N×sizeof(elem)`.
    TupleAlloc {
        init: Box<Typed<Expr>>,
        size: usize,
    },
    /// Positional element read: `arr.N` — emits GEP + load.
    TupleGet {
        base: Box<Typed<Expr>>,
        index: usize,
    },
    /// Positional element write: `arr.N: val` — emits GEP + store.
    TupleSet {
        base: Box<Typed<Expr>>,
        index: usize,
        value: Box<Typed<Expr>>,
    },
    /// Explicit numeric cast: `expr as Type` — emits trunci/extsi/sitofp/fptosi.
    Cast {
        expr: Box<Typed<Expr>>,
        ty: Intern<String>,
    },
    /// Dynamic buffer element read: `buf.(i)` — emits GEP(i * elem_bytes) + load.
    BufGet {
        buf: Box<Typed<Expr>>,
        index: Box<Typed<Expr>>,
    },
    /// Dynamic buffer element write: `buf.(i): val` — emits GEP(i * elem_bytes) + store.
    BufSet {
        buf: Box<Typed<Expr>>,
        index: Box<Typed<Expr>>,
        value: Box<Typed<Expr>>,
    },
    /// Take a raw pointer to a value: `@expr` — emits alloca + spill if needed, returns `!llvm.ptr`.
    TakePtr(Box<Typed<Expr>>),
    /// Take a safe reference: `ref expr` or `mut expr`.
    /// Does NOT consume `expr`. The original value stays Alive.
    /// Invalidated when the source is consumed.
    Ref {
        inner: Box<Typed<Expr>>,
        mutable: bool,
    },
    /// Dereference a pointer or reference: `deref expr`.
    Deref(Box<Typed<Expr>>),
    /// Unary negation: `-expr`.
    Negate(Box<Typed<Expr>>),

    /// Inline assembly: `asm("template", "constraints", args...)"
    Asm(AsmExpr),
    /// Argument passed with `eat` at call site: `eat expr` — explicit consume.
    ConsumeArg(Box<Typed<Expr>>),
    /// Explicit consume: `eat expr`. Used standalone, not at call site.
    Eat(Box<Typed<Expr>>),
    /// Record field read: `base.field` (compile-time when `base` is a const record).
    RecordGet {
        base: Box<Typed<Expr>>,
        field: internment::Intern<String>,
    },
    /// Tuple literal: `(e1, e2, …)` — at least two elements.
    TupleLit(Vec<Typed<Expr>>),
    /// List literal: `[e1, e2, …]` — homogeneous compile-time list.
    List(Vec<Typed<Expr>>),
}

impl From<crate::TypeExpr> for Expr {
    fn from(te: crate::TypeExpr) -> Self {
        match te {
            crate::TypeExpr::Nominal(name, _span) => Expr::TypeNominal(name),
            crate::TypeExpr::Qualified(path) => Expr::TypeQualified(path),
            crate::TypeExpr::Generic { name, params, .. } => Expr::TypeGeneric { name, params },
            crate::TypeExpr::Ref { inner, mutable } => Expr::TypeRef {
                inner: Box::new(Expr::TypeNominal(Intern::<String>::from_ref(
                    crate::type_surface_mangle_name(&inner.value),
                ))),
                mutable,
            },
            crate::TypeExpr::Literal(..) => Expr::Lit(crate::Literal::Number(0)),
            crate::TypeExpr::InRange { bounds, .. } => Expr::TypeInRange(bounds),
            crate::TypeExpr::Pointer(_)
            | crate::TypeExpr::Unit
            | crate::TypeExpr::ListEmpty
            | crate::TypeExpr::ListCons { .. }
            | crate::TypeExpr::Tuple(_) => Expr::Lit(crate::Literal::Number(0)),
        }
    }
}

impl Expr {
    /// If this expression is a type-position variant, return the equivalent [`TypeExpr`].
    ///
    /// The returned [`TypeExpr`] carries [`SpanId::INVALID`] for leaf variants since the
    /// span is available from the enclosing [`Spanned`](crate::Spanned) or
    /// [`Typed`] wrapper. Prefer calling this on the wrapper and using its [`SpanId`]
    /// when a real span is needed.
    pub fn as_type_expr(&self) -> Option<crate::TypeExpr> {
        match self {
            Expr::TypeNominal(name) => Some(crate::TypeExpr::Nominal(*name, SpanId::INVALID)),
            Expr::TypeQualified(path) => Some(crate::TypeExpr::Qualified(path.clone())),
            Expr::TypeGeneric { name, params } => Some(crate::TypeExpr::Generic {
                name: *name,
                params: params.clone(),
                param_spans: Vec::new(),
                span: SpanId::INVALID,
            }),
            Expr::TypeRef { inner, mutable } => {
                if let Expr::TypeNominal(name) = inner.as_ref() {
                    Some(crate::TypeExpr::Ref {
                        inner: Box::new(crate::span::Spanned {
                            value: crate::TypeExpr::Nominal(*name, SpanId::INVALID),
                            span_id: SpanId::INVALID,
                        }),
                        mutable: *mutable,
                    })
                } else {
                    None
                }
            }
            Expr::TypeInRange(bounds) => Some(crate::TypeExpr::InRange {
                bounds: bounds.clone(),
                span: SpanId::INVALID,
            }),
            _ => None,
        }
    }
}
