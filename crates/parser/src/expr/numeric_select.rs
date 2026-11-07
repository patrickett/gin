//! Projection context disambiguates raw `Float("0.1")` in `base.0.1`.
//! Raw lexer tokens and ordinary float expressions remain unchanged.

use ast::{NumericSelectExpression, Span};

use super::*;

impl Cursor<'_, '_> {
    pub(super) fn parse_numeric_select(&mut self, frame: &mut Frame) -> ParseResult<()> {
        assert!(!frame.needs_operand());
        assert_eq!(self.current(), Some(Token::Dot));
        let dot = self.advance()?.span_id;
        let token = self.advance()?;
        let span = self.spans.get(token.span_id);
        let spelling = span.extract(self.source);
        if token.value == Token::Int {
            return self.numeric_select_value(frame, dot, token.span_id);
        }
        assert_eq!(token.value, Token::Float);
        // The scanner admits exactly one decimal point in a float token.
        let offset = spelling.find('.').expect("float contains a decimal point");
        let middle = span.start() + offset;
        let first = self.spans.insert(Span::from_range(span.start()..middle));
        let second_dot = self.spans.insert(Span::from_range(middle..middle + 1));
        let second = self.spans.insert(Span::from_range(middle + 1..span.end()));
        self.numeric_select_value(frame, dot, first)?;
        self.numeric_select_value(frame, second_dot, second)
    }

    fn numeric_select_value(
        &mut self,
        frame: &mut Frame,
        dot: SpanId,
        index: SpanId,
    ) -> ParseResult<()> {
        let base = frame.values.pop().expect("numeric selection base");
        let depth = base.depth + 1;
        self.check_expression_depth(depth)?;
        let start = base.expression.span_id;
        let expression = Expr::NumericSelect(Box::new(NumericSelectExpression {
            base: base.expression,
            dot,
            index: Spanned::new(Expr::Literal(LiteralKind::Integer), index),
        }));
        frame.values.push(Value {
            expression: self.node(expression, start, index),
            depth,
        });
        assert!(!frame.needs_operand());
        Ok(())
    }
}
