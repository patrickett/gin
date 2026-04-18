use crate::Expr;
use crate::Spanned;
use crate::span::SpanId;
use internment::Intern;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AsmExpr {
    /// The assembly template string (e.g. "svc #0x80")
    pub template: Intern<String>,
    /// The LLVM-style constraint string (e.g. "={x0},{x16},0,{x1},~{memory}")
    pub constraints: Intern<String>,
    /// Input operand expressions
    pub operands: Vec<Spanned<Expr>>,
    /// Source span
    pub span: SpanId,
}
