//! Iterative operator reduction and delimiter frames; no recursive parser calls.

use ast::{Argument, BinaryOperator, Expr, Expression, LiteralKind, Name, SpanId, Spanned};
use lexer::TokenKind as Token;

use crate::cursor::{Cursor, ParseResult};
use crate::operators::{binary_operator, is_comparison, precedence};
use crate::{ParseFailure, ParseLimit};

mod case;
mod collection;
mod loops;
mod numeric_select;
mod prefix;
pub(crate) mod returns;
mod type_layout;

use case::{PendingMatch, StatementHead};
use prefix::Prefix;
pub(crate) use type_layout::TypeLayout;

#[derive(Clone, Copy)]
pub(crate) enum ExpressionEnd {
    Line,
    /// The condition of a `when` statement: ends at `then` (or `is`, which makes it a case).
    Condition,
    /// The domain after `is in`: a name, a call or a range, up to `and`/`or`.
    Domain,
    ReturnValue,
    ParameterDefault,
    TypeValue,
    DomainBound,
    Refinement,
    HeaderBase,
    HeaderBound,
    HeaderPredicate,
    HeaderPostcondition,
    /// One item of a parenthesized claim group: ends at the `and` that joins the next.
    HeaderGroupItem,
}

impl ExpressionEnd {
    pub fn for_header(self) -> Self {
        match self {
            Self::TypeValue => Self::HeaderBase,
            Self::DomainBound => Self::HeaderBound,
            Self::Refinement => Self::HeaderPredicate,
            _ => unreachable!("only domain parts select a header boundary"),
        }
    }

    fn is_header(self) -> bool {
        matches!(
            self,
            Self::HeaderBase
                | Self::HeaderBound
                | Self::HeaderPredicate
                | Self::HeaderPostcondition
                | Self::HeaderGroupItem
        )
    }

    fn allows_type_prefixes(self) -> bool {
        matches!(self, Self::TypeValue | Self::HeaderBase)
    }

    /// A value or a condition, where `not` and `is` are operators and a borrow is an expression:
    /// not a type, a domain or a refinement.
    fn is_value(self) -> bool {
        matches!(
            self,
            Self::Line | Self::Condition | Self::ReturnValue | Self::ParameterDefault
        )
    }

    fn matches(&self, token: Option<Token>) -> bool {
        if self.is_header()
            && matches!(
                token,
                None | Some(
                    Token::Colon
                        | Token::ColonEq
                        | Token::Comma
                        | Token::ParenClose
                        | Token::Newline
                        | Token::Dedent
                        | Token::Thus
                        | Token::Where
                )
            )
        {
            return true;
        }
        match self {
            Self::Line | Self::ParameterDefault => false,
            Self::Condition => matches!(token, Some(Token::Then | Token::Is)),
            Self::Domain => matches!(token, Some(Token::And | Token::Or)),
            Self::ReturnValue => token == Some(Token::Thus),
            Self::TypeValue => token == Some(Token::To),
            Self::DomainBound => matches!(token, Some(Token::To | Token::And)),
            Self::Refinement => token == Some(Token::And),
            Self::HeaderBase | Self::HeaderPredicate => {
                matches!(token, Some(Token::And | Token::Or))
            }
            Self::HeaderBound => matches!(token, Some(Token::To | Token::And | Token::Or)),
            Self::HeaderPostcondition => token == Some(Token::Or),
            Self::HeaderGroupItem => token == Some(Token::And),
        }
    }
}

pub(crate) struct Value {
    pub(crate) expression: Expression,
    pub(crate) depth: u16,
}

enum FrameKind {
    Root,
    Group(SpanId),
    Call {
        callee: Value,
        open: SpanId,
    },
    Collection(collection::PendingCollection),
    LoopHeader(loops::LoopHead),
    LoopInvariant(loops::PendingInvariant),
    LoopBody(loops::PendingLoop),
    MatchSubject(SpanId),
    MatchArms(PendingMatch),
    ArmValue,
    /// The guarded arms of a subjectless `when` chain, between its head and its `else`.
    ChainArms(case::PendingChain),
    /// `condition then <here>`: the value of a chain arm.
    ChainThen,
    /// `else <here>`: the value that ends a chain.
    ChainElse,
    /// `when condition then <here> else ...`: the value before `else`.
    CondThen(case::PendingConditional),
    /// `... else <here>`: the value after `else`, which ends the conditional.
    CondElse(case::PendingConditional),
    Block {
        statements: Vec<Spanned<ast::Statement>>,
        start: SpanId,
        depth: u16,
        inline: bool,
    },
    StatementValue(StatementHead),
    /// `not <here>`: the comparison or test it covers, up to `and`, `or` or the end of the
    /// condition. Carries the `not` keyword.
    Not(SpanId),
    /// A `when` statement between its head and its last block; the blocks are child frames.
    WhenBranch(case::PendingWhenBranch),
}

