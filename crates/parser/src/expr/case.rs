//! Case and ordinary arm-body continuations share the expression frame stack.

use ast::{
    AssignStatement, Binding, BindingOperator, BindingValue, Block, ConditionalExpression,
    DestructureStatement, ElseBranch, Guard, MatchArm, MatchExpression, MatchPattern,
    ReturnStatement, Statement, VariantPattern, ChainArm, ChainExpression,
};

use super::returns::PendingReturnValue;
use super::*;
use crate::cursor::ParseError;
use crate::declaration::{BindingContext, header_end};
use ast::IsTest;

pub(super) struct PendingMatch {
    when: SpanId,
    subject: Expression,
    is: SpanId,
    start: SpanId,
    arms: Vec<Spanned<MatchArm>>,
    pattern: Option<Spanned<MatchPattern>>,
    guard: Option<Guard>,
    then: Option<SpanId>,
    depth: u16,
}

/// The parts of an inline `when condition then value else other` parsed so far.
pub(super) struct PendingConditional {
    when: SpanId,
    condition: Expression,
    then: SpanId,
    then_branch: Option<Value>,
    else_keyword: Option<SpanId>,
    depth: u16,
}

/// The parts of a `when` chain parsed so far.
pub(super) struct PendingChain {
    when: SpanId,
    start: SpanId,
    arms: Vec<Spanned<ChainArm>>,
    /// The condition and `then` of the arm whose value is being read.
    arm: Option<(Expression, SpanId)>,
    else_keyword: Option<SpanId>,
    otherwise: Option<Expression>,
    depth: u16,
}

impl PendingChain {
    fn empty() -> Self {
        Self {
            when: SpanId::INVALID,
            start: SpanId::INVALID,
            arms: Vec::new(),
            arm: None,
            else_keyword: None,
            otherwise: None,
            depth: 0,
        }
    }
}

/// A `when` statement whose blocks are read as child [`FrameKind::Block`] frames.
pub(super) struct PendingWhenBranch {
    head: crate::when::WhenHead,
    then_block: Option<Spanned<Block>>,
    else_keyword: Option<SpanId>,
    depth: u16,
}

pub(super) enum StatementHead {
    Expression,
    Return {
        start: SpanId,
        pending: PendingReturnValue,
    },
    Binding(Box<Binding>, Spanned<BindingOperator>),
    Destructure(Spanned<VariantPattern>, Spanned<BindingOperator>),
    Assign(Expression, Option<Spanned<BinaryOperator>>, SpanId),
}

impl Cursor<'_, '_> {
    pub(super) fn open_case(
        &mut self,
        frames: &mut Vec<Frame>,
        end: ExpressionEnd,
    ) -> ParseResult<()> {
        // A `where` clause may hold the inline conditional `when a < b then A else B`, which
        // stays on one line, but never a case over patterns.
        let inline_in_header = matches!(end, ExpressionEnd::HeaderPostcondition)
            && self.tokens[self.position + 1..]
                .iter()
                .map(|token| token.value)
                .find(|token| {
                    matches!(
                        token,
                        Token::Then | Token::Is | Token::Newline | Token::Colon | Token::Comma
                    )
                })
                == Some(Token::Then);
        if !inline_in_header && !matches!(end, ExpressionEnd::Line | ExpressionEnd::ReturnValue) {
            return Err(
                self.error("case expressions are unsupported in headers and parameter defaults")
            );
        }
        if frames
            .iter()
            .any(|frame| matches!(frame.kind, FrameKind::LoopHeader(_) | FrameKind::LoopInvariant(_)))
        {
            return Err(self.error("case expressions are unsupported in loop headers"));
        }
        // `when a < b then x else y` on one line may sit in a call or a group; arms on later
        // lines may not.
        let one_line = self.tokens[self.position + 1..]
            .iter()
            .map(|token| token.value)
            .find(|token| {
                matches!(token, Token::Then | Token::Newline | Token::ParenClose | Token::BracketClose)
            })
            == Some(Token::Then);
        if !one_line && frames.iter().any(|frame| frame.kind.delimiter().is_some()) {
            return Err(self.error(
                "case arm layout inside parenthesized calls/groups or brackets is unsupported",
            ));
        }
        // Each owned expression contributes at most four continuations.
        self.check_expression_depth(u16::try_from(frames.len() / 4 + 1).unwrap_or(u16::MAX))?;
        let when = self.advance()?.span_id;
        // `when` alone on its line starts a chain of guarded arms.
        if matches!(self.current(), None | Some(Token::Newline)) {
            self.skip_newlines();
            let indented = self.current() == Some(Token::Indent);
            let start = if indented {
                self.advance()?.span_id
            } else {
                self.report("expected indented condition arms after 'when'")?;
                self.missing_span()
            };
            let pending = PendingChain {
                when,
                start,
                arms: Vec::new(),
                arm: None,
                else_keyword: None,
                otherwise: None,
                depth: 0,
            };
            if indented {
                frames.push(Frame::new(FrameKind::ChainArms(pending)));
            } else {
                self.chain_close(frames, pending, true)?;
            }
            return Ok(());
        }
        frames.push(Frame::new(FrameKind::MatchSubject(when)));
        Ok(())
    }

