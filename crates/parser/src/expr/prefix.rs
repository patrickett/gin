use ast::{Expr, GroupPath, PrefixedType, Spanned, TypePrefixKind, UnaryOperator};
use lexer::TokenKind as Token;

use super::{Frame, Value};
use crate::cursor::{Cursor, ParseResult};

pub(super) enum Prefix {
    Unary(Spanned<UnaryOperator>),
    Type {
        keyword: Spanned<TypePrefixKind>,
        group: Option<Spanned<GroupPath>>,
    },
}

impl Frame {
    pub(super) fn finish_prefixes(&mut self, cursor: &mut Cursor<'_, '_>) -> ParseResult<()> {
        if self.prefixes.is_empty() {
            return Ok(());
        }
        assert!(!self.needs_operand());
        let mut value = self.values.pop().expect("a prefix has an operand");
        for _ in 0..self.prefixes.len() {
            let prefix = self.prefixes.pop().expect("pending prefix");
            let depth = value.depth + 1;
            cursor.check_expression_depth(depth)?;
            let end = value.expression.span_id;
            let expression = match prefix {
                Prefix::Unary(operator) => {
                    let start = operator.span_id;
                    cursor.node(
                        Expr::Unary {
                            operator,
                            operand: Box::new(value.expression),
                        },
                        start,
                        end,
                    )
                }
                Prefix::Type { keyword, group } => {
                    let start = keyword.span_id;
                    cursor.node(
                        Expr::PrefixedType(PrefixedType {
                            keyword,
                            group,
                            target: Box::new(value.expression),
                        }),
                        start,
                        end,
                    )
                }
            };
            value = Value { expression, depth };
        }
        assert!(self.prefixes.is_empty());
        self.values.push(value);
        Ok(())
    }
}

impl Cursor<'_, '_> {
    pub(super) fn parse_unary_prefix(&mut self, frame: &mut Frame) -> ParseResult<()> {
        assert!(frame.needs_operand());
        let depth = u16::try_from(frame.prefixes.len()).expect("bounded prefix depth") + 2;
        self.check_expression_depth(depth)?;
        let (which, span) = match self.current() {
            Some(Token::Deref) => (UnaryOperator::Deref, self.advance()?.span_id),
            // `domain of` is one operator written as two words.
            Some(Token::Domain) => {
                let domain = self.advance()?.span_id;
                let of = self.advance()?.span_id;
                (UnaryOperator::DomainOf, self.spans.merge(domain, of))
            }
            _ => (UnaryOperator::Negate, self.advance()?.span_id),
        };
        let operator = Spanned::new(which, span);
        frame.prefixes.push(Prefix::Unary(operator));
        assert!(frame.prefixes.len() < usize::from(self.options.limits.expression_depth_max));
        Ok(())
    }

    pub(super) fn parse_type_prefix(&mut self, frame: &mut Frame) -> ParseResult<()> {
        assert!(frame.needs_operand());
        let kind = match self.current() {
            Some(Token::Ref) => TypePrefixKind::Ref,
            Some(Token::Mut) => TypePrefixKind::Mut,
            Some(Token::Own) => TypePrefixKind::Own,
            Some(Token::At) => TypePrefixKind::Pointer,
            _ => unreachable!("type prefix dispatch"),
        };
        let depth = u16::try_from(frame.prefixes.len()).expect("bounded prefix depth") + 2;
        self.check_expression_depth(depth)?;
        let keyword = Spanned::new(kind, self.advance()?.span_id);
        if kind == TypePrefixKind::Own && self.current() == Some(Token::CurlyOpen) {
            self.report("group paths attach to ref or mut, not own")?;
        }
        if kind == TypePrefixKind::Pointer && self.current() == Some(Token::CurlyOpen) {
            self.report("group paths attach to ref or mut, not @")?;
        }
        let group = self.parse_optional_group()?;
        frame.prefixes.push(Prefix::Type { keyword, group });
        Ok(())
    }
}
