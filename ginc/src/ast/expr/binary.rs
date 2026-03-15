use crate::codegen::prelude::*;
use crate::diagnostic::codegen::CodegenSymptom;
use crate::prelude::*;
use chumsky::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Binary {
    pub lhs: Box<Expr>,
    pub op: BinOp,
    pub rhs: Box<Expr>,
}

impl Binary {
    pub fn new(lhs: Expr, op: BinOp, rhs: Expr) -> Self {
        let lhs = Box::new(lhs);
        let rhs = Box::new(rhs);
        Self { lhs, op, rhs }
    }
}

/// Binary operations are defined as `lhs op rhs`
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BinOp {
    LessThanOrEqual,
    GreaterThanOrEqual,
    LessThan,
    GreaterThan,
    Add,
    Divide,
    Multiply,
    Subtract,
    NotEqual,
    Equal,
}

impl BinOp {
    pub fn is_comparison(&self) -> bool {
        matches!(
            self,
            BinOp::Equal
                | BinOp::NotEqual
                | BinOp::LessThan
                | BinOp::LessThanOrEqual
                | BinOp::GreaterThan
                | BinOp::GreaterThanOrEqual
        )
    }
}

/// Parser for comparison operators (==, !=, <, >, <=, >=)
pub fn comparison_op<'t, I>() -> impl Parser<'t, I, BinOp, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    use BinOp::*;
    use Token::*;

    select! {
        Eq => Equal,
        NotEq => NotEqual,
        Less => LessThan,
        Greater => GreaterThan,
        LessEq => LessThanOrEqual,
        GreaterEq => GreaterThanOrEqual,
    }
}

/// Parser for arithmetic operators (+, -, *, /)
pub fn arithmetic_op<'t, I>() -> impl Parser<'t, I, BinOp, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    use BinOp::*;
    use Token::*;

    select! {
        Plus => Add,
        Minus => Subtract,
        Star => Multiply,
        Slash => Divide,
    }
}

impl<'c> Lower<'c> for Binary {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Result<Value<'c, 'c>, CodegenSymptom> {
        let lhs = self.lhs.lower(ctx, block, symtab)?;
        let rhs = self.rhs.lower(ctx, block, symtab)?;

        Ok(match self.op {
            BinOp::Add => block.append_op(ctx.mlir.build_binop(ArithOps::ADD, lhs, rhs)),
            BinOp::Subtract => block.append_op(ctx.mlir.build_binop(ArithOps::SUB, lhs, rhs)),
            BinOp::Multiply => block.append_op(ctx.mlir.build_binop(ArithOps::MUL, lhs, rhs)),
            BinOp::Divide => block.append_op(ctx.mlir.build_binop(ArithOps::DIV, lhs, rhs)),
            BinOp::Equal => block.append_op(ctx.mlir.build_cmpi(Predicates::EQ, lhs, rhs)),
            BinOp::NotEqual => block.append_op(ctx.mlir.build_cmpi(Predicates::NE, lhs, rhs)),
            BinOp::LessThan => block.append_op(ctx.mlir.build_cmpi(Predicates::SLT, lhs, rhs)),
            BinOp::GreaterThan => block.append_op(ctx.mlir.build_cmpi(Predicates::SGT, lhs, rhs)),
            BinOp::LessThanOrEqual => {
                block.append_op(ctx.mlir.build_cmpi(Predicates::SLE, lhs, rhs))
            }
            BinOp::GreaterThanOrEqual => {
                block.append_op(ctx.mlir.build_cmpi(Predicates::SGE, lhs, rhs))
            }
        })
    }
}
