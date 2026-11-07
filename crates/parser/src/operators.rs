//! Binary operator policy for the source parser, independent of the compiler parser.

use ast::BinaryOperator;
use lexer::TokenKind as Token;

pub(crate) fn binary_operator(token: Token) -> Option<BinaryOperator> {
    Some(match token {
        Token::Star => BinaryOperator::Multiply,
        Token::Slash => BinaryOperator::Divide,
        Token::Percent => BinaryOperator::Remainder,
        Token::Plus => BinaryOperator::Add,
        Token::Minus => BinaryOperator::Subtract,
        Token::ShiftLeft => BinaryOperator::ShiftLeft,
        Token::ShiftRight => BinaryOperator::ShiftRight,
        Token::Ampersand => BinaryOperator::BitAnd,
        Token::Caret => BinaryOperator::BitXor,
        Token::Pipe => BinaryOperator::BitOr,
        Token::Eq => BinaryOperator::Equal,
        Token::Less => BinaryOperator::Less,
        Token::LessEq => BinaryOperator::LessOrEqual,
        Token::Greater => BinaryOperator::Greater,
        Token::GreaterEq => BinaryOperator::GreaterOrEqual,
        Token::And => BinaryOperator::And,
        Token::Or => BinaryOperator::Or,
        _ => return None,
    })
}

/// Larger values bind more tightly. Equal precedence reduces left to right.
pub(crate) fn precedence(operator: BinaryOperator) -> u8 {
    match operator {
        BinaryOperator::Multiply | BinaryOperator::Divide | BinaryOperator::Remainder => 9,
        BinaryOperator::Add | BinaryOperator::Subtract => 8,
        BinaryOperator::ShiftLeft | BinaryOperator::ShiftRight => 7,
        BinaryOperator::BitAnd => 6,
        BinaryOperator::BitXor => 5,
        BinaryOperator::BitOr => 4,
        BinaryOperator::Equal
        | BinaryOperator::Less
        | BinaryOperator::LessOrEqual
        | BinaryOperator::Greater
        | BinaryOperator::GreaterOrEqual => 3,
        BinaryOperator::And => 2,
        BinaryOperator::Or => 1,
    }
}

pub(crate) fn is_comparison(operator: BinaryOperator) -> bool {
    matches!(
        operator,
        BinaryOperator::Equal
            | BinaryOperator::Less
            | BinaryOperator::LessOrEqual
            | BinaryOperator::Greater
            | BinaryOperator::GreaterOrEqual
    )
}
