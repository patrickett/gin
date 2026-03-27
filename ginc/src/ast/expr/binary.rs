use crate::codegen::prelude::*;

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
    Modulo,
    NotEqual,
    Equal,
    BitAnd,
    BitOr,
    BitXor,
    ShiftLeft,
    ShiftRight,
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

/// Parser for arithmetic operators (+, -, *, /, %)
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
        Percent => Modulo,
    }
}

/// Parser for bitwise operators (&, |, ^, <<, >>)
pub fn bitwise_op<'t, I>() -> impl Parser<'t, I, BinOp, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    select! {
        Token::Ampersand  => BinOp::BitAnd,
        Token::Pipe       => BinOp::BitOr,
        Token::Caret      => BinOp::BitXor,
        Token::ShiftLeft  => BinOp::ShiftLeft,
        Token::ShiftRight => BinOp::ShiftRight,
    }
}

impl<'c> Lower<'c> for Binary {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        let lhs = self.lhs.lower(ctx, block, symtab)?;
        let rhs = self.rhs.lower(ctx, block, symtab)?;

        let result_ty = lhs.r#type();
        let is_float = result_ty == ctx.mlir.f64();

        Some(match self.op {
            BinOp::Add => block.append_op(ctx.mlir.build_binop(
                if is_float {
                    ArithOps::ADDF
                } else {
                    ArithOps::ADD
                },
                lhs,
                rhs,
                result_ty,
            )),
            BinOp::Subtract => block.append_op(ctx.mlir.build_binop(
                if is_float {
                    ArithOps::SUBF
                } else {
                    ArithOps::SUB
                },
                lhs,
                rhs,
                result_ty,
            )),
            BinOp::Multiply => block.append_op(ctx.mlir.build_binop(
                if is_float {
                    ArithOps::MULF
                } else {
                    ArithOps::MUL
                },
                lhs,
                rhs,
                result_ty,
            )),
            BinOp::Divide => block.append_op(ctx.mlir.build_binop(
                if is_float {
                    ArithOps::DIVF
                } else {
                    ArithOps::DIV
                },
                lhs,
                rhs,
                result_ty,
            )),
            BinOp::Modulo => block.append_op(ctx.mlir.build_binop(
                if is_float {
                    ArithOps::REMF
                } else {
                    ArithOps::REM
                },
                lhs,
                rhs,
                result_ty,
            )),
            BinOp::Equal => block.append_op(if is_float {
                ctx.mlir.build_cmpf(FPredicates::OEQ, lhs, rhs)
            } else {
                ctx.mlir.build_cmpi(Predicates::EQ, lhs, rhs)
            }),
            BinOp::NotEqual => block.append_op(if is_float {
                ctx.mlir.build_cmpf(FPredicates::ONE, lhs, rhs)
            } else {
                ctx.mlir.build_cmpi(Predicates::NE, lhs, rhs)
            }),
            BinOp::LessThan => block.append_op(if is_float {
                ctx.mlir.build_cmpf(FPredicates::OLT, lhs, rhs)
            } else {
                ctx.mlir.build_cmpi(Predicates::SLT, lhs, rhs)
            }),
            BinOp::GreaterThan => block.append_op(if is_float {
                ctx.mlir.build_cmpf(FPredicates::OGT, lhs, rhs)
            } else {
                ctx.mlir.build_cmpi(Predicates::SGT, lhs, rhs)
            }),
            BinOp::LessThanOrEqual => block.append_op(if is_float {
                ctx.mlir.build_cmpf(FPredicates::OLE, lhs, rhs)
            } else {
                ctx.mlir.build_cmpi(Predicates::SLE, lhs, rhs)
            }),
            BinOp::GreaterThanOrEqual => block.append_op(if is_float {
                ctx.mlir.build_cmpf(FPredicates::OGE, lhs, rhs)
            } else {
                ctx.mlir.build_cmpi(Predicates::SGE, lhs, rhs)
            }),
            BinOp::BitAnd => {
                block.append_op(ctx.mlir.build_binop(ArithOps::ANDI, lhs, rhs, result_ty))
            }
            BinOp::BitOr => {
                block.append_op(ctx.mlir.build_binop(ArithOps::ORI, lhs, rhs, result_ty))
            }
            BinOp::BitXor => {
                block.append_op(ctx.mlir.build_binop(ArithOps::XORI, lhs, rhs, result_ty))
            }
            BinOp::ShiftLeft => {
                block.append_op(ctx.mlir.build_binop(ArithOps::SHLI, lhs, rhs, result_ty))
            }
            BinOp::ShiftRight => {
                block.append_op(ctx.mlir.build_binop(ArithOps::SHRI, lhs, rhs, result_ty))
            }
        })
    }
}
