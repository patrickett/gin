//! Single-binder for loops and expression-condition while loops share required `do`.

use ast::{LoopExpression, LoopInvariant};

use super::*;

pub(super) enum LoopHead {
    For {
        keyword: SpanId,
        binder: Name,
        in_keyword: SpanId,
    },
    While(SpanId),
}

pub(super) struct PendingLoop {
    head: LoopHead,
    header: Value,
    invariant: Option<(SpanId, Value)>,
    do_keyword: SpanId,
}

/// `loop-head where <here>`: the invariant, up to `do`.
pub(super) struct PendingInvariant {
    head: LoopHead,
    header: Value,
    where_keyword: SpanId,
}

impl Cursor<'_, '_> {
    pub(super) fn open_loop(
        &mut self,
        frames: &mut Vec<Frame>,
        end: ExpressionEnd,
    ) -> ParseResult<()> {
        if !matches!(end, ExpressionEnd::Line | ExpressionEnd::ReturnValue) {
            return Err(self.error("loops are unsupported in headers and defaults"));
        }
        if frames
            .iter()
            .any(|frame| matches!(frame.kind, FrameKind::LoopHeader(_) | FrameKind::LoopInvariant(_)))
        {
            return Err(self.error("loops are unsupported in loop headers"));
        }
        if frames.iter().any(|frame| frame.kind.delimiter().is_some()) {
            return Err(self.error("loop layout inside delimiters is unsupported"));
        }
        self.check_expression_depth(u16::try_from(frames.len() / 4 + 1).unwrap_or(u16::MAX))?;
        let keyword = self.advance()?;
        let head = if keyword.value == Token::For {
            if self.current() != Some(Token::Id) {
                return Err(self.error("only single-name for binders are supported"));
            }
            let binder = self.parse_name()?;
            if self.current() != Some(Token::In) {
                return Err(self.error("expected 'in' after for binder"));
            }
            LoopHead::For {
                keyword: keyword.span_id,
                binder,
                in_keyword: self.advance()?.span_id,
            }
        } else {
            LoopHead::While(keyword.span_id)
        };
        frames.push(Frame::new(FrameKind::LoopHeader(head)));
        Ok(())
    }

    pub(super) fn loop_step(&mut self, frames: &mut Vec<Frame>) -> ParseResult<bool> {
        let frame = frames.last().expect("root");
        let in_header = matches!(frame.kind, FrameKind::LoopHeader(_));
        let in_invariant = matches!(frame.kind, FrameKind::LoopInvariant(_));
        if !in_header && !in_invariant {
            return Ok(false);
        }
        let boundary = matches!(
            self.current(),
            None | Some(Token::Newline | Token::Dedent | Token::Do)
        ) || (in_header && self.current() == Some(Token::Where));
        if !boundary {
            return Ok(false);
        }
        let mut frame = frames.pop().expect("loop header");
        let finished = frame.finish(self)?;
        if in_header {
            let FrameKind::LoopHeader(head) = frame.kind else {
                unreachable!()
            };
            if self.current() == Some(Token::Where) {
                let where_keyword = self.advance()?.span_id;
                frames.push(Frame::new(FrameKind::LoopInvariant(PendingInvariant {
                    head,
                    header: finished,
                    where_keyword,
                })));
                return Ok(true);
            }
            let do_keyword = self.loop_do()?;
            frames.push(Frame::new(FrameKind::LoopBody(PendingLoop {
                head,
                header: finished,
                invariant: None,
                do_keyword,
            })));
            return Ok(true);
        }
        let FrameKind::LoopInvariant(pending) = frame.kind else {
            unreachable!()
        };
        let do_keyword = self.loop_do()?;
        frames.push(Frame::new(FrameKind::LoopBody(PendingLoop {
            head: pending.head,
            header: pending.header,
            invariant: Some((pending.where_keyword, finished)),
            do_keyword,
        })));
        Ok(true)
    }

    fn loop_do(&mut self) -> ParseResult<SpanId> {
        Ok(if self.current() == Some(Token::Do) {
            self.advance()?.span_id
        } else {
            self.report("expected 'do' after loop header")?;
            self.missing_span()
        })
    }

    pub(super) fn close_loop(
        &mut self,
        frames: &mut [Frame],
        pending: PendingLoop,
        body: Value,
    ) -> ParseResult<()> {
        let depth = pending.header.depth.max(body.depth).max(pending.invariant.as_ref().map_or(0, |(_, value)| value.depth)) + 1;
        self.check_expression_depth(depth)?;
        let end = body.expression.span_id;
        let (start, expression) = match pending.head {
            LoopHead::For {
                keyword,
                binder,
                in_keyword,
            } => (
                keyword,
                LoopExpression::For {
                    for_keyword: keyword,
                    binder,
                    in_keyword,
                    iterable: pending.header.expression,
                    invariant: pending.invariant.map(|(where_keyword, value)| LoopInvariant { where_keyword, condition: value.expression }),
                    do_keyword: pending.do_keyword,
                    body: body.expression,
                },
            ),
            LoopHead::While(keyword) => (
                keyword,
                LoopExpression::While {
                    while_keyword: keyword,
                    condition: pending.header.expression,
                    invariant: pending.invariant.map(|(where_keyword, value)| LoopInvariant { where_keyword, condition: value.expression }),
                    do_keyword: pending.do_keyword,
                    body: body.expression,
                },
            ),
        };
        let expression = self.node(Expr::Loop(Box::new(expression)), start, end);
        let parent = frames.last_mut().expect("loop parent");
        parent.values.push(Value { expression, depth });
        parent.layout_end = true;
        Ok(())
    }
}
