use ast::{Condition, Expr, SpanId, SubSpan, Typed, WhenArm, WhenExpr};
use diagnostic::DiagnosticCode;
use diagnostic::code::{ExpectedSyntaxCode, ParseCode, SyntaxCode};
use lexer::Token;

use super::ExprFn;
use super::when_pattern::pattern_condition;
use crate::cursor::TokenCursor;

enum WhenHead {
    Condition(Condition),
    Patterns {
        prefix: Option<Condition>,
        subject: Typed<Expr>,
    },
}

enum WhenAlternative {
    Arm(WhenArm),
    Nested(WhenBranches),
}

struct WhenBranches {
    subject: Option<Typed<Expr>>,
    patterns: bool,
    prefix: Option<Condition>,
    seen_else: bool,
    arms_start: usize,
}

fn conjunction(prefix: Option<Condition>, condition: Condition) -> Condition {
    match prefix {
        Some(prefix) => Condition::And(Box::new(prefix), Box::new(condition)),
        None => condition,
    }
}

impl TokenCursor<'_, '_> {
    pub fn parse_when_expr(&mut self, expr_parser: ExprFn) -> Option<WhenExpr> {
        let start = self.current_span();
        if !self.eat(&Token::When) {
            return None;
        }
        if self.eat(&Token::Indent) {
            return self.parse_when_branches(expr_parser, None, false, None, start);
        }
        let first = self.parse_when_atom(expr_parser, false)?;
        if let WhenHead::Condition(Condition::Proposition { subject }) = &first {
            let continuation = self.pos();
            if self.eat(&Token::Indent) {
                if !self.is_at(&Token::Then) {
                    return self.parse_when_branches(
                        expr_parser,
                        Some(*subject.clone()),
                        false,
                        None,
                        start,
                    );
                }
                self.rewind(continuation);
            }
        }
        match self.parse_when_head_rest(expr_parser, first)? {
            WhenHead::Condition(condition) => self.parse_when_single(expr_parser, condition, start),
            WhenHead::Patterns { prefix, subject } => {
                self.parse_when_branches(expr_parser, Some(subject), true, prefix, start)
            }
        }
    }

