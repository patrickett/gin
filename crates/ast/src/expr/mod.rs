use crate::span::SpanId;
use crate::GroupPath;
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
    /// For [`TyState::Infer`] returns `None`.
    pub fn resolved_ty(&self) -> Option<&Ty> {
        self.ty.resolved_ty()
    }

    /// Returns the current (possibly narrowed) type if resolved.
    ///
    /// For [`TyState::Resolved(ty)`] returns `Some(ty)`.
    /// For [`TyState::Narrowed { current, .. }`] returns `Some(current)`.
    /// For [`TyState::Infer`] returns `None`.
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
    /// Stack-allocate an array: `(init_expr; size_expr)` — emits
    /// `llvm.alloca N×sizeof(elem)`. `size_expr` must be compile-time-known.
    TupleAlloc {
        init: Box<Typed<Expr>>,
        size: Box<Typed<Expr>>,
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
        group: Option<GroupPath>,
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
    /// Record field write: `base.field: value`
    RecordSet {
        base: Box<Typed<Expr>>,
        field: internment::Intern<String>,
        value: Box<Typed<Expr>>,
    },
    /// Record literal: `(name: val, …)` — named fields, behaves as a record value.
    RecordLit(Vec<(Intern<String>, Typed<Expr>)>),
    /// Tuple literal: `(e1, e2, …)` — at least two elements.
    TupleLit(Vec<Typed<Expr>>),
    /// List literal: `[e1, e2, …]` — homogeneous compile-time list.
    ///
    /// TODO: Evolve into a general-purpose sequential type with automatic
    /// stack (Ty::Array) vs heap (vec) inference based on usage (mutation,
    /// runtime size, escaping scope). Flow analysis already tracks this.
    List(Vec<Typed<Expr>>),
    /// Destructure bind: `Tag(field: bind, …) := expr`
    Destructure {
        /// The tag/type name being destructured
        tag_name: internment::Intern<String>,
        /// (field_name, bind_name) pairs
        field_bindings: Vec<(internment::Intern<String>, internment::Intern<String>)>,
        /// The value being destructured
        value: Box<Typed<Expr>>,
    },
}