    pub(super) fn case_step(
        &mut self,
        frames: &mut Vec<Frame>,
        end: ExpressionEnd,
    ) -> ParseResult<bool> {
        let frame = frames.last_mut().expect("root remains");
        match &frame.kind {
            FrameKind::CondThen(_) => {
                if self.current() == Some(Token::Else) {
                    self.cond_else(frames)?;
                    return Ok(true);
                }
                if matches!(self.current(), None | Some(Token::Newline | Token::Dedent))
                    || frame.layout_end
                {
                    return Err(self.error("expected 'else' after the value of a conditional"));
                }
            }
            FrameKind::CondElse(_) => {
                // An outer conditional's `else` ends this inner `else` value.
                let last = frames.len() - 1;
                if self.current() == Some(Token::Else)
                    && frames[..last]
                        .iter()
                        .any(|frame| matches!(frame.kind, FrameKind::CondThen(_)))
                {
                    let value = frames[last].finish(self)?;
                    self.case_finish_value(frames, value)?;
                    return Ok(true);
                }
                return self.case_value_step(frames, end);
            }
            FrameKind::MatchSubject(_) => {
                // `is` ends the subject unless an `and` or `or` is pending: it binds tighter than
                // those, so in `b > 2 and a is ...` it tests `a` alone, inside the condition.
                if self.current() == Some(Token::Is) && !frame.connective_pending()
                    || self.current() == Some(Token::Then)
                    || matches!(self.current(), None | Some(Token::Newline | Token::Dedent))
                    || frame.layout_end
                {
                    self.case_subject(frames)?;
                    return Ok(true);
                }
            }
            FrameKind::MatchArms(_) => {
                self.case_arm(frames)?;
                return Ok(true);
            }
            FrameKind::ChainArms(_) => {
                self.chain_step(frames)?;
                return Ok(true);
            }
            FrameKind::ChainThen | FrameKind::ChainElse => {
                return self.case_value_step(frames, end);
            }
            FrameKind::Block { .. } => {
                self.case_block_step(frames)?;
                return Ok(true);
            }
            FrameKind::WhenBranch(_) => {
                self.when_branch_step(frames)?;
                return Ok(true);
            }
            FrameKind::ArmValue | FrameKind::StatementValue(_) | FrameKind::LoopBody(_) => {
                return self.case_value_step(frames, end);
            }
            _ => {}
        }
        Ok(false)
    }

