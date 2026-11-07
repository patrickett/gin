use ast::{
    Binding, BindingConvention, BindingOperator, BindingValue, Block, ClauseItem, Contract,
    Convention, Expr, ExtraResult, Name, Parameter, ParameterConvention, RefinementPredicate,
    ResultAlternative, SpanId, Spanned, Transition,
};
use lexer::TokenKind as Token;

use crate::cursor::{Cursor, ParseResult};
use crate::file::PendingBody;

pub(crate) enum ParsedBinding {
    Complete(Spanned<Binding>),
    Body(PendingBody),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BindingContext {
    Value,
    Member,
}

pub(crate) fn header_end(binding: &Binding) -> SpanId {
    binding
        .alternatives
        .last()
        .map(|alternative| alternative.span_id)
        .or(binding
            .contract
            .as_ref()
            .map(|clause| clause.span_id))
        .or(binding.extra_results.last().map(|extra| extra.span_id))
        .or(binding
            .annotation
            .as_ref()
            .map(|annotation| annotation.span_id))
        .or(binding.result_binder.as_ref().map(|name| name.span_id))
        .or(binding
            .convention
            .as_ref()
            .map(|convention| convention.span_id))
        .or(binding
            .parameters
            .as_ref()
            .map(|parameters| parameters.span_id))
        .unwrap_or(binding.name.span_id)
}

impl Cursor<'_, '_> {
    pub fn starts_binding(&self) -> bool {
        if !matches!(self.current(), Some(Token::Id)) {
            return false;
        }
        if self.peek(1) != Some(Token::ParenOpen) {
            return self.binding_follows(self.position + 1);
        }
        match self.parenthesis_closes[self.position + 1] {
            Some(close) => self.binding_follows(close + 1),
            None => false,
        }
    }

    pub fn parse_binding(&mut self, context: BindingContext) -> ParseResult<ParsedBinding> {
        assert_eq!(self.current(), Some(Token::Id));
        let mut binding = self.parse_binding_header(context)?;
        let start = binding.name.span_id;
        let operator = match self.current() {
            Some(Token::Colon) => BindingOperator::Colon,
            Some(Token::ColonEq) => BindingOperator::ColonEquals,
            _ => {
                let end = header_end(&binding);
                return Ok(ParsedBinding::Complete(self.node(binding, start, end)));
            }
        };
        let operator = Spanned::new(operator, self.advance()?.span_id);
        if self.current() == Some(Token::Newline) {
            let next = self.tokens[self.position..]
                .iter()
                .position(|token| token.value != Token::Newline)
                .map(|offset| self.position + offset);
            if !next.is_some_and(|next| self.tokens[next].value == Token::Indent) {
                self.report("expected an initializer or indented binding body")?;
                let span = self.missing_span();
                let expression = Spanned::new(Expr::Missing, span);
                binding.value = Some(BindingValue {
                    operator,
                    expression,
                });
                return Ok(ParsedBinding::Complete(self.node(binding, start, span)));
            }
            self.position = next.expect("confirmed body indentation");
            self.advance()?;
            return Ok(ParsedBinding::Body(PendingBody {
                binding,
                operator,
                statements: Vec::new(),
            }));
        }
        let expression = if self.current() == Some(Token::Return) {
            let statement = self.parse_return()?;
            let span = statement.span_id;
            Spanned::new(
                Expr::Block(Block {
                    statements: vec![statement],
                }),
                span,
            )
        } else {
            self.parse_expression()?
        };
        let end = expression.span_id;
        binding.value = Some(BindingValue {
            operator,
            expression,
        });
        Ok(ParsedBinding::Complete(self.node(binding, start, end)))
    }

    pub(crate) fn starts_binding_parameters(&self) -> bool {
        // Declaration-only adjacency: `f(` has parameters; `field (` has an annotation.
        // Expression calls do not use this rule.
        self.current() == Some(Token::Id)
            && self.peek(1) == Some(Token::ParenOpen)
            && self.current_span().end()
                == self
                    .spans
                    .get(self.tokens[self.position + 1].span_id)
                    .start()
    }