    fn parse_when_atom(&mut self, expr_parser: ExprFn, continuation: bool) -> Option<WhenHead> {
        if self.is_at(&Token::Not) || self.parenthesized_when_condition() {
            return self
                .parse_condition_atom(expr_parser)
                .map(WhenHead::Condition);
        }
        let subject = self.parse_expr_min_prec_layout(3, continuation);
        if self.eat(&Token::Is) {
            if self.eat(&Token::Indent) {
                return Some(WhenHead::Patterns {
                    prefix: None,
                    subject,
                });
            }
            if matches!(self.raw_peek(), Some(Token::Newline)) {
                // parse-expected-indent
                self.error_here(
                    DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::Indentation)),
                    "indent the alternatives after 'is'",
                );
                return None;
            }
            let pattern = self.parse_condition_pattern(expr_parser)?;
            return Some(WhenHead::Condition(
                self.parse_when_pattern_condition(subject, pattern)?,
            ));
        }
        self.parse_condition_subject(expr_parser, subject)
            .map(WhenHead::Condition)
    }

    fn parse_when_pattern_condition(
        &mut self,
        subject: Typed<Expr>,
        pattern: ast::Spanned<ast::Pattern>,
    ) -> Option<Condition> {
        let condition = pattern_condition(subject, pattern);
        if condition.is_none() {
            // parse-pattern-too-large
            self.error_here(
                DiagnosticCode::Parse(ParseCode::Syntax(SyntaxCode::PatternTooLarge)),
                "constructor pattern exceeds the supported size",
            );
        }
        condition
    }

    fn parse_when_head_rest(&mut self, expr_parser: ExprFn, first: WhenHead) -> Option<WhenHead> {
        let WhenHead::Condition(mut condition) = first else {
            return Some(first);
        };
        let mut alternatives: Option<Condition> = None;
        for _ in 0..self.tokens_remaining() {
            let and = self.eat(&Token::And);
            if !and && !self.eat(&Token::Or) {
                break;
            }
            if !and {
                alternatives = Some(match alternatives {
                    Some(previous) => {
                        Condition::Or(Box::new(previous), Box::new(condition.clone()))
                    }
                    None => condition.clone(),
                });
            }
            match self.parse_when_atom(expr_parser, true)? {
                WhenHead::Condition(next) => {
                    condition = if and {
                        conjunction(Some(condition), next)
                    } else {
                        next
                    };
                }
                WhenHead::Patterns { subject, .. } => {
                    if !and || alternatives.is_some() {
                        // parse-nested-is-disjunction
                        self.error_here(
                            DiagnosticCode::Parse(ParseCode::Syntax(
                                SyntaxCode::NestedIsDisjunction,
                            )),
                            "parenthesize alternatives before a nested 'is' block",
                        );
                        return None;
                    }
                    return Some(WhenHead::Patterns {
                        prefix: Some(condition),
                        subject,
                    });
                }
            }
        }
        if let Some(previous) = alternatives {
            condition = Condition::Or(Box::new(previous), Box::new(condition));
        }
        Some(WhenHead::Condition(condition))
    }

    fn parse_when_branches(
        &mut self,
        expr_parser: ExprFn,
        subject: Option<Typed<Expr>>,
        patterns: bool,
        prefix: Option<Condition>,
        start: SpanId,
    ) -> Option<WhenExpr> {
        let root_subject = if prefix.is_none() {
            subject.clone()
        } else {
            None
        };
        let mut frames = vec![WhenBranches {
            subject,
            patterns,
            prefix,
            seen_else: false,
            arms_start: 0,
        }];
        let mut arms = Vec::new();
        for _ in 0..self.tokens_remaining() {
            self.skip_newlines();
            if self.eat(&Token::Dedent) {
                let frame = frames.pop()?;
                if frame.arms_start == arms.len() {
                    // parse-empty-when
                    self.error_here(
                        DiagnosticCode::Parse(ParseCode::Syntax(SyntaxCode::EmptyWhen)),
                        "an 'is' or 'when' block requires an alternative",
                    );
                    return None;
                }
                if frames.is_empty() {
                    return Some(WhenExpr {
                        subject: root_subject.map(Box::new),
                        arms,
                        body_span: SubSpan::new(self.merge_span(start, self.last_consumed_span())),
                    });
                }
                continue;
            }
            let frame = frames.last_mut()?;
            match self.parse_when_alternative(
                expr_parser,
                frame,
                root_subject.as_ref(),
                arms.len(),
            )? {
                WhenAlternative::Arm(arm) => arms.push(arm),
                WhenAlternative::Nested(frame) => frames.push(frame),
            }
        }
        // parse-unclosed-when
        self.error_here(
            DiagnosticCode::Parse(ParseCode::Syntax(SyntaxCode::UnclosedWhen)),
            "expected the end of the 'when' block",
        );
        None
    }

    fn parse_when_alternative(
        &mut self,
        expr_parser: ExprFn,
        frame: &mut WhenBranches,
        root_subject: Option<&Typed<Expr>>,
        arms_start: usize,
    ) -> Option<WhenAlternative> {
        if frame.seen_else {
            // parse-misplaced-else
            self.error_here(
                DiagnosticCode::Parse(ParseCode::Syntax(SyntaxCode::MisplacedElse)),
                "'else' must be the last alternative in its block",
            );
            return None;
        }
        let start = self.current_span();
        let aligned_else = self.is_at(&Token::Indent) && self.peek_at(1) == Some(&Token::Else);
        if aligned_else {
            self.advance();
        }
        if self.eat(&Token::Else) {
            frame.seen_else = true;
            let body = self.parse_branch_body(expr_parser)?;
            let span = SubSpan::new(self.merge_span(start, body.span_id));
            let arm = match &frame.prefix {
                Some(condition) => WhenArm::Cond {
                    condition: condition.clone(),
                    body: Box::new(body),
                    arm_span: span,
                },
                None => WhenArm::Else(Box::new(body), span),
            };
            if aligned_else {
                self.expect(&Token::Dedent)?;
            }
            return Some(WhenAlternative::Arm(arm));
        }
        let first = self.parse_when_branch_head(expr_parser, frame)?;
        match self.parse_when_head_rest(expr_parser, first)? {
            WhenHead::Patterns { prefix, subject } => Some(WhenAlternative::Nested(WhenBranches {
                subject: Some(subject),
                patterns: true,
                prefix: match prefix {
                    Some(condition) => Some(conjunction(frame.prefix.clone(), condition)),
                    None => frame.prefix.clone(),
                },
                seen_else: false,
                arms_start,
            })),
            WhenHead::Condition(condition) => {
                let condition = conjunction(frame.prefix.clone(), condition);
                self.parse_when_branch_body(expr_parser, condition, root_subject, start)
                    .map(WhenAlternative::Arm)
            }
        }
    }

    fn parse_when_branch_head(
        &mut self,
        expr_parser: ExprFn,
        frame: &WhenBranches,
    ) -> Option<WhenHead> {
        let Some(subject) = &frame.subject else {
            return self.parse_when_atom(expr_parser, true);
        };
        if frame.patterns {
            let pattern = self.parse_condition_pattern(expr_parser)?;
            let mut condition = self.parse_when_pattern_condition(subject.clone(), pattern)?;
            for _ in 0..self.tokens_remaining() {
                if !self.eat(&Token::Or) {
                    break;
                }
                let pattern = self.parse_condition_pattern(expr_parser)?;
                condition = Condition::Or(
                    Box::new(condition),
                    Box::new(self.parse_when_pattern_condition(subject.clone(), pattern)?),
                );
            }
            return Some(WhenHead::Condition(condition));
        }
        let relation = match self.advance()?.0 {
            Token::Less => ast::ProofRelation::Less,
            Token::LessEq => ast::ProofRelation::LessOrEqual,
            Token::Greater => ast::ProofRelation::Greater,
            Token::GreaterEq => ast::ProofRelation::GreaterOrEqual,
            Token::Eq => ast::ProofRelation::Equal,
            _ => {
                // parse-expected-comparison
                self.error_here(
                    DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::Comparison)),
                    "expected a comparison operator in a shared-subject 'when'",
                );
                return None;
            }
        };
        let right = self.parse_expr_min_prec(3);
        Some(WhenHead::Condition(Condition::Compare {
            left: Box::new(subject.clone()),
            relation,
            right: Box::new(right),
        }))
    }

    fn parse_when_branch_body(
        &mut self,
        expr_parser: ExprFn,
        condition: Condition,
        root_subject: Option<&Typed<Expr>>,
        start: SpanId,
    ) -> Option<WhenArm> {
        let continuation = self.eat(&Token::Indent);
        let WhenHead::Condition(condition) =
            self.parse_when_head_rest(expr_parser, WhenHead::Condition(condition))?
        else {
            // parse-expected-then
            self.error_here(
                DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::Then)),
                "expected 'then' before the branch body",
            );
            return None;
        };
        if continuation {
            self.eat(&Token::Dedent);
        }
        self.expect(&Token::Then)?;
        let body = self.parse_branch_body(expr_parser)?;
        let arm_span = SubSpan::new(self.merge_span(start, body.span_id));
        if let Condition::Is { subject, pattern } = &condition {
            if root_subject.is_some_and(|root| root.span_id == subject.span_id) {
                return Some(WhenArm::Is {
                    pattern: pattern.clone(),
                    body: Box::new(body),
                    arm_span,
                });
            }
        }
        Some(WhenArm::Cond {
            condition,
            body: Box::new(body),
            arm_span,
        })
    }
    fn parse_when_single(
        &mut self,
        expr_parser: ExprFn,
        condition: Condition,
        start: SpanId,
    ) -> Option<WhenExpr> {
        let continuation = self.eat(&Token::Indent);
        self.expect(&Token::Then)?;
        let body = self.parse_branch_body(expr_parser)?;
        let span = SubSpan::new(self.merge_span(start, body.span_id));
        let (subject, arm) = match condition {
            Condition::Is { subject, pattern } => (
                Some(subject),
                WhenArm::Is {
                    pattern,
                    body: Box::new(body),
                    arm_span: span,
                },
            ),
            condition => (
                None,
                WhenArm::Cond {
                    condition,
                    body: Box::new(body),
                    arm_span: span,
                },
            ),
        };
        let mut arms = vec![arm];
        self.skip_newlines();
        let aligned_else = self.is_at(&Token::Indent) && self.peek_at(1) == Some(&Token::Else);
        if aligned_else {
            self.advance();
        }
        if self.eat(&Token::Else) {
            let start = self.last_consumed_span();
            let body = self.parse_branch_body(expr_parser)?;
            let span = SubSpan::new(self.merge_span(start, body.span_id));
            arms.push(WhenArm::Else(Box::new(body), span));
        }
        if aligned_else {
            self.expect(&Token::Dedent)?;
        }
        if continuation {
            self.expect(&Token::Dedent)?;
        }
        Some(WhenExpr {
            subject,
            arms,
            body_span: SubSpan::new(self.merge_span(start, self.last_consumed_span())),
        })
    }

    fn parse_branch_body(&mut self, expr_parser: ExprFn) -> Option<Typed<Expr>> {
        let multiline = matches!(self.raw_peek(), Some(Token::Newline));
        if self.eat(&Token::Indent) {
            let start = self.current_span();
            let items = self.parse_body_exprs(expr_parser);
            self.expect(&Token::Dedent)?;
            let span = self.merge_span(start, self.last_consumed_span());
            return Some(Typed::infer(Expr::Block(items), span));
        }
        if multiline {
            // parse-expected-indent
            self.error_here(
                DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::Indentation)),
                "indent the branch body after 'then' or 'else'",
            );
            return None;
        }
        if let Some(ret) =
            self.parse_return_with(|cursor| cursor.parse_expr_min_prec_layout(0, false))
        {
            let span = ret.span_id;
            return Some(Typed::infer(Expr::Return(ret), span));
        }
        Some(self.parse_expr_min_prec_layout(0, false))
    }
}
