use crate::parameter::ParameterKind;
use crate::path::ModPath;
use crate::span::SpanId;
use internment::Intern;

use crate::span::Spanned;

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
    SelfRef(SpanId),
    /// A capitalized variant constructor with arguments, e.g. `Some(5)`.
    TagCall(TagCall),
    /// A bare capitalized tag in expression position, e.g. `None`, `True`.
    AnonymousTag(Intern<String>, SpanId),
    /// Type position: bare `Tag` (e.g. `Str` in `(x Str)`).
    TypeNominal(Intern<String>, SpanId),
    /// Type position: qualified path `Tag.Tag…`.
    TypeQualified(Spanned<ModPath>),
    /// Type position: `Tag(...)` with generic / named parameters.
    /// Stored as a vector (declaration order) so [`Hash`] can be derived despite the
    /// `ParameterKind` ↔ `Expr` recursion.
    TypeGeneric {
        name: Intern<String>,
        params: Vec<(Intern<String>, ParameterKind)>,
        span: SpanId,
    },
    /// Stack-allocate an array: `(init_expr; N)` — emits `llvm.alloca N×sizeof(elem)`.
    TupleAlloc {
        init: Box<Spanned<Expr>>,
        size: usize,
    },
    /// Positional element read: `arr.N` — emits GEP + load.
    TupleGet {
        base: Box<Spanned<Expr>>,
        index: usize,
    },
    /// Positional element write: `arr.N: val` — emits GEP + store.
    TupleSet {
        base: Box<Spanned<Expr>>,
        index: usize,
        value: Box<Spanned<Expr>>,
    },
    /// Explicit numeric cast: `expr as Type` — emits trunci/extsi/sitofp/fptosi.
    Cast {
        expr: Box<Spanned<Expr>>,
        ty: Intern<String>,
    },
    /// Dynamic buffer element read: `buf.(i)` — emits GEP(i * elem_bytes) + load.
    BufGet {
        buf: Box<Spanned<Expr>>,
        index: Box<Spanned<Expr>>,
    },
    /// Dynamic buffer element write: `buf.(i): val` — emits GEP(i * elem_bytes) + store.
    BufSet {
        buf: Box<Spanned<Expr>>,
        index: Box<Spanned<Expr>>,
        value: Box<Spanned<Expr>>,
    },
    /// Take a raw pointer to a value: `@expr` — emits alloca + spill if needed, returns `!llvm.ptr`.
    TakePtr(Box<Spanned<Expr>>),
    /// Take a reference to a value: `^expr` — same layout as TakePtr for now.
    TakeRef(Box<Spanned<Expr>>),
    /// Dereference a pointer or reference: `*expr` — emits `llvm.load` of the pointed-to value.
    Deref(Box<Spanned<Expr>>),
    /// Unary negation: `-expr`.
    Negate(Box<Spanned<Expr>>),
    /// Inline assembly: `asm("template", "constraints", args...)`
    Asm(AsmExpr),
    /// Tuple literal: `(e1, e2, …)` — at least two elements.
    TupleLit(Vec<Spanned<Expr>>),
    /// List literal: `[e1, e2, …]` — homogeneous compile-time list.
    List(Vec<Spanned<Expr>>),
}

impl From<crate::TypeExpr> for Expr {
    fn from(te: crate::TypeExpr) -> Self {
        match te {
            crate::TypeExpr::Nominal(name, span) => Expr::TypeNominal(name, span),
            crate::TypeExpr::Qualified(path) => Expr::TypeQualified(path),
            crate::TypeExpr::Generic { name, params, span } => {
                Expr::TypeGeneric { name, params, span }
            }
            crate::TypeExpr::Literal(..) => Expr::Lit(crate::Literal::Number(0)),
        }
    }
}

impl Expr {
    /// If this expression is a type-position variant, return the equivalent [`TypeExpr`].
    pub fn as_type_expr(&self) -> Option<crate::TypeExpr> {
        match self {
            Expr::TypeNominal(name, span) => Some(crate::TypeExpr::Nominal(*name, *span)),
            Expr::TypeQualified(path) => Some(crate::TypeExpr::Qualified(path.clone())),
            Expr::TypeGeneric { name, params, span } => Some(crate::TypeExpr::Generic {
                name: *name,
                params: params.clone(),
                span: *span,
            }),
            _ => None,
        }
    }
}