    pub(crate) fn parse_binding_header(&mut self, context: BindingContext) -> ParseResult<Binding> {
        let has_parameters = self.starts_binding_parameters();
        let name = self.parse_name()?;
        let parameters = if has_parameters {
            Some(self.parse_parameters(true)?)
        } else {
            None
        };
        let result_header = context == BindingContext::Value || parameters.is_some();
        if self.at_end_restriction() && !result_header {
            return Err(self.error("end restrictions on fields are not supported"));
        }
        if parameters.is_some() && self.at_end_restriction() {
            return Err(self.error("end restricts an owner input or local, not a result"));
        }
        let convention = if result_header {
            self.parse_optional_binding_convention()?
        } else {
            None
        };
        let result_binder = if result_header && self.starts_result_binder() {
            Some(self.parse_name()?)
        } else {
            None
        };
        if self.at_end_restriction() {
            return Err(self.error("end restricts an owner input or local, not a result"));
        }
        let annotation = self.parse_optional_annotation(parameters.is_some())?;
        let extra_results = self.parse_extra_results(parameters.is_some())?;
        let contract = self.parse_contract(result_header)?;
        self.require_grouped_claims(contract.as_ref())?;
        let alternatives = self.parse_result_alternatives(parameters.is_some())?;
        Ok(Binding {
            attributes: Vec::new(),
            name,
            parameters,
            convention,
            result_binder,
            annotation,
            contract,
            alternatives,
            extra_results,
            value: None,
        })
    }

    fn starts_result_binder(&self) -> bool {
        if self.current() == Some(Token::Id) && self.peek(1) == Some(Token::ParenOpen) {
            let name = self.spans.get(self.tokens[self.position].span_id);
            let open = self.spans.get(self.tokens[self.position + 1].span_id);
            return name.end() < open.start();
        }
        self.current() == Some(Token::Id)
            && !self.at_end_restriction()
            && matches!(
                self.peek(1),
                Some(
                    Token::Id
                        | Token::In
                        | Token::Tag
                        | Token::SelfTag
                        | Token::Ref
                        | Token::Mut
                        | Token::Own
                        | Token::At
                        | Token::BracketOpen
                )
            )
    }

    fn at_end_restriction(&self) -> bool {
        self.current() == Some(Token::Id) && self.peek_spelling(0) == Some("end")
    }

    /// `where item, item`: the claims and transitions that hold when the call returns. A claim
    /// is a proposition, or a leading comparison of an unnamed result (`>= 10`); a transition is
    /// `place :: value`. A disjunction inside a claim is parenthesized.
    // TODO(header-layout): once layout tracks continued headers, allow complete clauses on
    // indented continuation lines without commas. Inline clauses keep comma separators.
    // Retain item/joiner spans; this is layout syntax, not a change to contract checking.
    fn parse_contract(&mut self, result_header: bool) -> ParseResult<Option<Spanned<Contract>>> {
        if self.current() != Some(Token::Where) {
            return Ok(None);
        }
        if !result_header {
            return Err(self.error("where clauses are not field annotations"));
        }
        let where_keyword = self.advance()?.span_id;
        // A result variant's claims are parenthesized, and joined by `and` or `,`.
        let open = self.starts_claim_group().then_some(())
            .map(|()| self.advance().map(|token| token.span_id))
            .transpose()?;
        let mut items = Vec::new();
        let mut joiners = Vec::new();
        loop {
            items.push(self.parse_clause_item(open.is_some())?);
            if self.current() == Some(Token::Comma)
                || (open.is_some() && self.current() == Some(Token::And))
            {
                joiners.push(self.advance()?.span_id);
                // Between the parentheses of a variant's claims, the next claim may start a line.
                if open.is_some() {
                    self.skip_newlines();
                }
                continue;
            }
            break;
        }
        let close = if open.is_some() {
            if self.current() != Some(Token::ParenClose) {
                return Err(self.error("expected ')' after the claims of a result variant"));
            }
            Some(self.advance()?.span_id)
        } else {
            None
        };
        let (start, end) = (where_keyword, close.unwrap_or(items.last().expect("one").span_id));
        Ok(Some(self.node(
            Contract {
                where_keyword,
                open,
                items,
                joiners,
                close,
            },
            start,
            end,
        )))
    }

