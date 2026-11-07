//! Collection delimiters use the same bounded continuation stack as calls and groups.

use ast::{
    ArrayExpression, ArrayLength, RepeatExpression, SelectExpression, SequenceDelimiters,
    SequenceExpression,
};

use super::*;

pub(super) enum CollectionKind {
    Array,
    Sequence,
    Select { base: Value, dot: SpanId },
    Repeat,
}

pub(super) struct PendingCollection {
    kind: CollectionKind,
    open: SpanId,
    elements: Vec<Expression>,
    labels: Vec<Option<ast::Name>>,
    separators: Vec<SpanId>,
    length_separator: Option<SpanId>,
    separator_pending: bool,
    depth: u16,
}

impl PendingCollection {
    fn new(kind: CollectionKind, open: SpanId) -> Self {
        Self {
            kind,
            open,
            elements: Vec::new(),
            labels: Vec::new(),
            separators: Vec::new(),
            length_separator: None,
            separator_pending: false,
            depth: 0,
        }
    }

    pub(super) fn is_array(&self) -> bool {
        matches!(self.kind, CollectionKind::Array)
    }

    pub(super) fn close_token(&self) -> Token {
        match self.kind {
            CollectionKind::Repeat | CollectionKind::Sequence => Token::ParenClose,
            _ => Token::BracketClose,
        }
    }
}

impl Cursor<'_, '_> {
    pub(super) fn open_array(&mut self, frames: &mut Vec<Frame>) -> ParseResult<()> {
        self.check_delimiter_depth(frames)?;
        let open = self.advance()?.span_id;
        frames.push(Frame::new(FrameKind::Collection(PendingCollection::new(
            CollectionKind::Array,
            open,
        ))));
        Ok(())
    }

    pub(super) fn open_selection(&mut self, frames: &mut Vec<Frame>) -> ParseResult<()> {
        self.check_delimiter_depth(frames)?;
        let base = frames
            .last_mut()
            .expect("parent")
            .values
            .pop()
            .expect("selection base");
        let dot = self.advance()?.span_id;
        let open = self.advance()?.span_id;
        frames.push(Frame::new(FrameKind::Collection(PendingCollection::new(
            CollectionKind::Select { base, dot },
            open,
        ))));
        Ok(())
    }

    fn collection_element(&mut self, frame: &mut Frame) -> ParseResult<()> {
        let value = frame.finish(self)?;
        let FrameKind::Collection(collection) = &mut frame.kind else {
            unreachable!("collection element")
        };
        collection.depth = collection.depth.max(value.depth);
        collection.elements.push(value.expression);
        collection.labels.push(frame.argument_name.take());
        collection.separator_pending = false;
        Ok(())
    }

    pub(super) fn collection_comma(&mut self, frame: &mut Frame) -> ParseResult<()> {
        if let FrameKind::Group(open) = frame.kind {
            frame.kind =
                FrameKind::Collection(PendingCollection::new(CollectionKind::Sequence, open));
        }
        let FrameKind::Collection(collection) = &frame.kind else {
            unreachable!()
        };
        if !matches!(
            collection.kind,
            CollectionKind::Array | CollectionKind::Sequence
        ) || collection.length_separator.is_some()
        {
            return Err(self.error("commas require an array or parenthesized sequence"));
        }
        // A newline may already have finished the preceding element.
        if frame.expression_started()
            || collection.elements.is_empty()
            || collection.separator_pending
        {
            self.collection_element(frame)?;
        }
        let comma = self.advance()?.span_id;
        let FrameKind::Collection(collection) = &mut frame.kind else {
            unreachable!()
        };
        collection.separators.push(comma);
        collection.separator_pending = true;
        Ok(())
    }

    pub(super) fn collection_newline(&mut self, frame: &mut Frame) -> ParseResult<()> {
        let FrameKind::Collection(collection) = &frame.kind else {
            unreachable!()
        };
        if matches!(collection.kind, CollectionKind::Array)
            && collection.length_separator.is_none()
            && !matches!(
                self.next_collection_token(),
                Some(Token::ColonSemi | Token::Comma)
            )
        {
            self.collection_element(frame)?;
        }
        Ok(())
    }

    fn next_collection_token(&self) -> Option<Token> {
        self.tokens[self.position..]
            .iter()
            .find(|token| token.value != Token::Newline)
            .map(|token| token.value)
    }

    pub(super) fn collection_length(&mut self, frame: &mut Frame) -> ParseResult<()> {
        if let FrameKind::Group(open) = frame.kind {
            frame.kind =
                FrameKind::Collection(PendingCollection::new(CollectionKind::Repeat, open));
        }
        let FrameKind::Collection(collection) = &frame.kind else {
            unreachable!()
        };
        if matches!(collection.kind, CollectionKind::Sequence)
            || collection.length_separator.is_some()
            || !collection.elements.is_empty()
            || !collection.separators.is_empty()
        {
            return Err(self.error("expected exactly one element before ';'"));
        }
        self.collection_element(frame)?;
        let separator = self.advance()?.span_id;
        let FrameKind::Collection(collection) = &mut frame.kind else {
            unreachable!()
        };
        collection.length_separator = Some(separator);
        Ok(())
    }

    pub(super) fn close_collection(
        &mut self,
        mut frame: Frame,
        close: SpanId,
    ) -> ParseResult<Value> {
        let FrameKind::Collection(collection) = &frame.kind else {
            unreachable!()
        };
        let needs_length = collection.length_separator.is_some();
        let needs_index = matches!(collection.kind, CollectionKind::Select { .. })
            && collection.elements.is_empty();
        if needs_length || needs_index || frame.expression_started() {
            self.collection_element(&mut frame)?;
        }
        let FrameKind::Collection(mut collection) = frame.kind else {
            unreachable!()
        };
        let length = collection.length_separator.map(|separator| ArrayLength {
            separator,
            value: collection.elements.pop().expect("length expression"),
        });
        let (depth, start) = match &collection.kind {
            CollectionKind::Select { base, .. } => (
                collection.depth.max(base.depth) + 1,
                base.expression.span_id,
            ),
            _ => (collection.depth + 1, collection.open),
        };
        self.check_expression_depth(depth)?;
        let expression = match collection.kind {
            CollectionKind::Sequence => Expr::Sequence(Box::new(SequenceExpression {
                delimiters: SequenceDelimiters::Parenthesized {
                    open: collection.open,
                    close,
                },
                elements: collection.elements,
                separators: collection.separators,
            })),
            CollectionKind::Array => Expr::Array(Box::new(ArrayExpression {
                open: collection.open,
                elements: collection.elements,
                labels: collection.labels,
                separators: collection.separators,
                length,
                close,
            })),
            CollectionKind::Repeat => {
                let length = length.expect("repeat has a written separator");
                Expr::Repeat(Box::new(RepeatExpression {
                    open: collection.open,
                    value: collection.elements.pop().expect("repeat value"),
                    separator: length.separator,
                    length: length.value,
                    close,
                }))
            }
            CollectionKind::Select { base, dot } => Expr::Select(Box::new(SelectExpression {
                base: base.expression,
                dot,
                open: collection.open,
                index: collection.elements.pop().expect("index"),
                length,
                close,
            })),
        };
        Ok(Value {
            expression: self.node(expression, start, close),
            depth,
        })
    }
}
