use crate::Expr;
use crate::Spanned;
use crate::span::SpanId;
use internment::Intern;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AsmExpr {
    /// The assembly template string (e.g. "svc #0x80")
    pub template: Intern<String>,
    /// Typed constraint expressions — values of the `Constraint` union type.
    /// Each constraint describes how a register participates in the assembly block,
    /// e.g. `Output[X0]`, `Input[X16]`, `ClobberMemory`.
    pub constraints: Vec<Spanned<Expr>>,
    /// Input operand expressions
    pub operands: Vec<Spanned<Expr>>,
    /// Source span
    pub span: SpanId,
}