    /// Whether a `:` that opens a body follows on this line, outside any brackets. A header
    /// in a list of members written on one line (`Thing has a() Int, b Int`) has none, and its
    /// commas separate members, not results.
    fn body_follows_on_line(&self) -> bool {
        let mut depth = 0usize;
        for token in &self.tokens[self.position..] {
            match token.value {
                Token::ParenOpen | Token::BracketOpen | Token::CurlyOpen => depth += 1,
                Token::ParenClose | Token::BracketClose | Token::CurlyClose => {
                    depth = depth.saturating_sub(1)
                }
                Token::Colon if depth == 0 => return true,
                Token::Newline | Token::Dedent => return false,
                _ => {}
            }
        }
        false
    }

    /// Whether the name `offset` tokens ahead is followed at once by `(`.
    fn name_adjoins_paren(&self, offset: usize) -> bool {
        let (Some(name), Some(open)) = (
            self.tokens.get(self.position + offset),
            self.tokens.get(self.position + offset + 1),
        ) else {
            return false;
        };
        open.value == Token::ParenOpen
            && self.spans.get(name.span_id).end() == self.spans.get(open.span_id).start()
    }

    /// `, name Type`, repeated: the further named results of a function that returns several.
    fn parse_extra_results(&mut self, function: bool) -> ParseResult<Vec<Spanned<ExtraResult>>> {
        let mut extras = Vec::new();
        while function
            && self.current() == Some(Token::Comma)
            && self.peek(1) == Some(Token::Id)
            // `second(..)` written against its name is the next member, not a result.
            && !self.name_adjoins_paren(1)
            && self.body_follows_on_line()
            && matches!(
                self.peek(2),
                Some(
                    Token::Id
                        | Token::Tag
                        | Token::In
                        | Token::ParenOpen
                        | Token::BracketOpen
                        | Token::Ref
                        | Token::Mut
                        | Token::Own
                        | Token::At
                )
            )
        {
            let comma = self.advance()?.span_id;
            let name = self.parse_name()?;
            let Some(annotation) = self.parse_optional_annotation(false)? else {
                return Err(self.error("expected a type after the name of a result"));
            };
            let (start, end) = (comma, annotation.span_id);
            extras.push(self.node(
                ExtraResult {
                    comma,
                    name,
                    annotation,
                },
                start,
                end,
            ));
        }
        Ok(extras)
    }

    /// `or Variant [where claims]`, repeated, after the first variant of a function's result.
    /// The `or` ends its line and the next variant continues on the next one.
    fn parse_result_alternatives(
        &mut self,
        function: bool,
    ) -> ParseResult<Vec<Spanned<ResultAlternative>>> {
        let mut alternatives = Vec::new();
        while function && self.current() == Some(Token::Or) {
            let or_keyword = self.advance()?.span_id;
            while matches!(self.current(), Some(Token::Newline | Token::Indent)) {
                self.advance()?;
            }
            let Some(annotation) = self.parse_optional_annotation(true)? else {
                return Err(self.error("expected a result variant after `or`"));
            };
            let contract = self.parse_contract(true)?;
            self.require_grouped_claims(contract.as_ref())?;
            let end = contract.as_ref().map_or(annotation.span_id, |c| c.span_id);
            alternatives.push(self.node(
                ResultAlternative {
                    or_keyword,
                    annotation,
                    contract,
                },
                or_keyword,
                end,
            ));
        }
        Ok(alternatives)
    }

    /// A variant that another follows writes its claims in parentheses, so the `or` after them
    /// is not part of a claim.
    fn require_grouped_claims(&mut self, contract: Option<&Spanned<Contract>>) -> ParseResult<()> {
        if self.current() == Some(Token::Or)
            && contract.is_some_and(|contract| contract.value.open.is_none())
        {
            return Err(self.error("the claims of a result variant are written in parentheses"));
        }
        Ok(())
    }