impl FrameKind {
    fn delimiter(&self) -> Option<Token> {
        match self {
            Self::Group(_) | Self::Call { .. } => Some(Token::ParenClose),
            Self::Collection(collection) => Some(collection.close_token()),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
enum ReductionOperator {
    Binary(BinaryOperator),
    Range,
    Cast,
}

impl ReductionOperator {
    fn precedence(self) -> u8 {
        match self {
            Self::Binary(operator) => precedence(operator),
            Self::Range => 2,
            Self::Cast => 10,
        }
    }
}

struct Frame {
    kind: FrameKind,
    values: Vec<Value>,
    operators: Vec<Spanned<ReductionOperator>>,
    prefixes: Vec<Prefix>,
    arguments: Vec<Spanned<Argument>>,
    argument_name: Option<Name>,
    /// The argument is `name Type` (a payload field declared in place), not `name: value`.
    argument_typed: bool,
    arguments_depth: u16,
    /// This frame's value consumed its terminating layout, not a sibling's.
    layout_end: bool,
    /// A group or call opened within a written cast target.
    type_prefixes: bool,
    /// A group opened within an argument; type/value roles are resolved later.
    pointer_arguments: bool,
    /// `(name in Domain: <here>)`: the binding written before the value of a group.
    bind: Option<Box<BindHead>>,
}

struct BindHead {
    name: Name,
    annotation: Spanned<ast::Annotation>,
    operator: Spanned<ast::BindingOperator>,
}

impl Frame {
    fn new(kind: FrameKind) -> Self {
        Self {
            kind,
            values: Vec::new(),
            operators: Vec::new(),
            prefixes: Vec::new(),
            arguments: Vec::new(),
            argument_name: None,
            argument_typed: false,
            arguments_depth: 0,
            layout_end: false,
            type_prefixes: false,
            pointer_arguments: false,
            bind: None,
        }
    }

    /// An `and` or `or` is waiting for its right operand.
    fn connective_pending(&self) -> bool {
        self.operators.iter().any(|operator| {
            matches!(
                operator.value,
                ReductionOperator::Binary(BinaryOperator::And | BinaryOperator::Or)
            )
        })
    }

    fn needs_operand(&self) -> bool {
        assert!(self.values.len() <= self.operators.len() + 1);
        self.values.len() == self.operators.len()
    }

    fn allows_type_prefixes(&self) -> bool {
        self.type_prefixes
            || matches!(self.kind, FrameKind::Collection(_))
            || self
                .operators
                .last()
                .is_some_and(|operator| matches!(operator.value, ReductionOperator::Cast))
    }

    fn allows_pointer_arguments(&self) -> bool {
        self.pointer_arguments || matches!(self.kind, FrameKind::Call { .. })
    }

    fn expression_started(&self) -> bool {
        !self.values.is_empty() || !self.operators.is_empty() || !self.prefixes.is_empty()
    }

    fn argument_started(&self) -> bool {
        assert!(matches!(self.kind, FrameKind::Call { .. }));
        !self.values.is_empty() || self.argument_name.is_some() || !self.prefixes.is_empty()
    }

    fn finish(&mut self, cursor: &mut Cursor<'_, '_>) -> ParseResult<Value> {
        if self.needs_operand() {
            self.values.push(cursor.missing_expression()?);
        }
        self.finish_prefixes(cursor)?;
        for _ in 0..self.operators.len() {
            self.reduce(cursor)?;
        }
        assert_eq!(self.values.len(), 1);
        Ok(self.values.pop().expect("one expression remains"))
    }

    fn finish_root(&mut self, cursor: &mut Cursor<'_, '_>) -> ParseResult<Value> {
        let value = self.finish(cursor)?;
        if self.layout_end {
            cursor.expression_layout_end = Some(cursor.position);
        }
        Ok(value)
    }

    fn reduce(&mut self, cursor: &mut Cursor<'_, '_>) -> ParseResult<()> {
        assert!(self.values.len() >= 2);
        let operator = self.operators.pop().expect("pending operator");
        let right = self.values.pop().expect("right operand");
        let left = self.values.pop().expect("left operand");
        let depth = left.depth.max(right.depth) + 1;
        cursor.check_expression_depth(depth)?;
        let start = left.expression.span_id;
        let end = right.expression.span_id;
        let expression = match operator.value {
            ReductionOperator::Binary(binary) => Expr::Binary {
                left: Box::new(left.expression),
                operator: Spanned::new(binary, operator.span_id),
                right: Box::new(right.expression),
            },
            ReductionOperator::Range => Expr::Range {
                start: Box::new(left.expression),
                to_keyword: operator.span_id,
                end: Box::new(right.expression),
            },
            ReductionOperator::Cast => Expr::Cast {
                value: Box::new(left.expression),
                as_keyword: operator.span_id,
                target: Box::new(right.expression),
            },
        };
        let expression = cursor.node(expression, start, end);
        self.values.push(Value { expression, depth });
        Ok(())
    }

    fn finish_argument(&mut self, cursor: &mut Cursor<'_, '_>) -> ParseResult<()> {
        let value = self.finish(cursor)?;
        self.arguments_depth = self.arguments_depth.max(value.depth);
        let end = value.expression.span_id;
        let argument = match self.argument_name.take() {
            Some(name) => {
                let start = name.span_id;
                let typed = std::mem::take(&mut self.argument_typed);
                cursor.node(
                    if typed {
                        Argument::Typed { name, annotation: value.expression }
                    } else {
                        Argument::Named { name, value: value.expression }
                    },
                    start,
                    end,
                )
            }
            None => Spanned::new(Argument::Positional(value.expression), end),
        };
        self.arguments.push(argument);
        Ok(())
    }
}

impl Cursor<'_, '_> {
    pub fn parse_expression(&mut self) -> ParseResult<Expression> {
        Ok(self
            .parse_expression_inner(None, ExpressionEnd::Line)?
            .expression)
    }

    pub(crate) fn parse_domain_expression(&mut self) -> ParseResult<Expression> {
        Ok(self
            .parse_expression_inner(None, ExpressionEnd::Domain)?
            .expression)
    }

    pub(crate) fn parse_condition(&mut self) -> ParseResult<Expression> {
        Ok(self
            .parse_expression_inner(None, ExpressionEnd::Condition)?
            .expression)
    }

    pub(crate) fn parse_return_part(&mut self) -> ParseResult<Value> {
        self.parse_expression_inner(None, ExpressionEnd::ReturnValue)
    }

    pub(crate) fn parse_parameter_default(&mut self) -> ParseResult<Expression> {
        Ok(self
            .parse_expression_inner(None, ExpressionEnd::ParameterDefault)?
            .expression)
    }

    pub fn parse_type_part(
        &mut self,
        layout: &mut TypeLayout,
        end: ExpressionEnd,
    ) -> ParseResult<Expression> {
        Ok(self.parse_expression_inner(Some(layout), end)?.expression)
    }

    pub fn parse_header_part(&mut self, end: ExpressionEnd) -> ParseResult<Expression> {
        assert!(end.is_header());
        Ok(self.parse_expression_inner(None, end)?.expression)
    }

    fn parse_expression_inner(
        &mut self,
        layout: Option<&mut TypeLayout>,
        end: ExpressionEnd,
    ) -> ParseResult<Value> {
        self.expression_layout_end = None;
        let start = self.position;
        let expression = self.parse_expression_frames(layout, end)?;
        if self.position > start && self.tokens[self.position - 1].value == Token::Dedent {
            self.expression_layout_end = Some(self.position);
        }
        Ok(expression)
    }

    fn parse_expression_frames(
        &mut self,
        mut layout: Option<&mut TypeLayout>,
        end: ExpressionEnd,
    ) -> ParseResult<Value> {
        let mut frames = vec![Frame::new(FrameKind::Root)];
        if let Some(seed) = self.expression_seed.take() {
            frames[0].values.push(seed);
        }
        let steps_max = 8 * (self.tokens.len() - self.position)
            + usize::from(self.options.limits.parentheses_max)
            + 2;
        for _ in 0..steps_max {
            self.checkpoint()?;
            if self.case_step(&mut frames, end)? {
                continue;
            }
            if self.loop_step(&mut frames)? {
                continue;
            }
            if self.not_step(&mut frames)? {
                continue;
            }
            let nested = frames.len() > 1;
            let delimiter = frames.last().expect("frame").kind.delimiter();
            let delimited = delimiter.is_some();
            if delimited && self.expression_recovery_boundary() {
                self.close_unfinished_expressions(&mut frames)?;
                continue;
            }
            let frame = frames.last_mut().expect("root frame remains");
            if !nested
                && frame.needs_operand()
                && end.is_header()
                && self.skip_header_eof_newlines()?
            {
                continue;
            }
            // `is` ends a condition's subject only when the subject is a single operand.
            let at_end = if matches!(end, ExpressionEnd::Condition)
                && self.current() == Some(Token::Is)
            {
                !frame.connective_pending() && frame.prefixes.is_empty()
            } else {
                end.matches(self.current())
            };
            if !nested && (frame.layout_end || at_end) {
                return frame.finish_root(self);
            }
            if !nested
                && let Some(layout) = layout.as_deref_mut()
                && layout.continue_root(self, frame)?
            {
                continue;
            }
            if let Some(expression) = self.parse_expression_token(&mut frames, end)? {
                return Ok(expression);
            }
        }
        unreachable!("each expression step consumes a token or returns");
    }

    fn parse_expression_token(
        &mut self,
        frames: &mut Vec<Frame>,
        end: ExpressionEnd,
    ) -> ParseResult<Option<Value>> {
        let nested = frames.len() > 1;
        let frame = frames.last_mut().expect("root frame remains");
        let delimiter = frame.kind.delimiter();
        match self.current() {
            Some(Token::When) if frame.needs_operand() => self.open_case(frames, end)?,
            Some(Token::Not) if frame.needs_operand() && end.is_value() => {
                let keyword = self.advance()?.span_id;
                self.check_expression_depth(u16::try_from(frames.len() / 4 + 2).unwrap_or(u16::MAX))?;
                frames.push(Frame::new(FrameKind::Not(keyword)));
            }
            Some(Token::Is) if !frame.needs_operand() && end.is_value() => self.parse_is(frame)?,
            Some(Token::Newline) if delimiter.is_some() => self.parse_nested_newline(frame)?,
            Some(token) if Some(token) == delimiter => self.close_delimited_expression(frames)?,
            Some(Token::For | Token::While) if frame.needs_operand() => {
                self.open_loop(frames, end)?;
            }
            Some(Token::ParenOpen) => self.open_expression(frames)?,
            Some(Token::BracketOpen) if frame.needs_operand() => self.open_array(frames)?,
            Some(Token::ColonSemi)
                if matches!(frame.kind, FrameKind::Group(_) | FrameKind::Collection(_)) =>
            {
                self.collection_length(frame)?;
            }
            Some(Token::Comma)
                if matches!(frame.kind, FrameKind::Group(_) | FrameKind::Collection(_)) =>
            {
                self.collection_comma(frame)?;
            }
            _ if frame.needs_operand() => {
                let type_prefixes = end.allows_type_prefixes() || frame.allows_type_prefixes();
                // A borrow is a value: not in a type, a domain or a refinement.
                let borrows = end.is_value();
                self.parse_operand(frame, type_prefixes, borrows)?;
            }
            Some(Token::Dot) if self.peek(1) == Some(Token::BracketOpen) => {
                self.open_selection(frames)?;
            }
            Some(Token::Dot) if matches!(self.peek(1), Some(Token::Int | Token::Float)) => {
                self.parse_numeric_select(frame)?;
            }
            Some(Token::Dot) => self.parse_member(frame)?,
            Some(token)
                if binary_operator(token).is_some() || matches!(token, Token::To | Token::As) =>
            {
                self.parse_binary_operator(frame, token)?;
            }
            Some(Token::Comma) if matches!(frame.kind, FrameKind::Call { .. }) => {
                frame.finish_argument(self)?;
                self.advance()?;
            }
            _ if !nested => return frame.finish_root(self).map(Some),
            _ => return Err(self.error("unsupported syntax or missing argument separator")),
        }
        Ok(None)
    }

    /// `not <operand>` ends where the condition it covers ends: at `and` or `or`, at a closing
    /// delimiter, or where the line, the `then`, the `do` or the `else` ends it.
    fn not_step(&mut self, frames: &mut Vec<Frame>) -> ParseResult<bool> {
        let frame = frames.last().expect("root remains");
        let FrameKind::Not(keyword) = frame.kind else {
            return Ok(false);
        };
        let ends = matches!(
            self.current(),
            None | Some(
                Token::Newline
                    | Token::Dedent
                    | Token::And
                    | Token::Or
                    | Token::Then
                    | Token::Do
                    | Token::Else
                    | Token::ParenClose
                    | Token::BracketClose
                    | Token::CurlyClose
                    | Token::Comma
                    | Token::Colon
                    | Token::ColonColon
            )
        ) || frame.layout_end;
        if !ends {
            return Ok(false);
        }
        let mut frame = frames.pop().expect("not frame");
        let operand = frame.finish(self)?;
        let depth = operand.depth + 1;
        self.check_expression_depth(depth)?;
        let end = operand.expression.span_id;
        let operator = Spanned::new(ast::UnaryOperator::Not, keyword);
        let expression = self.node(
            Expr::Unary {
                operator,
                operand: Box::new(operand.expression),
            },
            keyword,
            end,
        );
        frames
            .last_mut()
            .expect("a not has a parent")
            .values
            .push(Value { expression, depth });
        Ok(true)
    }

    /// `subject is Pattern`, a test with the precedence of a comparison.
    fn parse_is(&mut self, frame: &mut Frame) -> ParseResult<()> {
        frame.finish_prefixes(self)?;
        for _ in 0..frame.operators.len() {
            let previous = frame.operators.last().expect("remaining operator").value;
            if previous.precedence() < 3 {
                break;
            }
            frame.reduce(self)?;
        }
        let subject = frame.values.pop().expect("an operand precedes `is`");
        let is_keyword = self.advance()?.span_id;
        let (pattern, pattern_depth) = self.parse_match_pattern()?;
        let depth = subject.depth.max(pattern_depth) + 1;
        self.check_expression_depth(depth)?;
        let start = subject.expression.span_id;
        let end = pattern.span_id;
        let expression = self.node(
            Expr::IsTest(Box::new(ast::IsTest {
                subject: subject.expression,
                is_keyword,
                pattern,
            })),
            start,
            end,
        );
        frame.values.push(Value { expression, depth });
        Ok(())
    }

    fn parse_operand(
        &mut self,
        frame: &mut Frame,
        type_prefixes: bool,
        borrows: bool,
    ) -> ParseResult<()> {
        assert!(frame.needs_operand());
        if self.begin_named_argument(frame, type_prefixes)? {
            return Ok(());
        }
        if matches!(self.current(), Some(Token::Minus | Token::Deref))
            || self.current() == Some(Token::Domain) && self.peek(1) == Some(Token::Of)
        {
            return self.parse_unary_prefix(frame);
        }
        if self.current() == Some(Token::At) && frame.allows_pointer_arguments() {
            return self.parse_type_prefix(frame);
        }
        if type_prefixes
            && matches!(
                self.current(),
                Some(Token::Ref | Token::Mut | Token::Own | Token::At)
            )
        {
            return self.parse_type_prefix(frame);
        }
        // In a value position `ref place` and `mut place` create a borrow of the place, and
        // `@place` a raw pointer to it.
        if borrows && matches!(self.current(), Some(Token::Ref | Token::Mut | Token::At)) {
            return self.parse_type_prefix(frame);
        }
        let value = self.parse_atom()?;
        frame.values.push(value);
        assert!(!frame.needs_operand());
        Ok(())
    }

    fn skip_header_eof_newlines(&mut self) -> ParseResult<bool> {
        if self.current() != Some(Token::Newline) {
            return Ok(false);
        }
        for token in &self.tokens[self.position..] {
            self.options.checkpoint(self.spans.get(token.span_id))?;
            match token.value {
                Token::Newline => {}
                Token::Dedent if self.spans.get(token.span_id).start() == self.source.len() => {}
                _ => return Ok(false),
            }
        }
        self.skip_expression_newlines()?;
        Ok(true)
    }

    fn parse_nested_newline(&mut self, frame: &mut Frame) -> ParseResult<()> {
        assert!(!matches!(frame.kind, FrameKind::Root));
        assert_eq!(self.current(), Some(Token::Newline));
        if matches!(frame.kind, FrameKind::Call { .. }) && !frame.needs_operand() {
            frame.finish_argument(self)?;
        }
        if matches!(frame.kind, FrameKind::Collection(_)) && !frame.needs_operand() {
            self.collection_newline(frame)?;
        }
        self.skip_expression_newlines()
    }

    fn close_delimited_expression(&mut self, frames: &mut Vec<Frame>) -> ParseResult<()> {
        assert!(frames.len() > 1);
        assert_eq!(
            self.current(),
            frames.last().expect("frame").kind.delimiter()
        );
        let close = self.tokens[self.position].span_id;
        let frame = frames.pop().expect("nested frame");
        let value = self.close_expression(frame, close)?;
        self.advance()?;
        frames.last_mut().expect("parent frame").values.push(value);
        Ok(())
    }

    fn expression_recovery_boundary(&self) -> bool {
        match self.current() {
            None | Some(Token::Indent | Token::Dedent) => true,
            Some(Token::Newline) => {
                let next = self.tokens[self.position..]
                    .iter()
                    .position(|token| token.value != Token::Newline)
                    .map(|offset| self.position + offset);
                match next {
                    // With no following statement, the missing delimiter belongs
                    // at EOF, not before trailing layout inside the expression.
                    None => false,
                    Some(next) => {
                        let token = &self.tokens[next];
                        if token.value == Token::Dedent
                            && self.spans.get(token.span_id).start() == self.source.len()
                        {
                            false
                        } else {
                            matches!(token.value, Token::Indent | Token::Dedent)
                                || self.starts_statement(next)
                        }
                    }
                }
            }
            _ => false,
        }
    }

    fn close_unfinished_expressions(&mut self, frames: &mut Vec<Frame>) -> ParseResult<()> {
        assert!(frames.len() > 1);
        for _ in 1..frames.len() {
            let Some(delimiter) = frames.last().expect("frame").kind.delimiter() else {
                break;
            };
            self.checkpoint()?;
            let message = if delimiter == Token::BracketClose {
                "expected ']' before the end of this expression"
            } else {
                "expected ')' before the end of this expression"
            };
            self.report(message)?;
            let close = self.missing_span();
            let frame = frames.pop().expect("nested delimiter");
            let value = self.close_expression(frame, close)?;
            frames.last_mut().expect("parent frame").values.push(value);
        }
        Ok(())
    }

    fn skip_expression_newlines(&mut self) -> ParseResult<()> {
        assert_eq!(self.current(), Some(Token::Newline));
        for _ in self.position..self.tokens.len() {
            if self.current() != Some(Token::Newline) {
                break;
            }
            self.advance()?;
        }
        assert_ne!(self.current(), Some(Token::Newline));
        Ok(())
    }

    fn missing_expression(&mut self) -> ParseResult<Value> {
        self.report("expected an expression")?;
        let span = self.missing_span();
        Ok(Value {
            expression: Spanned::new(Expr::Missing, span),
            depth: 1,
        })
    }

    pub(crate) fn check_expression_depth(&self, depth: u16) -> ParseResult<()> {
        if depth <= self.options.limits.expression_depth_max {
            return Ok(());
        }
        Err(ParseFailure::LimitExceeded {
            limit: ParseLimit::ExpressionDepth,
            maximum: u32::from(self.options.limits.expression_depth_max),
            span: self.current_span(),
        }
        .into())
    }

    fn open_expression(&mut self, frames: &mut Vec<Frame>) -> ParseResult<()> {
        self.check_delimiter_depth(frames)?;
        let open = self.advance()?.span_id;
        let parent = frames.last_mut().expect("root frame remains");
        let type_prefixes = parent.allows_type_prefixes();
        let pointer_arguments = parent.allows_pointer_arguments();
        let kind = if parent.needs_operand() {
            FrameKind::Group(open)
        } else {
            FrameKind::Call {
                callee: parent.values.pop().expect("callee expression"),
                open,
            }
        };
        let binds = matches!(kind, FrameKind::Group(_))
            && self.current() == Some(Token::Id)
            && self.peek(1) == Some(Token::In);
        let mut frame = Frame::new(kind);
        frame.type_prefixes = type_prefixes;
        frame.pointer_arguments = pointer_arguments;
        if binds {
            let name = self.parse_name()?;
            let Some(annotation) = self.parse_optional_annotation(false)? else {
                return Err(self.error("expected a domain after `in`"));
            };
            if self.current() != Some(Token::Colon) {
                return Err(self.error("expected ':' and a value after the domain of a binding"));
            }
            let operator = Spanned::new(ast::BindingOperator::Colon, self.advance()?.span_id);
            frame.bind = Some(Box::new(BindHead {
                name,
                annotation,
                operator,
            }));
        }
        frames.push(frame);
        Ok(())
    }

    fn check_delimiter_depth(&self, frames: &[Frame]) -> ParseResult<()> {
        let delimiters = frames
            .iter()
            .filter(|frame| frame.kind.delimiter().is_some())
            .count();
        if delimiters == usize::from(self.options.limits.parentheses_max) {
            return Err(ParseFailure::LimitExceeded {
                limit: ParseLimit::Parentheses,
                maximum: u32::from(self.options.limits.parentheses_max),
                span: self.current_span(),
            }
            .into());
        }
        Ok(())
    }

    fn close_expression(&mut self, mut frame: Frame, close: SpanId) -> ParseResult<Value> {
        if matches!(frame.kind, FrameKind::Collection(_)) {
            return self.close_collection(frame, close);
        }
        if let FrameKind::Group(open) = frame.kind
            && frame.values.is_empty()
            && frame.prefixes.is_empty()
            && frame.operators.is_empty()
        {
            let expression = self.node(
                Expr::Sequence(Box::new(ast::SequenceExpression {
                    delimiters: ast::SequenceDelimiters::Parenthesized { open, close },
                    elements: Vec::new(),
                    separators: Vec::new(),
                })),
                open,
                close,
            );
            self.check_expression_depth(1)?;
            return Ok(Value {
                expression,
                depth: 1,
            });
        }
        if matches!(frame.kind, FrameKind::Group(_)) {
            let inner = frame.finish(self)?;
            let FrameKind::Group(open) = frame.kind else {
                unreachable!();
            };
            let depth = inner.depth + 1;
            self.check_expression_depth(depth)?;
            let inner_expression = match frame.bind.take() {
                Some(head) => {
                    let start = head.name.span_id;
                    let end = inner.expression.span_id;
                    let binding = ast::Binding {
                        attributes: Vec::new(),
                        name: head.name,
                        parameters: None,
                        convention: None,
                        result_binder: None,
                        annotation: Some(head.annotation),
                        contract: None,
                        alternatives: Vec::new(),
                        extra_results: Vec::new(),
                        value: Some(ast::BindingValue {
                            operator: head.operator,
                            expression: inner.expression,
                        }),
                    };
                    self.node(Expr::Bind(Box::new(binding)), start, end)
                }
                None => inner.expression,
            };
            let expression = self.node(Expr::Group(Box::new(inner_expression)), open, close);
            return Ok(Value { expression, depth });
        }
        if frame.argument_started() {
            frame.finish_argument(self)?;
        }
        let FrameKind::Call { callee, open } = frame.kind else {
            unreachable!("only nested delimiters close here");
        };
        let depth = callee.depth.max(frame.arguments_depth) + 1;
        self.check_expression_depth(depth)?;
        let start = callee.expression.span_id;
        let arguments = self.node(frame.arguments, open, close);
        let expression = self.node(
            Expr::Call {
                callee: Box::new(callee.expression),
                arguments,
            },
            start,
            close,
        );
        Ok(Value { expression, depth })
    }

    fn begin_named_argument(&mut self, frame: &mut Frame, in_type: bool) -> ParseResult<bool> {
        let unstarted = match &frame.kind {
            FrameKind::Call { .. } => !frame.argument_started(),
            FrameKind::Collection(collection) if collection.is_array() => {
                !frame.expression_started() && frame.argument_name.is_none()
            }
            _ => false,
        };
        if unstarted
            && matches!(self.current(), Some(Token::Id))
            && self.peek(1) == Some(Token::Colon)
        {
            let name = self.parse_name()?;
            self.advance()?;
            frame.argument_name = Some(name);
            return Ok(true);
        }
        // `name Type`: a name, then a type written in place. Where a type is expected (a variant
        // of an `or` list, a result variant) the type may also be a lowercase type parameter
        // (`code element`) or a range (`proposal in 1 to 16`); in a value call `f(a b)` is still
        // not `f(a: b)`.
        if unstarted
            && matches!(&frame.kind, FrameKind::Call { .. })
            && matches!(self.current(), Some(Token::Id))
            && match self.peek(1) {
                Some(Token::Tag | Token::Ref | Token::Mut | Token::Own | Token::At | Token::BracketOpen) => true,
                Some(Token::Id | Token::In) => in_type,
                _ => false,
            }
        {
            let name = self.parse_name()?;
            if self.current() == Some(Token::In) {
                self.advance()?;
            }
            frame.argument_name = Some(name);
            frame.argument_typed = true;
            return Ok(true);
        }
        Ok(false)
    }

    fn parse_atom(&mut self) -> ParseResult<Value> {
        let expression = match self.current() {
            Some(Token::Id | Token::Tag | Token::SelfInstance | Token::SelfTag) => {
                Expr::Name(self.current_span().extract(self.source).to_owned())
            }
            Some(Token::Int) => Expr::Literal(LiteralKind::Integer),
            Some(Token::Float) => Expr::Literal(LiteralKind::Float),
            Some(Token::String) => Expr::Literal(LiteralKind::String),
            None
            | Some(
                Token::Newline
                | Token::Dedent
                | Token::Comma
                | Token::ParenClose
                | Token::BracketClose
                | Token::ColonSemi,
            ) => {
                return self.missing_expression();
            }
            Some(Token::Pound)
                if self.peek(1) == Some(Token::Id) && self.peek_spelling(1) == Some("todo") =>
            {
                let start = self.advance()?.span_id;
                let end = self.advance()?.span_id;
                return Ok(Value {
                    expression: self.node(Expr::Hole, start, end),
                    depth: 1,
                });
            }
            // `#name`, a compile-time query; `#todo` above is the hole.
            Some(Token::Pound) if self.peek(1) == Some(Token::Id) => {
                let hash = self.advance()?.span_id;
                let name = self.parse_name()?;
                let end = name.span_id;
                return Ok(Value {
                    expression: self.node(Expr::Query { hash, name }, hash, end),
                    depth: 1,
                });
            }
            _ => return Err(self.error("expected an expression; syntax may be unsupported")),
        };
        let span = self.advance()?.span_id;
        Ok(Value {
            expression: Spanned::new(expression, span),
            depth: 1,
        })
    }

    fn parse_member(&mut self, frame: &mut Frame) -> ParseResult<()> {
        let dot = self.advance()?.span_id;
        let name = match self.current() {
            Some(Token::Id | Token::Tag) => self.parse_name()?,
            _ => {
                self.report("expected a member name after '.'")?;
                let base = frame.values.pop().expect("member base");
                let depth = base.depth + 1;
                self.check_expression_depth(depth)?;
                let start = base.expression.span_id;
                let expression = self.node(
                    Expr::IncompleteMember {
                        base: Box::new(base.expression),
                    },
                    start,
                    dot,
                );
                frame.values.push(Value { expression, depth });
                return Ok(());
            }
        };
        let end = name.span_id;
        let base = frame.values.pop().expect("member base");
        let depth = base.depth + 1;
        self.check_expression_depth(depth)?;
        let start = base.expression.span_id;
        let expression = self.node(
            Expr::Member {
                base: Box::new(base.expression),
                name,
            },
            start,
            end,
        );
        frame.values.push(Value { expression, depth });
        Ok(())
    }

    fn parse_binary_operator(&mut self, frame: &mut Frame, token: Token) -> ParseResult<()> {
        frame.finish_prefixes(self)?;
        let operator = match token {
            Token::To => ReductionOperator::Range,
            Token::As => ReductionOperator::Cast,
            _ => ReductionOperator::Binary(binary_operator(token).expect("operator dispatch")),
        };
        for _ in 0..frame.operators.len() {
            let previous = frame.operators.last().expect("remaining operator").value;
            if previous.precedence() < operator.precedence() {
                break;
            }
            if let (ReductionOperator::Binary(previous), ReductionOperator::Binary(operator)) =
                (previous, operator)
                && is_comparison(previous)
                && is_comparison(operator)
            {
                return Err(self.error("comparison chains require explicit grouping"));
            }
            frame.reduce(self)?;
        }
        let span = self.advance()?.span_id;
        frame.operators.push(Spanned::new(operator, span));
        Ok(())
    }
}