    fn case_value_step(
        &mut self,
        frames: &mut Vec<Frame>,
        end: ExpressionEnd,
    ) -> ParseResult<bool> {
        if self.case_return_comma(frames)? {
            return Ok(true);
        }
        if self.case_return_postcondition(frames)? {
            return Ok(true);
        }
        let frame = frames.last_mut().expect("value frame");
        if matches!(frame.kind, FrameKind::LoopBody(_))
            && frame.values.is_empty()
            && !matches!(self.current(), None | Some(Token::Newline | Token::Dedent))
        {
            return Err(self.error("expected an indented loop body"));
        }
        // `then return ...` on one line: the arm (or binding) body is a block holding the return.
        if matches!(
            frame.kind,
            FrameKind::StatementValue(StatementHead::Binding(..)) | FrameKind::ArmValue
        ) && self.current() == Some(Token::Return)
            && frame.values.is_empty()
        {
            let start = self.tokens[self.position].span_id;
            frames.push(Frame::new(FrameKind::Block {
                statements: Vec::new(),
                start,
                depth: 0,
                inline: true,
            }));
            return Ok(true);
        }
        if frame.values.is_empty()
            && frame.prefixes.is_empty()
            && self.current() == Some(Token::Newline)
        {
            self.skip_expression_newlines()?;
            if self.current() == Some(Token::Indent) {
                let start = self.advance()?.span_id;
                frames.push(Frame::new(FrameKind::Block {
                    statements: Vec::new(),
                    start,
                    depth: 0,
                    inline: false,
                }));
                return Ok(true);
            }
            let value = frames.last_mut().expect("missing value").finish(self)?;
            self.case_finish_value(frames, value)?;
            return Ok(true);
        }
        let closes_delimiter = matches!(
            self.current(),
            Some(Token::ParenClose | Token::BracketClose | Token::Comma)
        ) && frames.iter().any(|frame| frame.kind.delimiter().is_some())
            && matches!(
                frames.last().map(|frame| &frame.kind),
                Some(FrameKind::CondThen(_) | FrameKind::CondElse(_))
            );
        let frame = frames.last_mut().expect("frame");
        // Only the owning layout boundary can finish an undelimited value.
        // A stray ')' must reach ordinary syntax recovery, never restart the
        // block's statement continuation at the same unconsumed token.
        let boundary = matches!(self.current(), None | Some(Token::Newline | Token::Dedent))
            || closes_delimiter
            || frame.layout_end
            // In a header the conditional ends where the header does.
            || (matches!(end, ExpressionEnd::HeaderPostcondition)
                && matches!(
                    self.current(),
                    Some(Token::Colon | Token::ColonEq | Token::Comma | Token::Thus)
                ));
        if boundary {
            let value = frame.finish(self)?;
            self.case_finish_value(frames, value)?;
            return Ok(true);
        }
        Ok(false)
    }

    fn case_subject(&mut self, frames: &mut Vec<Frame>) -> ParseResult<()> {
        let mut frame = frames.pop().expect("subject frame");
        let layout_end = frame.layout_end || self.current() == Some(Token::Newline);
        let subject = frame.finish(self)?;
        let FrameKind::MatchSubject(when) = frame.kind else {
            unreachable!()
        };
        // `when condition then ...` is the inline conditional, not a case over patterns.
        if self.current() == Some(Token::Then) && !layout_end {
            let then = self.advance()?.span_id;
            frames.push(Frame::new(FrameKind::CondThen(PendingConditional {
                when,
                condition: subject.expression,
                then,
                then_branch: None,
                else_keyword: None,
                depth: subject.depth,
            })));
            return Ok(());
        }
        let is = self.case_keyword(
            Token::Is,
            "expected 'is' after the case subject",
            layout_end,
        )?;
        // `when subject is Pattern then a else b` on one line is a conditional on a test; arms
        // on the following lines are a case.
        if !layout_end
            && !matches!(
                self.current(),
                None | Some(Token::Newline | Token::Dedent | Token::Indent)
            )
        {
            let (pattern, pattern_depth) = self.parse_match_pattern()?;
            let depth = subject.depth.max(pattern_depth) + 1;
            self.check_expression_depth(depth)?;
            let start = subject.expression.span_id;
            let end = pattern.span_id;
            let condition = self.node(
                Expr::IsTest(Box::new(ast::IsTest {
                    subject: subject.expression,
                    is_keyword: is,
                    pattern,
                })),
                start,
                end,
            );
            // `when x is P and y is Q then ...`: the test is the left of a longer condition.
            let condition = self.continue_condition(condition, depth)?;
            if self.current() != Some(Token::Then) {
                return Err(self.error("expected 'then' after the pattern of a conditional"));
            }
            let then = self.advance()?.span_id;
            frames.push(Frame::new(FrameKind::CondThen(PendingConditional {
                when,
                condition,
                then,
                then_branch: None,
                else_keyword: None,
                depth,
            })));
            return Ok(());
        }
        self.skip_newlines();
        let indented = self.current() == Some(Token::Indent);
        let start = if indented {
            self.advance()?.span_id
        } else {
            self.missing_span()
        };
        if !indented {
            self.report("expected indented case arms")?;
        }
        let pending = PendingMatch {
            when,
            subject: subject.expression,
            is,
            start,
            arms: Vec::new(),
            pattern: None,
            guard: None,
            then: None,
            depth: subject.depth,
        };
        if indented {
            frames.push(Frame::new(FrameKind::MatchArms(pending)));
        } else {
            self.case_close(frames, pending, layout_end)?;
        }
        Ok(())
    }