    /// `(` that opens the claims of a result variant: its `)` ends the clause, so what follows is
    /// the next variant's `or`, the body's `:`, or the end of the line. A parenthesized
    /// proposition that something else follows (`(a or b) and c`) is just a claim.
    fn starts_claim_group(&self) -> bool {
        if self.current() != Some(Token::ParenOpen) {
            return false;
        }
        let Some(close) = self.parenthesis_closes[self.position] else {
            return false;
        };
        matches!(
            self.tokens.get(close + 1).map(|token| token.value),
            None | Some(
                Token::Or | Token::Colon | Token::ColonEq | Token::Newline | Token::Dedent
            )
        )
    }

    fn parse_clause_item(&mut self, grouped: bool) -> ParseResult<Spanned<ClauseItem>> {
        use crate::expr::ExpressionEnd::{HeaderGroupItem, HeaderPostcondition};
        // Inside parentheses the items are joined by `and`, so a claim ends at one.
        let end = if grouped { HeaderGroupItem } else { HeaderPostcondition };
        if self.starts_transition() {
            let place = self.parse_place()?;
            let start = place.span_id;
            let operator = self.advance()?.span_id;
            let value = self.parse_header_part(end)?;
            let end = value.span_id;
            return Ok(self.node(
                ClauseItem::Transition(Transition {
                    place,
                    operator,
                    value,
                }),
                start,
                end,
            ));
        }
        let comparison = self
            .current()
            .and_then(crate::operators::binary_operator)
            .filter(|operator| crate::operators::is_comparison(*operator));
        if let Some(operator) = comparison {
            let operator = Spanned::new(operator, self.advance()?.span_id);
            let start = operator.span_id;
            let right = self.parse_header_part(end)?;
            let end = right.span_id;
            return Ok(self.node(
                ClauseItem::Claim(RefinementPredicate::ImplicitComparison { operator, right }),
                start,
                end,
            ));
        }
        let claim = self.parse_header_part(end)?;
        let span = claim.span_id;
        Ok(Spanned::new(
            ClauseItem::Claim(RefinementPredicate::Expression(claim)),
            span,
        ))
    }

    /// A parameter or a field path of one, then `::`.
    fn starts_transition(&self) -> bool {
        if !matches!(self.current(), Some(Token::Id | Token::SelfInstance)) {
            return false;
        }
        let mut offset = 1;
        loop {
            match self.peek(offset) {
                Some(Token::ColonColon) => return true,
                Some(Token::Dot) if self.peek(offset + 1) == Some(Token::Id) => offset += 2,
                _ => return false,
            }
        }
    }

    fn parse_optional_binding_convention(
        &mut self,
    ) -> ParseResult<Option<Spanned<BindingConvention>>> {
        let convention = if self.at_end_restriction() {
            BindingConvention::End
        } else if self.current() == Some(Token::Own) {
            BindingConvention::Own
        } else {
            return Ok(None);
        };
        let span = self.advance()?.span_id;
        if self.current() == Some(Token::CurlyOpen) {
            return Err(self.error("binding conventions/restrictions do not take group paths"));
        }
        self.reject_following_end()?;
        Ok(Some(Spanned::new(convention, span)))
    }

