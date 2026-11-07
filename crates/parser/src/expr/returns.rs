//! Return lists assemble the same sequence expression as parenthesized syntax.

use ast::{SequenceDelimiters, SequenceExpression};

use super::*;

#[derive(Default)]
pub(crate) struct PendingReturnValue {
    elements: Vec<Expression>,
    separators: Vec<SpanId>,
    depth: u16,
}

impl PendingReturnValue {
    pub(crate) fn push(&mut self, value: Value) {
        assert_eq!(self.elements.len(), self.separators.len());
        self.depth = self.depth.max(value.depth);
        self.elements.push(value.expression);
        assert_eq!(self.elements.len(), self.separators.len() + 1);
    }

    pub(crate) fn comma(&mut self, span: SpanId) {
        assert_eq!(self.elements.len(), self.separators.len() + 1);
        self.separators.push(span);
        assert_eq!(self.elements.len(), self.separators.len());
    }

    pub(crate) fn finish(self, cursor: &mut Cursor<'_, '_>) -> ParseResult<Option<Value>> {
        let mut elements = self.elements;
        if elements.is_empty() {
            assert!(self.separators.is_empty());
            return Ok(None);
        }
        assert_eq!(elements.len(), self.separators.len() + 1);
        if self.separators.is_empty() {
            return Ok(Some(Value {
                expression: elements.pop().expect("single return expression"),
                depth: self.depth,
            }));
        }
        let depth = self.depth + 1;
        cursor.check_expression_depth(depth)?;
        let start = elements.first().expect("sequence start").span_id;
        let end = elements.last().expect("sequence end").span_id;
        let expression = Expr::Sequence(Box::new(SequenceExpression {
            delimiters: SequenceDelimiters::Bare,
            elements,
            separators: self.separators,
        }));
        Ok(Some(Value {
            expression: cursor.node(expression, start, end),
            depth,
        }))
    }
}