    /// `else` ends the value after `then`; what follows is the other branch.
    fn cond_else(&mut self, frames: &mut Vec<Frame>) -> ParseResult<()> {
        let mut frame = frames.pop().expect("then value");
        let value = frame.finish(self)?;
        let FrameKind::CondThen(mut pending) = frame.kind else {
            unreachable!()
        };
        pending.depth = pending.depth.max(value.depth);
        pending.then_branch = Some(value);
        pending.else_keyword = Some(self.advance()?.span_id);
        frames.push(Frame::new(FrameKind::CondElse(pending)));
        Ok(())
    }

    /// One step of a chain: a guarded arm, the closing `else`, or the end of the arms.
    fn chain_step(&mut self, frames: &mut Vec<Frame>) -> ParseResult<()> {
        self.skip_newlines();
        let FrameKind::ChainArms(pending) = &frames.last().expect("chain").kind else {
            unreachable!()
        };
        let finished = pending.otherwise.is_some();
        if matches!(self.current(), None | Some(Token::Dedent)) {
            let frame = frames.pop().expect("chain");
            let FrameKind::ChainArms(mut pending) = frame.kind else {
                unreachable!()
            };
            if self.current() == Some(Token::Dedent) {
                self.advance()?;
            }
            if !finished {
                self.report("expected 'else' to end the condition arms")?;
                let span = self.missing_span();
                pending.else_keyword = Some(span);
                pending.otherwise = Some(Spanned::new(Expr::Missing, span));
            }
            return self.chain_close(frames, pending, true);
        }
        if finished {
            // Nothing may follow the `else` arm: skip the stray lines, keeping later items.
            self.report("expected the condition arms to end after 'else'")?;
            self.skip_arm_line()?;
            return Ok(());
        }
        if self.current() == Some(Token::Else) {
            let else_keyword = self.advance()?.span_id;
            let FrameKind::ChainArms(pending) = &mut frames.last_mut().expect("chain").kind else {
                unreachable!()
            };
            pending.else_keyword = Some(else_keyword);
            frames.push(Frame::new(FrameKind::ChainElse));
            return Ok(());
        }
        let condition = match self.chain_condition() {
            Ok(condition) => condition,
            Err(ParseError::Syntax(diagnostic)) => {
                self.record(diagnostic).map_err(ParseError::Abort)?;
                self.skip_arm_line()?;
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let FrameKind::ChainArms(pending) = &mut frames.last_mut().expect("chain").kind else {
            unreachable!()
        };
        if self.current() == Some(Token::Then) {
            let then = self.advance()?.span_id;
            pending.arm = Some((condition, then));
            frames.push(Frame::new(FrameKind::ChainThen));
            return Ok(());
        }
        self.report("expected 'then' after the condition of an arm")?;
        let then = self.missing_span();
        let value = Spanned::new(Expr::Missing, then);
        self.skip_arm_line()?;
        let FrameKind::ChainArms(pending) = &mut frames.last_mut().expect("chain").kind else {
            unreachable!()
        };
        pending.arm = Some((condition, then));
        self.chain_push_arm(pending, Value { expression: value, depth: 1 });
        Ok(())
    }

    /// A condition, which may end in `subject is Pattern` and continue with `and`/`or`: the
    /// guard of a chain arm or of a case arm, before its `then`.
    pub(super) fn chain_condition(&mut self) -> ParseResult<Expression> {
        let mut condition = self.parse_condition()?;
        if self.current() == Some(Token::Is) {
            let is_keyword = self.advance()?.span_id;
            let (pattern, depth) = self.parse_match_pattern()?;
            self.check_expression_depth(depth + 1)?;
            let start = condition.span_id;
            let end = pattern.span_id;
            condition = self.node(
                Expr::IsTest(Box::new(IsTest {
                    subject: condition,
                    is_keyword,
                    pattern,
                })),
                start,
                end,
            );
            condition = self.continue_condition(condition, depth + 1)?;
        }
        Ok(condition)
    }

    /// `left and ...` / `left or ...` after a leading `is` test: the rest of the condition,
    /// with `left` as its first operand. Returns `left` when no connective follows.
    pub(crate) fn continue_condition(
        &mut self,
        left: Expression,
        depth: u16,
    ) -> ParseResult<Expression> {
        if !matches!(self.current(), Some(Token::And | Token::Or)) {
            return Ok(left);
        }
        self.expression_seed = Some(Value {
            expression: left,
            depth,
        });
        self.parse_condition()
    }

    /// Skips the rest of the current arm line, with any lines indented under it.
    fn skip_arm_line(&mut self) -> ParseResult<()> {
        let mut nested = 0_usize;
        for _ in self.position..self.tokens.len() {
            match self.current() {
                None => break,
                Some(Token::Newline) if nested == 0 => break,
                Some(Token::Dedent) if nested == 0 => break,
                Some(Token::Indent) => nested += 1,
                Some(Token::Dedent) => nested -= 1,
                _ => {}
            }
            self.advance()?;
        }
        Ok(())
    }

    fn chain_push_arm(&mut self, pending: &mut PendingChain, value: Value) {
        let (condition, then) = pending.arm.take().expect("arm head");
        let start = condition.span_id;
        let end = value.expression.span_id;
        pending.depth = pending.depth.max(value.depth);
        let arm = self.node(
            ChainArm {
                condition,
                then_keyword: then,
                value: value.expression,
            },
            start,
            end,
        );
        pending.arms.push(arm);
    }

    fn chain_close(
        &mut self,
        frames: &mut [Frame],
        pending: PendingChain,
        layout_end: bool,
    ) -> ParseResult<()> {
        let (else_keyword, otherwise) = match (pending.else_keyword, pending.otherwise) {
            (Some(keyword), Some(value)) => (keyword, value),
            _ => {
                self.report("expected 'else' to end the condition arms")?;
                let span = self.missing_span();
                (span, Spanned::new(Expr::Missing, span))
            }
        };
        let end = otherwise.span_id;
        let depth = pending.depth + 1;
        self.check_expression_depth(depth)?;
        let _ = pending.start;
        let expression = self.node(
            Expr::Chain(Box::new(ChainExpression {
                when_keyword: pending.when,
                arms: pending.arms,
                else_keyword,
                otherwise,
            })),
            pending.when,
            end,
        );
        let parent = frames.last_mut().expect("parent");
        parent.values.push(Value { expression, depth });
        parent.layout_end = layout_end;
        Ok(())
    }

    fn case_arm(&mut self, frames: &mut Vec<Frame>) -> ParseResult<()> {
        self.skip_newlines();
        if matches!(self.current(), None | Some(Token::Dedent)) {
            let frame = frames.pop().expect("case arms");
            let FrameKind::MatchArms(pending) = frame.kind else {
                unreachable!()
            };
            if self.current() == Some(Token::Dedent) {
                self.advance()?;
            }
            return self.case_close(frames, pending, true);
        }
        // `else value`: the arm for everything the others do not name; it has no `then`.
        let mut guard = None;
        let (pattern, pattern_depth, then) = if self.current() == Some(Token::Else) {
            let else_keyword = self.advance()?.span_id;
            let pattern = self.node(MatchPattern::Else { else_keyword }, else_keyword, else_keyword);
            (pattern, 1, self.missing_span())
        } else {
            let (pattern, pattern_depth) = self.parse_match_pattern_nested(true)?;
            self.check_expression_depth(pattern_depth)?;
            // `pattern and condition then value`: only a constructor pattern has binders to
            // guard on; the other patterns keep their plain `then`.
            if self.current() == Some(Token::And)
                && matches!(pattern.value, MatchPattern::Variant(_))
            {
                let and_keyword = self.advance()?.span_id;
                let condition = self.chain_condition()?;
                guard = Some(Guard {
                    and_keyword,
                    condition,
                });
            }
            let then = self.case_keyword(
                Token::Then,
                "expected 'then' after case pattern; computed patterns are unsupported",
                false,
            )?;
            (pattern, pattern_depth, then)
        };
        self.check_expression_depth(pattern_depth)?;
        let FrameKind::MatchArms(pending) = &mut frames.last_mut().expect("arms").kind else {
            unreachable!()
        };
        pending.depth = pending.depth.max(pattern_depth);
        pending.pattern = Some(pattern);
        pending.guard = guard;
        pending.then = Some(then);
        frames.push(Frame::new(FrameKind::ArmValue));
        Ok(())
    }

    fn case_keyword(
        &mut self,
        token: Token,
        message: &str,
        owned_boundary: bool,
    ) -> ParseResult<SpanId> {
        if self.current() == Some(token) {
            return Ok(self.advance()?.span_id);
        }
        let layout_end = owned_boundary
            || self.position > 0 && self.tokens[self.position - 1].value == Token::Dedent;
        if !matches!(self.current(), None | Some(Token::Newline | Token::Dedent)) && !layout_end {
            return Err(self.error(message));
        }
        self.report(message)?;
        Ok(self.missing_span())
    }

    fn case_close(
        &mut self,
        frames: &mut [Frame],
        pending: PendingMatch,
        layout_end: bool,
    ) -> ParseResult<()> {
        if pending.arms.is_empty() {
            self.report("expected at least one case arm")?;
        }
        let end = pending.arms.last().map_or(pending.start, |arm| arm.span_id);
        let arms = self.node(pending.arms, pending.start, end);
        let depth = pending.depth + 1;
        self.check_expression_depth(depth)?;
        let expression = self.node(
            Expr::Match(Box::new(MatchExpression {
                when_keyword: pending.when,
                subject: pending.subject,
                is_keyword: pending.is,
                arms,
            })),
            pending.when,
            end,
        );
        let parent = frames.last_mut().expect("parent");
        parent.values.push(Value { expression, depth });
        parent.layout_end = layout_end;
        Ok(())
    }

    fn case_finish_value(&mut self, frames: &mut Vec<Frame>, value: Value) -> ParseResult<()> {
        let frame = frames.pop().expect("value continuation");
        match frame.kind {
            FrameKind::LoopBody(pending) => self.close_loop(frames, pending, value)?,
            FrameKind::CondElse(pending) => {
                let then_branch = pending.then_branch.expect("then value");
                let depth = pending.depth.max(value.depth) + 1;
                self.check_expression_depth(depth)?;
                let end = value.expression.span_id;
                let expression = self.node(
                    Expr::Conditional(Box::new(ConditionalExpression {
                        when_keyword: pending.when,
                        condition: pending.condition,
                        then_keyword: pending.then,
                        then_branch: then_branch.expression,
                        else_keyword: pending.else_keyword.expect("else"),
                        else_branch: value.expression,
                    })),
                    pending.when,
                    end,
                );
                let parent = frames.last_mut().expect("parent");
                parent.values.push(Value { expression, depth });
            }
            FrameKind::ChainThen => {
                let FrameKind::ChainArms(pending) = &mut frames.last_mut().expect("chain").kind
                else {
                    unreachable!()
                };
                // `chain_push_arm` needs `self`; the pending chain is moved out and back.
                let mut taken = std::mem::replace(pending, PendingChain::empty());
                self.chain_push_arm(&mut taken, value);
                let FrameKind::ChainArms(pending) = &mut frames.last_mut().expect("chain").kind
                else {
                    unreachable!()
                };
                *pending = taken;
            }
            FrameKind::ChainElse => {
                let FrameKind::ChainArms(pending) = &mut frames.last_mut().expect("chain").kind
                else {
                    unreachable!()
                };
                pending.depth = pending.depth.max(value.depth);
                pending.otherwise = Some(value.expression);
            }
            FrameKind::ArmValue => {
                let FrameKind::MatchArms(pending) = &mut frames.last_mut().expect("arms").kind
                else {
                    unreachable!()
                };
                let pattern = pending.pattern.take().expect("arm pattern");
                let start = pattern.span_id;
                let end = value.expression.span_id;
                let arm = self.node(
                    MatchArm {
                        pattern,
                        guard: pending.guard.take(),
                        then_keyword: pending.then.take().expect("then"),
                        body: value.expression,
                    },
                    start,
                    end,
                );
                pending.depth = pending.depth.max(value.depth);
                pending.arms.push(arm);
            }
            FrameKind::StatementValue(head) => {
                let (statement, statement_depth) = self.case_statement(head, value)?;
                let FrameKind::Block {
                    statements, depth, ..
                } = &mut frames.last_mut().expect("block").kind
                else {
                    unreachable!()
                };
                *depth = (*depth).max(statement_depth);
                statements.push(statement);
            }
            _ => unreachable!("only case value continuations finish here"),
        }
        Ok(())
    }

    fn case_statement(
        &mut self,
        head: StatementHead,
        value: Value,
    ) -> ParseResult<(Spanned<Statement>, u16)> {
        let mut depth = value.depth;
        let value = value.expression;
        let end = value.span_id;
        let (start, statement) = match head {
            StatementHead::Expression => (end, Statement::Expression(value)),
            StatementHead::Return { start, mut pending } => {
                pending.push(Value {
                    expression: value,
                    depth,
                });
                let returned = pending.finish(self)?.expect("return expression");
                depth = returned.depth;
                (
                    start,
                    Statement::Return(ReturnStatement {
                        value: Some(returned.expression),
                    }),
                )
            }
            StatementHead::Binding(mut binding, operator) => {
                let start = binding.name.span_id;
                binding.value = Some(BindingValue {
                    operator,
                    expression: value,
                });
                (start, Statement::Binding(binding))
            }
            StatementHead::Assign(place, compound, operator) => (
                place.span_id,
                Statement::Assign(AssignStatement {
                    place,
                    compound,
                    operator,
                    value,
                }),
            ),
            StatementHead::Destructure(pattern, operator) => (
                pattern.span_id,
                Statement::Destructure(DestructureStatement {
                    pattern,
                    operator,
                    value,
                }),
            ),
        };
        Ok((self.node(statement, start, end), depth))
    }

    fn case_block_step(&mut self, frames: &mut Vec<Frame>) -> ParseResult<()> {
        let FrameKind::Block {
            statements, inline, ..
        } = &frames.last().expect("block").kind
        else {
            unreachable!()
        };
        if *inline && !statements.is_empty() {
            return self.case_block_close(frames);
        }
        self.skip_newlines();
        if matches!(self.current(), None | Some(Token::Dedent)) {
            return self.case_block_close(frames);
        }
        if self.current() == Some(Token::When)
            && let Some(head) = self.parse_when_head()?
        {
            frames.push(Frame::new(FrameKind::WhenBranch(PendingWhenBranch {
                head,
                then_block: None,
                else_keyword: None,
                depth: 0,
            })));
            return self.when_block_open(frames);
        }
        let head = if self.current() == Some(Token::Return) {
            let start = self.advance()?.span_id;
            if matches!(self.current(), None | Some(Token::Newline | Token::Dedent)) {
                let statement = Spanned::new(
                    Statement::Return(ReturnStatement {
                        value: None,
                    }),
                    start,
                );
                let FrameKind::Block { statements, .. } =
                    &mut frames.last_mut().expect("block").kind
                else {
                    unreachable!()
                };
                statements.push(statement);
                return Ok(());
            }
            StatementHead::Return {
                start,
                pending: PendingReturnValue::default(),
            }
        } else if self.starts_assignment() {
            let place = self.parse_place()?;
            let (compound, operator) = self.parse_assignment_operator()?;
            StatementHead::Assign(place, compound, operator)
        } else if self.starts_destructure() {
            let pattern = self.parse_variant_pattern()?;
            let operator = Spanned::new(BindingOperator::ColonEquals, self.advance()?.span_id);
            StatementHead::Destructure(pattern, operator)
        } else if self.starts_binding() {
            let binding = self.parse_binding_header(BindingContext::Value)?;
            let operator = match self.current() {
                Some(Token::Colon) => BindingOperator::Colon,
                Some(Token::ColonEq) => BindingOperator::ColonEquals,
                _ => {
                    let start = binding.name.span_id;
                    let end = header_end(&binding);
                    let statement = self.node(Statement::Binding(Box::new(binding)), start, end);
                    let FrameKind::Block { statements, .. } =
                        &mut frames.last_mut().expect("block").kind
                    else {
                        unreachable!()
                    };
                    statements.push(statement);
                    return Ok(());
                }
            };
            let operator = Spanned::new(operator, self.advance()?.span_id);
            StatementHead::Binding(Box::new(binding), operator)
        } else {
            StatementHead::Expression
        };
        frames.push(Frame::new(FrameKind::StatementValue(head)));
        Ok(())
    }

    /// The block after `then` or `else`: indented lines, or one statement on the same line.
    fn when_block_open(&mut self, frames: &mut Vec<Frame>) -> ParseResult<()> {
        self.check_expression_depth(u16::try_from(frames.len() / 4 + 1).unwrap_or(u16::MAX))?;
        let inline = self.current() != Some(Token::Newline);
        let start = if inline {
            self.tokens[self.position].span_id
        } else {
            self.skip_newlines();
            if self.current() != Some(Token::Indent) {
                return Err(self.error("expected an indented block"));
            }
            self.advance()?.span_id
        };
        frames.push(Frame::new(FrameKind::Block {
            statements: Vec::new(),
            start,
            depth: 0,
            inline,
        }));
        Ok(())
    }

    /// A branch block has closed into this frame's values: it is the then-block, or the
    /// else-block that ends the statement. `else` may follow the then-block on its own line.
    fn when_branch_step(&mut self, frames: &mut Vec<Frame>) -> ParseResult<()> {
        let frame = frames.last_mut().expect("when branch");
        let value = frame.values.pop().expect("a closed branch block");
        let Expr::Block(block) = value.expression.value else {
            unreachable!("branches are blocks")
        };
        let block = Spanned::new(block, value.expression.span_id);
        let FrameKind::WhenBranch(pending) = &mut frame.kind else {
            unreachable!()
        };
        pending.depth = pending.depth.max(value.depth);
        if pending.then_block.is_none() {
            pending.then_block = Some(block);
            let mut at = self.position;
            while self.tokens.get(at).map(|token| token.value) == Some(Token::Newline) {
                at += 1;
            }
            if self.tokens.get(at).map(|token| token.value) == Some(Token::Else) {
                self.position = at;
                let else_keyword = self.advance()?.span_id;
                let FrameKind::WhenBranch(pending) = &mut frames.last_mut().expect("when").kind
                else {
                    unreachable!()
                };
                pending.else_keyword = Some(else_keyword);
                return self.when_block_open(frames);
            }
            return self.when_branch_close(frames, None);
        }
        self.when_branch_close(frames, Some(block))
    }

    fn when_branch_close(
        &mut self,
        frames: &mut Vec<Frame>,
        else_block: Option<Spanned<Block>>,
    ) -> ParseResult<()> {
        let FrameKind::WhenBranch(pending) = frames.pop().expect("when branch").kind else {
            unreachable!()
        };
        let then_block = pending.then_block.expect("the then-block is read first");
        let end = else_block
            .as_ref()
            .map_or(then_block.span_id, |block| block.span_id);
        let else_branch = else_block.map(|block| ElseBranch {
            else_keyword: pending.else_keyword.expect("an else-block follows `else`"),
            block,
        });
        let depth = pending.depth + 1;
        self.check_expression_depth(depth)?;
        let start = pending.head.when_keyword;
        let statement = self.node(pending.head.into_statement(then_block, else_branch), start, end);
        let FrameKind::Block {
            statements,
            depth: block_depth,
            ..
        } = &mut frames.last_mut().expect("enclosing block").kind
        else {
            unreachable!("a when statement is read inside a block")
        };
        *block_depth = (*block_depth).max(depth);
        statements.push(statement);
        Ok(())
    }

    fn case_block_close(&mut self, frames: &mut Vec<Frame>) -> ParseResult<()> {
        let frame = frames.pop().expect("block");
        let FrameKind::Block {
            statements,
            start,
            depth,
            inline,
        } = frame.kind
        else {
            unreachable!()
        };
        let end = statements
            .last()
            .map_or(start, |statement| statement.span_id);
        let expression = self.node(Expr::Block(Block { statements }), start, end);
        let depth = depth + 1;
        self.check_expression_depth(depth)?;
        if !inline && self.current() == Some(Token::Dedent) {
            self.advance()?;
        }
        let parent = frames.last_mut().expect("body value");
        parent.values.push(Value { expression, depth });
        parent.layout_end = true;
        Ok(())
    }

    /// `return value thus claim` is gone: a claim is written in the header's `where` clause.
    fn case_return_postcondition(&mut self, frames: &mut [Frame]) -> ParseResult<bool> {
        if self.current() != Some(Token::Thus) {
            return Ok(false);
        }
        let frame = frames.last_mut().expect("return value");
        if !matches!(
            frame.kind,
            FrameKind::StatementValue(StatementHead::Return { .. })
        ) {
            return Ok(false);
        }
        Err(self.error("a claim is written in the header's `where` clause, not on a return"))
    }

    fn case_return_comma(&mut self, frames: &mut [Frame]) -> ParseResult<bool> {
        if self.current() != Some(Token::Comma) {
            return Ok(false);
        }
        let frame = frames.last_mut().expect("return value");
        if !matches!(
            frame.kind,
            FrameKind::StatementValue(StatementHead::Return { .. })
        ) {
            return Ok(false);
        }
        let value = frame.finish(self)?;
        let FrameKind::StatementValue(StatementHead::Return { pending, .. }) = &mut frame.kind
        else {
            unreachable!()
        };
        pending.push(value);
        pending.comma(self.advance()?.span_id);
        frame.layout_end = false;
        Ok(true)
    }
}