    pub(crate) fn parse_parameters(
        &mut self,
        allow_receiver: bool,
    ) -> ParseResult<Spanned<Vec<Spanned<Parameter>>>> {
        let start = self.advance()?.span_id;
        let mut parameters = Vec::new();
        self.skip_list_layout();
        for _ in 0..=self.tokens.len() {
            if self.current() == Some(Token::ParenClose) {
                let end = self.advance()?.span_id;
                return Ok(self.node(parameters, start, end));
            }
            let named = self.current() == Some(Token::Id);
            let receiver = allow_receiver && self.current() == Some(Token::SelfInstance);
            if !named && !receiver {
                return Err(self.error("only named parameters are supported in this source slice"));
            }
            let receiver = self.current() == Some(Token::SelfInstance);
            let name = if receiver {
                let span = self.advance()?.span_id;
                Spanned::new("self".to_owned(), span)
            } else {
                self.parse_name()?
            };
            let parameter_start = name.span_id;
            let convention = self.parse_optional_convention()?;
            let annotation = self.parse_optional_annotation(false)?;
            let default = if self.current() == Some(Token::Colon) {
                if annotation.is_some() {
                    return Err(self.error("typed parameter defaults are not supported"));
                }
                self.advance()?;
                Some(self.parse_parameter_default()?)
            } else {
                None
            };
            let end = default
                .as_ref()
                .map(|expression| expression.span_id)
                .or_else(|| annotation.as_ref().map(|annotation| annotation.span_id))
                .or_else(|| convention.as_ref().map(|convention| convention.span_id))
                .unwrap_or(parameter_start);
            let parameter = Parameter {
                name,
                convention,
                annotation,
                default,
            };
            parameters.push(self.node(parameter, parameter_start, end));
            let newline = self.skip_list_layout();
            if self.current() == Some(Token::Comma) {
                self.advance()?;
                self.skip_list_layout();
            } else if self.current() != Some(Token::ParenClose) && !newline {
                return Err(self.error("expected ',' or newline between parameters"));
            }
        }
        unreachable!("each parameter consumes at least one token");
    }

    fn parse_optional_convention(&mut self) -> ParseResult<Option<Spanned<ParameterConvention>>> {
        let convention = match self.current() {
            Some(Token::Ref) => Convention::Ref,
            Some(Token::Mut) => Convention::Mut,
            Some(Token::Own) => Convention::Own,
            Some(Token::Id) if self.at_end_restriction() => Convention::End,
            _ => return Ok(None),
        };
        let span = self.advance()?.span_id;
        if convention == Convention::End && self.current() == Some(Token::CurlyOpen) {
            return Err(self.error("end restrictions do not take group paths"));
        }
        if convention == Convention::Own && self.current() == Some(Token::CurlyOpen) {
            self.report("group paths attach to ref or mut, not own")?;
        }
        let group = self.parse_optional_group()?;
        self.reject_following_end()?;
        let end = group.as_ref().map_or(span, |group| group.span_id);
        Ok(Some(Spanned::new(
            ParameterConvention {
                keyword: Spanned::new(convention, span),
                group,
            },
            self.spans.merge(span, end),
        )))
    }

    fn reject_following_end(&self) -> ParseResult<()> {
        if self.at_end_restriction() {
            return Err(self.error("end cannot be repeated or mixed with another convention"));
        }
        Ok(())
    }

    pub fn parse_name(&mut self) -> ParseResult<Name> {
        match self.current() {
            Some(Token::Id | Token::Tag) => {}
            _ => return Err(self.error("expected an identifier; syntax may be unsupported")),
        }
        let spelling = self.current_span().extract(self.source).to_owned();
        let span = self.advance()?.span_id;
        Ok(Spanned::new(spelling, span))
    }

    fn skip_list_layout(&mut self) -> bool {
        let next = self.tokens[self.position..]
            .iter()
            .position(|token| token.value != Token::Newline)
            .map_or(self.tokens.len(), |offset| self.position + offset);
        if self.starts_statement(next) {
            return false;
        }
        let newline = next > self.position;
        self.position = next;
        newline
    }

    fn binding_follows(&self, index: usize) -> bool {
        matches!(
            self.tokens.get(index).map(|token| token.value),
            Some(
                Token::Colon
                    | Token::ColonEq
                    | Token::Tag
                    | Token::Id
                    | Token::SelfTag
                    | Token::Is
                    | Token::In
                    | Token::Thus
                    | Token::Where
                    | Token::Ref
                    | Token::Mut
                    | Token::Own
                    | Token::At
                    | Token::ParenOpen
                    | Token::BracketOpen
            )
        )
    }
}
