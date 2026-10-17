use diagnostic::DiagnosticCode;
use diagnostic::code::{ExpectedSyntaxCode, ParseCode, RemovedSyntaxCode, SyntaxCode};
use indexmap::IndexMap;
use internment::Intern;
use lexer::Token;

use ast::{
    AttributeItem, Bind, BindAttributes, BindOperator, BindValue, COMPILER_QUERY_NAMES, DocComment,
    Expr, GroupParam, GroupPath, ModPath, NamedResult, ParamConvention, Parameter, ParameterKind,
    Parameters, Pattern, PredicateExpr, ProofProposition, ProofRelation, ProofTerm,
    ResultAlternative, Spanned, Typed,
};

use super::ExprFn;
use super::path_call;
use crate::cursor::TokenCursor;

type ReturnTypePart = (
    Option<Intern<String>>,
    Option<Box<Spanned<Expr>>>,
    Option<(Intern<String>, Vec<Typed<Expr>>)>,
    Option<Spanned<ModPath>>,
);

pub(crate) type ParsedParams = (
    Option<Parameters>,
    IndexMap<Intern<String>, ParamConvention>,
    IndexMap<Intern<String>, GroupPath>,
    IndexMap<Intern<String>, PredicateExpr>,
);

pub(crate) type ParsedOneParam = (
    Intern<String>,
    Parameter,
    ParamConvention,
    bool,
    Option<GroupPath>,
    Option<PredicateExpr>,
);

impl<'src, 't> TokenCursor<'src, 't> {
    pub(crate) fn parse_result_alternatives(
        &mut self,
        return_tag: &mut Option<Box<Spanned<Expr>>>,
        expr_parser: ExprFn,
        first_name: Option<Intern<String>>,
        payload_postconditions: &mut Vec<ast::PayloadPostcondition>,
    ) -> Vec<Spanned<ResultAlternative>> {
        if !matches!(self.peek_past_indent(), Some(Token::Or | Token::Thus)) {
            return Vec::new();
        }
        let Some(first) = return_tag.take() else {
            return Vec::new();
        };
        let mut members = vec![*first];
        let mut propositions = Vec::new();
        let mut names = vec![first_name];
        loop {
            propositions.push(
                self.eat(&Token::Thus)
                    .then(|| self.parse_proof_proposition()),
            );
            if self.peek_past_indent() != Some(&Token::Or) {
                break;
            }
            self.skip_indents();
            self.advance();
            self.skip_indents();
            let name = if matches!(self.peek(), Some(Token::Id(_))) {
                self.parse_name()
            } else {
                None
            };
            names.push(name);
            let Some(member) = self.parse_type_expr(expr_parser) else {
                // parse-expected-result-alternative
                self.error_here(
                    DiagnosticCode::Parse(ParseCode::Expected(
                        ExpectedSyntaxCode::ResultAlternative,
                    )),
                    "expected a result alternative after `or`",
                );
                break;
            };
            members.push(member);
        }
        if names.iter().all(Option::is_none)
            && members
                .iter()
                .all(|member| matches!(member.value, Expr::AnonymousTag(_)))
        {
            return members
                .into_iter()
                .zip(propositions)
                .map(|(member, proposition)| {
                    let Expr::AnonymousTag(label) = member.value else {
                        unreachable!()
                    };
                    Spanned::new(ResultAlternative { label, proposition }, member.span_id)
                })
                .collect();
        }
        for ((member, proposition), name) in members.iter().zip(propositions).zip(names) {
            if let Some(proposition) = proposition {
                if let Some(name) = name {
                    payload_postconditions.push(ast::PayloadPostcondition {
                        name,
                        ty: Box::new(member.clone()),
                        proposition,
                    });
                } else {
                    // parse-result-proof-requires-name
                    self.error(
                        DiagnosticCode::Parse(ParseCode::Syntax(
                            SyntaxCode::ResultProofRequiresName,
                        )),
                        "a payload proof contract requires a named result",
                        member.span_id,
                    );
                }
            }
        }
        *return_tag = members
            .into_iter()
            .reduce(|lhs, rhs| {
                let span_id = self.merge_span(lhs.span_id, rhs.span_id);
                Spanned::new(
                    Expr::Binary(ast::Binary::new(
                        Typed::infer(lhs.value, lhs.span_id),
                        ast::BinOp::LogicalOr,
                        Typed::infer(rhs.value, rhs.span_id),
                    )),
                    span_id,
                )
            })
            .map(Box::new);
        Vec::new()
    }

    pub fn parse_bind(&mut self, expr_parser: ExprFn) -> Option<Bind> {
        self.skip_newlines();

        let doc_before = self.parse_doc_comment();
        if doc_before.is_some() {
            self.skip_newlines();
        }
        let attrs = self.parse_bind_attributes();

        self.eat(&Token::Indent);

        let binding_ref_mutability = if self.starts_reserved_binding() {
            None
        } else if self.eat(&Token::Ref) {
            Some(false)
        } else if self.eat(&Token::Mut) {
            Some(true)
        } else {
            None
        };

        let name_span = self.peek_span()?;
        let name = self.parse_name()?;

        let (params, conventions, param_groups, param_refinements) = self.parse_params(expr_parser);

        let mut domain_type = None;
        let mut return_refinement = if self.eat(&Token::Is) {
            let mut predicates = if self.eat(&Token::In) {
                if matches!(self.peek(), Some(Token::Tag(_) | Token::SelfTag)) {
                    let ty = self.parse_type_expr(expr_parser)?;
                    domain_type = Some(Box::new(Spanned::new(
                        Expr::IntegerDomain(Box::new(Typed::infer(ty.value, ty.span_id))),
                        ty.span_id,
                    )));
                    Vec::new()
                } else {
                    match self.parse_symbolic_range() {
                        Some((min, max)) => vec![PredicateExpr::Ge(min), PredicateExpr::Le(max)],
                        None => {
                            // parse-expected-integer-range
                            self.error_here(
                                DiagnosticCode::Parse(ParseCode::Expected(
                                    ExpectedSyntaxCode::IntegerRange,
                                )),
                                "expected integer range after 'in'",
                            );
                            Vec::new()
                        }
                    }
                }
            } else {
                vec![self.parse_one_predicate()]
            };
            while self.eat(&Token::And) {
                if domain_type.is_some() {
                    // parse-domain-extra-refinement
                    self.error_here(
                        DiagnosticCode::Parse(ParseCode::Syntax(SyntaxCode::DomainExtraRefinement)),
                        "additional refinements on `is in T` are not supported",
                    );
                }
                predicates.push(self.parse_one_predicate());
            }
            Some(if predicates.len() == 1 {
                predicates.remove(0)
            } else {
                PredicateExpr::And(predicates)
            })
        } else {
            None
        };

        let (mut return_type_name, mut return_tag, type_annotation, type_annotation_qual) =
            self.parse_return_type_part(expr_parser);
        if domain_type.is_some() {
            return_tag = domain_type;
            return_refinement = None;
        }
        if self.is_at(&Token::Or)
            && return_tag.is_none()
            && let Some(name) = return_type_name.take()
        {
            return_tag = Some(Box::new(Spanned::new(
                Expr::AnonymousTag(name),
                self.last_consumed_span(),
            )));
        }
        let mut named_results = Vec::new();
        if let Some(name) = return_type_name {
            let name_span = self.last_consumed_span();
            if matches!(
                self.peek(),
                Some(Token::Tag(_) | Token::At | Token::Ref | Token::Mut | Token::BracketOpen)
            ) && let Some(ty) = self.parse_type_expr(expr_parser)
            {
                named_results.push(NamedResult {
                    name: Some(name),
                    name_span,
                    ty: Box::new(ty),
                });
            }
        }
        if self.is_at(&Token::Comma)
            && named_results.is_empty()
            && let Some(ty) = return_tag.take()
        {
            named_results.push(NamedResult {
                name: None,
                name_span: ty.span_id,
                ty,
            });
        }
        while !named_results.is_empty() && self.eat(&Token::Comma) {
            let name_span = self.current_span();
            let name = if matches!(self.peek(), Some(Token::Id(_)))
                || self.peek().and_then(|token| token.keyword()).is_some()
            {
                self.parse_name()
            } else {
                None
            };
            let Some(ty) = self.parse_type_expr(expr_parser) else {
                // parse-expected-result-type
                self.error_here(
                    DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::ResultType)),
                    "expected a result type after ','",
                );
                break;
            };
            if let Some(_name1) = name
                && named_results.iter().any(|result| result.name == name)
            {
                // parse-duplicate-result-name
                self.error(
                    DiagnosticCode::Parse(ParseCode::Syntax(SyntaxCode::DuplicateResultName)),
                    format!("duplicate named result `{}`", name.unwrap()),
                    name_span,
                );
            }
            named_results.push(NamedResult {
                name,
                name_span,
                ty: Box::new(ty),
            });
        }
        if self.eat(&Token::And) {
            let mut predicates = match return_refinement.take() {
                Some(PredicateExpr::And(predicates)) => predicates,
                Some(predicate) => vec![predicate],
                None => Vec::new(),
            };
            predicates.push(self.parse_one_predicate());
            while self.eat(&Token::And) {
                predicates.push(self.parse_one_predicate());
            }
            return_refinement = Some(if predicates.len() == 1 {
                predicates.remove(0)
            } else {
                PredicateExpr::And(predicates)
            });
        }
        let mut payload_postconditions = Vec::new();
        let first_name =
            if named_results.len() == 1 && matches!(self.peek(), Some(Token::Thus | Token::Or)) {
                let result = named_results.pop().unwrap();
                return_type_name = None;
                return_tag = Some(result.ty);
                result.name
            } else {
                None
            };
        let anonymous_result_alternatives = self.parse_result_alternatives(
            &mut return_tag,
            expr_parser,
            first_name,
            &mut payload_postconditions,
        );
        if return_type_name.is_none()
            && return_tag.is_none()
            && type_annotation.is_none()
            && self.is_at(&Token::Colon)
            && matches!(
                self.peek_at(1),
                Some(Token::Tag(_) | Token::Ref | Token::Mut | Token::BracketOpen)
            )
        {
            let checkpoint = self.checkpoint();
            let errors = self.errors.len();
            self.advance();
            if let Some(type_expr) = self.parse_type_expr(expr_parser) {
                if self.is_at(&Token::Colon) && self.errors.len() == errors {
                    return_tag = Some(Box::new(type_expr));
                } else {
                    self.rewind(checkpoint);
                    self.errors.truncate(errors);
                }
            } else {
                self.rewind(checkpoint);
                self.errors.truncate(errors);
            }
        }
        let mut return_tag = return_tag;
        if let Some(mutable) = binding_ref_mutability
            && let Some(inner) = return_tag.take()
        {
            let span_id = inner.span_id;
            return_tag = Some(Box::new(Spanned {
                value: Expr::Ref {
                    inner: Box::new(Typed::infer(inner.value, inner.span_id)),
                    mutable,
                    group: None,
                },
                span_id,
            }));
        }
        let group_params = params
            .iter()
            .flat_map(|params| params.values())
            .filter_map(|param| match &param.kind {
                ParameterKind::Tagged(sp) => match &sp.value {
                    Expr::Ref {
                        inner,
                        mutable,
                        group: Some(name),
                    } => Some(GroupParam {
                        path: name.clone(),
                        ty_name: match &inner.value {
                            Expr::AnonymousTag(name) => *name,
                            Expr::FnCall(call) => call.path.value.root,
                            Expr::TagCall(call) => call.name,
                            _ => Intern::from_ref("<expr>"),
                        },
                        mutable: *mutable,
                    }),
                    _ => None,
                },
                _ => None,
            })
            .fold(Vec::new(), |mut groups, group| {
                if !groups
                    .iter()
                    .any(|existing: &GroupParam| existing.path == group.path)
                {
                    groups.push(group);
                }
                groups
            });

        let (value, postfix_doc) = if self.eat(&Token::Extern) {
            // parse-removed-extern
            self.error(
                DiagnosticCode::Parse(ParseCode::Removed(RemovedSyntaxCode::Extern)),
                "`extern` has been removed; provide a Gin body using #emit for native instructions",
                name_span,
            );
            (BindValue::Unassigned, None)
        } else if return_tag.is_some() && !self.is_at(&Token::Colon) && !self.is_at(&Token::ColonEq)
        {
            (BindValue::Unassigned, None)
        } else {
            let operator = if self.eat(&Token::ColonEq) {
                BindOperator::FreshImmutable
            } else if self.eat(&Token::ColonColon) {
                BindOperator::Rebind
            } else if let Some(operator) = self.eat_compound_rebind() {
                BindOperator::Compound(operator)
            } else if self.eat(&Token::Colon) {
                BindOperator::FreshMutable
            } else {
                // parse-expected-bind-operator
                self.error_here(
                    DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::BindOperator)),
                    "expected ':' or ':='",
                );
                return None;
            };
            let (value, postfix_doc) = self.parse_bind_value(expr_parser);
            let doc = postfix_doc.or(doc_before);
            let mut bind = Bind::new(name, name_span, value)
                .with_params(params)
                .with_return_type_name(return_type_name)
                .with_named_results(named_results.clone())
                .with_doc(doc);
            bind.operator = operator;
            if let Some(attrs) = attrs {
                bind = bind.with_attributes(attrs);
            }
            bind.param_conventions = conventions;
            bind.group_params = group_params;
            bind.param_groups = param_groups;
            bind.param_refinements = param_refinements;
            bind.return_tag = return_tag;
            bind.return_refinement = return_refinement;
            bind.anonymous_result_alternatives = anonymous_result_alternatives;
            bind.payload_postconditions = payload_postconditions;
            bind.type_annotation = type_annotation;
            bind.type_annotation_qual = type_annotation_qual;
            return Some(bind);
        };

        let doc = postfix_doc.or(doc_before);

        let mut bind = Bind::new(name, name_span, value)
            .with_params(params)
            .with_return_type_name(return_type_name)
            .with_named_results(named_results)
            .with_doc(doc);

        if let Some(attrs) = attrs {
            bind = bind.with_attributes(attrs);
        }
        bind.param_conventions = conventions;
        bind.group_params = group_params;
        bind.param_groups = param_groups;
        bind.param_refinements = param_refinements;
        bind.return_tag = return_tag;
        bind.return_refinement = return_refinement;
        bind.anonymous_result_alternatives = anonymous_result_alternatives;
        bind.payload_postconditions = payload_postconditions;
        bind.type_annotation = type_annotation;
        bind.type_annotation_qual = type_annotation_qual;

        Some(bind)
    }

    pub(crate) fn parse_proof_proposition(&mut self) -> ProofProposition {
        let mut proposition = self.parse_proof_and();
        while self.is_at(&Token::Or)
            && !(matches!(self.peek_at(1), Some(Token::Tag(_)))
                && matches!(
                    self.peek_at(2),
                    None | Some(
                        Token::Thus
                            | Token::Or
                            | Token::Colon
                            | Token::Newline
                            | Token::Indent
                            | Token::ParenOpen
                    )
                ))
            && !(matches!(self.peek_at(1), Some(Token::Id(_)))
                && matches!(self.peek_at(2), Some(Token::Tag(_))))
        {
            self.advance();
            proposition =
                ProofProposition::Or(Box::new(proposition), Box::new(self.parse_proof_and()));
        }
        proposition
    }

    fn parse_proof_and(&mut self) -> ProofProposition {
        let mut proposition = self.parse_proof_not();
        while self.eat(&Token::And) {
            proposition =
                ProofProposition::And(Box::new(proposition), Box::new(self.parse_proof_not()));
        }
        proposition
    }

    fn parse_proof_not(&mut self) -> ProofProposition {
        if self.eat(&Token::Not) {
            return ProofProposition::Not(Box::new(self.parse_proof_not()));
        }
        if self.eat(&Token::ParenOpen) {
            let proposition = self.parse_proof_proposition();
            if !self.eat(&Token::ParenClose) {
                // parse-expected-proof-close
                self.error(
                    DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::ProofClose)),
                    "expected `)` after proof proposition",
                    self.current_span(),
                );
            }
            return proposition;
        }
        self.parse_proof_relation()
    }

    fn parse_proof_relation(&mut self) -> ProofProposition {
        let left = self.parse_proof_term();
        if self.eat(&Token::In) {
            let start = self.parse_proof_term();
            if !self.eat(&Token::To) {
                // parse-expected-proof-range
                self.error_here(
                    DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::ProofRange)),
                    "expected `to` in proof range",
                );
            }
            let end = self.parse_proof_term();
            return ProofProposition::InRange {
                value: left,
                start,
                end,
            };
        }
        let relation = match self.peek() {
            Some(Token::Eq) | Some(Token::EqEq) => ProofRelation::Equal,
            Some(Token::NotEq) => ProofRelation::NotEqual,
            Some(Token::Less) => ProofRelation::Less,
            Some(Token::LessEq) => ProofRelation::LessOrEqual,
            Some(Token::Greater) => ProofRelation::Greater,
            Some(Token::GreaterEq) => ProofRelation::GreaterOrEqual,
            _ => {
                // parse-expected-proof-relation
                self.error_here(
                    DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::ProofRelation)),
                    "expected a mathematical relation in proof proposition",
                );
                return ProofProposition::Compare {
                    left,
                    relation: ProofRelation::Equal,
                    right: ProofTerm::Value(i256::I256::from(0)),
                };
            }
        };
        self.advance();
        ProofProposition::Compare {
            left,
            relation,
            right: self.parse_proof_term(),
        }
    }

    fn parse_proof_term(&mut self) -> ProofTerm {
        let mut term = self.parse_proof_product();
        loop {
            if self.eat(&Token::Plus) {
                term = ProofTerm::Add(Box::new(term), Box::new(self.parse_proof_product()));
            } else if self.eat(&Token::Minus) {
                term = ProofTerm::Sub(Box::new(term), Box::new(self.parse_proof_product()));
            } else {
                return term;
            }
        }
    }

    fn parse_proof_product(&mut self) -> ProofTerm {
        let mut term = self.parse_proof_primary();
        loop {
            if self.eat(&Token::Star) {
                term = ProofTerm::Mul(Box::new(term), Box::new(self.parse_proof_primary()));
            } else if self.eat(&Token::Percent) {
                term = ProofTerm::Remainder(Box::new(term), Box::new(self.parse_proof_primary()));
            } else {
                return term;
            }
        }
    }

    fn parse_proof_primary(&mut self) -> ProofTerm {
        if self.eat(&Token::Minus) {
            return ProofTerm::Sub(
                Box::new(ProofTerm::Value(i256::I256::from(0))),
                Box::new(self.parse_proof_primary()),
            );
        }
        let Some((token, _)) = self.advance() else {
            // parse-expected-proof-term
            self.error_here(
                DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::ProofTerm)),
                "expected an integer term in proof proposition",
            );
            return ProofTerm::Value(i256::I256::from(0));
        };
        match token {
            Token::Int(value) => ProofTerm::Value(value),
            Token::Id(name) | Token::Tag(name) => {
                let name = self.intern(name);
                if name.as_str() == "PowerOfTwo" && self.eat(&Token::ParenOpen) {
                    let argument = self.parse_proof_term();
                    if !self.eat(&Token::ParenClose) {
                        // parse-expected-proof-close
                        self.error(
                            DiagnosticCode::Parse(ParseCode::Expected(
                                ExpectedSyntaxCode::ProofClose,
                            )),
                            "expected `)` after `PowerOfTwo` term",
                            self.current_span(),
                        );
                    }
                    ProofTerm::PowerOfTwo(Box::new(argument))
                } else {
                    ProofTerm::Name(self.parse_proof_member_path(name))
                }
            }
            Token::SelfInstance => {
                ProofTerm::Name(self.parse_proof_member_path(Intern::from_ref("self")))
            }
            Token::Pound => {
                let kind = self.parse_target_query_kind();
                self.expect(&Token::ParenOpen);
                let operand = self.parse_type_reference().unwrap_or_else(|| {
                    ast::TypeReference::Nominal(Intern::from_ref("__parse_error"))
                });
                self.expect(&Token::ParenClose);
                ProofTerm::TargetQuery { kind, operand }
            }
            Token::ParenOpen => {
                let term = self.parse_proof_term();
                if !self.eat(&Token::ParenClose) {
                    // parse-expected-proof-close
                    self.error(
                        DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::ProofClose)),
                        "expected `)` after proof term",
                        self.current_span(),
                    );
                }
                term
            }
            _ => {
                // parse-expected-proof-term
                self.error_here(
                    DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::ProofTerm)),
                    "expected an integer term in proof proposition",
                );
                ProofTerm::Value(i256::I256::from(0))
            }
        }
    }

    pub fn parse_bind_attributes(&mut self) -> Option<BindAttributes> {
        let items = self.parse_attribute_items()?;
        let mut attrs = BindAttributes {
            raw_attributes: Some(items),
            ..Default::default()
        };
        attrs.extract_intrinsic_attributes();
        Some(attrs)
    }

    pub(crate) fn parse_attribute_items(&mut self) -> Option<Vec<AttributeItem>> {
        if !matches!(self.raw_peek(), Some(Token::Pound)) {
            return None;
        }
        if matches!(self.peek_at(1), Some(Token::BracketOpen)) {
            // parse-removed-grouped-attributes
            self.error(
                DiagnosticCode::Parse(ParseCode::Removed(RemovedSyntaxCode::GroupedAttributes)),
                "grouped attributes are no longer valid; write each attribute as `#name`",
                self.span_at(self.pos() + 1),
            );
            self.advance_raw();
            self.advance_raw();
            let mut depth = 1;
            while depth > 0 {
                match self.advance_raw() {
                    Some((Token::BracketOpen, _)) => depth += 1,
                    Some((Token::BracketClose, _)) => depth -= 1,
                    Some(_) => {}
                    None => break,
                }
            }
            if matches!(self.raw_peek(), Some(Token::Newline)) {
                self.advance_raw();
            }
            return Some(Vec::new());
        }

        let mut items = Vec::new();
        loop {
            self.advance_raw();
            if let Some(item) = self.parse_one_attribute_item() {
                items.push(item);
            } else {
                // parse-expected-attribute
                self.error_here(
                    DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::Attribute)),
                    "expected attribute name after `#`",
                );
            }

            if matches!(self.raw_peek(), Some(Token::Comma)) {
                self.advance_raw();
                continue;
            }

            match self.raw_peek() {
                Some(Token::Newline) => {
                    self.advance_raw();
                    if matches!(self.raw_peek(), Some(Token::Pound)) {
                        continue;
                    }
                    if self.attribute_is_detached_from_next_token()
                        || matches!(
                            self.raw_peek(),
                            Some(Token::Comment(_) | Token::DocComment(_))
                        )
                    {
                        // parse-detached-attribute
                        self.error_here(
                            DiagnosticCode::Parse(ParseCode::Syntax(SyntaxCode::DetachedAttribute)),
                            "attribute must be on the line directly above the item it attaches to",
                        );
                    }
                    break;
                }
                Some(Token::Pound) => continue,
                _ => {
                    // parse-attribute-line
                    self.error_here(
                        DiagnosticCode::Parse(ParseCode::Syntax(SyntaxCode::AttributeLine)),
                        "attribute must appear on the line directly above the item it attaches to",
                    );
                    break;
                }
            }
        }

        Some(items)
    }

    pub(crate) fn parse_one_attribute_item(&mut self) -> Option<AttributeItem> {
        match self.peek()? {
            Token::Id(_) | Token::Tag(_) | Token::Private => {
                let name_interned = match self.peek()? {
                    Token::Id(name) | Token::Tag(name) => self.intern(name),
                    Token::Private => self.intern("private"),
                    _ => unreachable!(),
                };
                let name_span = self.peek_span()?;
                self.advance();

                if name_interned.as_str() == "auto_destroy" {
                    // parse-deferred-auto-destroy
                    self.error(
                        DiagnosticCode::Parse(ParseCode::Syntax(SyntaxCode::DeferredAutoDestroy)),
                        "`#auto_destroy` is deferred; call terminal methods explicitly",
                        name_span,
                    );
                } else if name_interned.as_str() == "intrinsic" {
                    // parse-removed-intrinsic
                    self.error(
                        DiagnosticCode::Parse(ParseCode::Removed(RemovedSyntaxCode::Intrinsic)),
                        "`#intrinsic` has been removed; use Gin operations or assembly",
                        name_span,
                    );
                } else if !COMPILER_QUERY_NAMES.contains(&name_interned.as_str()) {
                    // parse-unknown-compiler-query
                    self.error(
                        DiagnosticCode::Parse(ParseCode::Syntax(SyntaxCode::UnknownCompilerQuery)),
                        format!("unknown compiler query `{}`", name_interned.as_str()),
                        name_span,
                    );
                }

                // Id(...) → function call attribute
                if self.is_at(&Token::ParenOpen) {
                    let args = self.parse_paren_args().unwrap_or_default();
                    if name_interned.as_str() == "default" {
                        // parse-removed-default-attribute
                        self.error(
                            DiagnosticCode::Parse(ParseCode::Removed(RemovedSyntaxCode::DefaultAttribute)),
                            "`#default` has been removed; use an explicit type or contextual construction",
                            name_span,
                        );
                    }
                    Some(AttributeItem::Call {
                        name: name_interned,
                        name_span,
                        args,
                    })
                } else {
                    if name_interned.as_str() == "auto" {
                        // parse-removed-auto-attribute
                        self.error(
                            DiagnosticCode::Parse(ParseCode::Removed(
                                RemovedSyntaxCode::AutoAttribute,
                            )),
                            "`#auto` has been removed; ownership is linear by default",
                            name_span,
                        );
                    }
                    Some(AttributeItem::Flag {
                        name: name_interned,
                        span: name_span,
                    })
                }
            }
            _ => None,
        }
    }

    fn parse_return_type_part(&mut self, expr_parser: ExprFn) -> ReturnTypePart {
        if matches!(self.raw_peek(), Some(Token::Newline | Token::Dedent)) {
            return (None, None, None, None);
        }
        // lowercase id → named union return type (e.g., `print(a) result:`)
        if let Some(name) = self.parse_id() {
            return (Some(name), None, None, None);
        }

        if matches!(
            self.peek(),
            Some(Token::At)
                | Some(Token::Ref)
                | Some(Token::Mut)
                | Some(Token::BracketOpen)
                | Some(Token::SelfTag)
        ) {
            return match self.parse_type_expr(expr_parser) {
                Some(sp) => (None, Some(Box::new(sp)), None, None),
                None => (None, None, None, None),
            };
        }

        if self.peek().and_then(|token| token.keyword()).is_some()
            && matches!(self.peek_at(1), Some(Token::Tag(_) | Token::Colon))
        {
            return (self.parse_name(), None, None, None);
        }

        // Tag-based type annotations
        if !matches!(self.peek(), Some(Token::Tag(_))) {
            return (None, None, None, None);
        }

        // Qualified path: Tag.Tag[.Tag...][(args)] (e.g., `Maybe.Some(3)`, `Bool.True`)
        // Guard: a qualified path requires Tag followed by Dot. Skip the speculative
        // checkpoint + allocation when this pattern is absent (e.g., simple `Str`, `Int`).
        if matches!(self.peek_at(1), Some(Token::Dot)) {
            let checkpoint = self.checkpoint();
            if let Some(path) = self.parse_tag_rooted_path(true) {
                if self.is_at(&Token::ParenOpen) {
                    let args = self.parse_type_annotation_args(expr_parser);
                    if !args.is_empty() {
                        let variant_name = *path.segments.last().unwrap_or(&path.root);
                        return (None, None, Some((variant_name, args)), Some(path));
                    }
                }
                let span = path.span_id;
                return (
                    None,
                    Some(Box::new(Spanned {
                        value: path_call(path),
                        span_id: span,
                    })),
                    None,
                    None,
                );
            }
            self.rewind(checkpoint);
        }

        // Simple Tag or Tag(args) (e.g., `Str`, `Maybe(3)`)
        let (name, name_span) = match self.peek() {
            Some(Token::Tag(n)) => {
                let name = self.intern(n);
                let span = self
                    .peek_span()
                    .expect("peek confirmed Tag token, peek_span should succeed");
                self.advance();
                (name, span)
            }
            _ => return (None, None, None, None),
        };

        // `Tag(args)` in return position — try type params first, fall through
        // to value annotation if args are value expressions.
        if self.is_at(&Token::ParenOpen) {
            let args = self.parse_type_annotation_args(expr_parser);
            if !args.is_empty() {
                // If the args are all type-like (bare identifiers or tags), promote
                // to a generic return tag so the typechecker sees a uniform
                // type-surface shape (matching how receivers are stored). Examples:
                //   `Range[x]` → return_tag = generic `Range` with `[x]`
                //   `Maybe(3)` → falls through to `type_annotation` (value annotation)
                let end_span = self.last_consumed_span();
                if let Some(type_args) = Self::try_args_as_type_params(&args) {
                    return (
                        None,
                        Some(Box::new(Spanned {
                            value: Expr::TagCall(ast::TagCall {
                                name,
                                qual_path: None,
                                args: type_args,
                            }),
                            span_id: end_span,
                        })),
                        None,
                        None,
                    );
                }
                return (None, None, Some((name, args)), None);
            }
        }

        (
            None,
            Some(Box::new(Spanned {
                value: Expr::AnonymousTag(name),
                span_id: name_span,
            })),
            None,
            None,
        )
    }

    /// If every arg is a type-like expression, retain it as a generic type application.
    /// Otherwise return `None` so the caller can fall back to the value-annotation
    /// path (e.g., `Maybe(3)`).
    fn try_args_as_type_params(args: &[Typed<Expr>]) -> Option<Vec<Typed<Expr>>> {
        let mut out = Vec::with_capacity(args.len());
        for Typed {
            value: arg,
            span_id: span,
            ..
        } in args
        {
            match arg {
                // Bare lowercase identifier, e.g. `x` in `Range[x]` → type-variable
                // *introduction* (matches how `Range[x]` parses in receiver and
                // declaration positions). Stored as `Generic` so the typechecker
                // doesn't report `x` as an undeclared tag.
                Expr::FnCall(call) if call.path.segments.is_empty() && call.args.is_none() => {
                    out.push(Typed::infer(Expr::FnCall(call.clone()), *span));
                }
                // Bare capitalized tag, e.g. `Int` in `Range(Int)` → concrete
                // instantiation. Tagged so the typechecker resolves and validates it.
                Expr::AnonymousTag(n) => {
                    out.push(Typed::infer(Expr::AnonymousTag(*n), *span));
                }
                _ => return None,
            }
        }
        Some(out)
    }

    fn parse_type_annotation_args(&mut self, expr_parser: ExprFn) -> Vec<Typed<Expr>> {
        if !self.eat(&Token::ParenOpen) {
            return Vec::new();
        }

        // Skip layout after opening paren — allows multiline args like
        // `Maybe(\n    Int\n    Str\n)` where Indent tokens appear inside.
        self.skip_layout();

        let mut args = Vec::new();

        if !self.is_at(&Token::ParenClose) {
            loop {
                if self.is_at(&Token::ParenClose) {
                    break;
                }

                args.push(self.parse_type_argument(expr_parser));

                if self.is_at(&Token::ParenClose) {
                    break;
                }

                if self.eat_list_separator() {
                    continue;
                }

                // parse-expected-annotation-separator
                self.error_here(
                    DiagnosticCode::Parse(ParseCode::Expected(
                        ExpectedSyntaxCode::AnnotationSeparator,
                    )),
                    "expected ',' or newline between type annotation arguments",
                );
                break;
            }
        }

        // Skip trailing layout (e.g. Dedent before `)`) before consuming close.
        self.skip_layout();
        self.expect(&Token::ParenClose);
        args
    }

    pub(crate) fn parse_bind_value(
        &mut self,
        expr_parser: ExprFn,
    ) -> (BindValue, Option<DocComment>) {
        if self.is_at(&Token::Extern) {
            let span = self.current_span();
            self.advance();
            // parse-removed-extern
            self.error(
                DiagnosticCode::Parse(ParseCode::Removed(RemovedSyntaxCode::Extern)),
                "`extern` has been removed; provide a Gin body using #emit for native instructions",
                span,
            );
            return (BindValue::Unassigned, None);
        }

        if self.is_at(&Token::Return) {
            if matches!(self.raw_peek(), Some(Token::Newline)) {
                // parse-expected-indent
                self.error_here(
                    DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::Indentation)),
                    "indent the return inside the bind body",
                );
            }
            let ret = self.parse_return().expect("return token checked");
            return (
                BindValue::Body {
                    exprs: Vec::new(),
                    ret,
                },
                None,
            );
        }

        if self.is_at(&Token::Indent) {
            self.advance();
            let exprs = self.parse_body_exprs(expr_parser);
            self.expect(&Token::Dedent);
            let mut exprs = exprs;
            if let Some(Typed {
                value: Expr::Return(_),
                ..
            }) = exprs.last()
            {
                let Expr::Return(ret) = exprs.pop().unwrap().value else {
                    unreachable!()
                };
                return (BindValue::Body { exprs, ret }, None);
            }
            let span = match (exprs.first(), exprs.last()) {
                (Some(first), Some(last)) => self.merge_span(first.span_id, last.span_id),
                _ => self.current_span(),
            };
            return (
                BindValue::Expr(Box::new(Typed::infer(Expr::Block(exprs), span))),
                None,
            );
        }

        // Otherwise → single expression with optional postfix doc comment
        let expr = self.parse_sequence_value();
        let doc = self.parse_same_line_doc();

        (BindValue::Expr(Box::new(expr)), doc)
    }

    pub(crate) fn parse_params(&mut self, expr_parser: ExprFn) -> ParsedParams {
        if !self.is_at(&Token::ParenOpen) {
            return (None, IndexMap::new(), IndexMap::new(), IndexMap::new());
        }

        let mut params = Parameters::new();
        let mut conventions = IndexMap::new();
        let mut param_groups = IndexMap::new();
        let mut param_refinements = IndexMap::new();
        let mut seen_default = false;
        self.advance();

        // Skip layout after opening paren: allows `(\n  a Int, b Int)` style.
        self.skip_layout();

        if self.is_at(&Token::ParenClose) {
            self.advance();
            return (Some(params), conventions, param_groups, param_refinements);
        }

        let mut ingest = |key: Intern<String>,
                          kind: Parameter,
                          conv: ParamConvention,
                          explicit: bool,
                          group: Option<GroupPath>,
                          refinement: Option<PredicateExpr>| {
            let previous = params.insert(key, kind);
            if explicit {
                conventions.insert(key, conv);
            }
            if let Some(gn) = group {
                param_groups.insert(key, gn);
            }
            if let Some(refinement) = refinement {
                param_refinements.insert(key, refinement);
            }
            previous
        };

        loop {
            if let Some((key, kind, conv, explicit, group, refinement)) =
                self.parse_one_param(expr_parser)
            {
                if seen_default && !matches!(kind.kind, ParameterKind::Default(_)) {
                    // parse-parameter-after-default
                    self.error(
                        DiagnosticCode::Parse(ParseCode::Syntax(SyntaxCode::ParameterAfterDefault)),
                        format!(
                            "positional parameter `{}` appears after a default parameter",
                            key.as_str()
                        ),
                        self.current_span(),
                    );
                }
                if matches!(kind.kind, ParameterKind::Default(_)) {
                    seen_default = true;
                }
                let name_span = kind.name_span;
                if ingest(key, kind, conv, explicit, group, refinement).is_some()
                    && key.as_str() != "_"
                {
                    // parse-duplicate-parameter
                    self.error(
                        DiagnosticCode::Parse(ParseCode::Syntax(SyntaxCode::DuplicateParameter)),
                        format!("parameter `{key}` is already declared in this parameter list"),
                        name_span,
                    );
                }
            }

            if self.is_at(&Token::ParenClose) {
                break;
            }

            if self.eat_list_separator() {
                continue;
            }

            // parse-expected-param-separator
            self.error_here(
                DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::ParameterSeparator)),
                "expected ',' or newline between parameters",
            );
            break;
        }

        // Skip trailing layout (Dedent before `)`) before consuming the close.
        self.skip_layout();
        self.expect(&Token::ParenClose);
        (Some(params), conventions, param_groups, param_refinements)
    }

    pub(crate) fn parse_one_param(&mut self, expr_parser: ExprFn) -> Option<ParsedOneParam> {
        if matches!(self.peek(), Some(Token::Ref | Token::Mut))
            && matches!(self.peek_at(1), Some(Token::Id(_)))
        {
            // parse-removed-parameter-convention-syntax
            self.error_here(DiagnosticCode::Parse(ParseCode::Removed(RemovedSyntaxCode::ParameterConventionSyntax)), "removed syntax in parameter position: use `x` for terminal or `x ref`/`x mut`/`x own`");
            self.advance();
        }

        if self.is_at(&Token::SelfTag) && matches!(self.peek_at(1), Some(Token::Tag(_))) {
            // parse-reserved-name
            self.error_here(
                DiagnosticCode::Parse(ParseCode::Syntax(SyntaxCode::ReservedName)),
                "`Self` cannot name a parameter",
            );
        }

        // Positional: bare Tag → (tag_name, ParameterKind::Tagged(tag))
        if matches!(self.peek(), Some(Token::Tag(_) | Token::SelfTag)) {
            let sp = self.parse_type_expr(expr_parser)?;
            let key = Intern::<String>::from_ref(
                Pattern::from_expr(sp.value.clone()).surface_mangle_name(),
            );
            return Some((
                key,
                Parameter::new(
                    sp.span_id,
                    ParameterKind::Tagged(Box::new(Spanned {
                        value: sp.value,
                        span_id: sp.span_id,
                    })),
                ),
                ParamConvention::Own,
                false,
                None,
                None,
            ));
        }

        let name = match self.peek() {
            Some(Token::Id(n)) => {
                let name_span = self.peek_span()?;
                if *n == "eat" {
                    // parse-removed-parameter-convention-syntax
                    self.error_here(DiagnosticCode::Parse(ParseCode::Removed(RemovedSyntaxCode::ParameterConventionSyntax)), "removed syntax in parameter position: use `x` for terminal or `x ref`/`x mut`/`x own`");
                    return None;
                }
                let id = self.intern(n);
                self.advance();
                (id, name_span)
            }
            Some(Token::SelfInstance) => {
                let name_span = self.peek_span()?;
                self.advance();
                (Intern::from_ref("self"), name_span)
            }
            _ => {
                let span = self.peek_span()?;
                (self.parse_name()?, span)
            }
        };

        let (mut convention, explicit) = if self.eat(&Token::Own) {
            (ParamConvention::Own, true)
        } else if self.eat(&Token::Ref) {
            (ParamConvention::Observe, true)
        } else if self.eat(&Token::Mut) {
            (ParamConvention::Mutate, true)
        } else {
            (ParamConvention::Terminal, false)
        };

        let group_name = if matches!(
            convention,
            ParamConvention::Observe | ParamConvention::Mutate
        ) && self.eat(&Token::CurlyOpen)
        {
            let group = self.parse_group_path()?;
            self.expect(&Token::CurlyClose)?;
            Some(group)
        } else {
            None
        };

        if self.eat(&Token::Is) {
            if !self.eat(&Token::In) {
                let proposition = self.parse_proof_proposition();
                return Some((
                    name.0,
                    Parameter::new(name.1, ParameterKind::Proposition(Box::new(proposition))),
                    convention,
                    explicit,
                    group_name,
                    None,
                ));
            }
            if matches!(self.peek(), Some(Token::Tag(_) | Token::SelfTag)) {
                let ty = self.parse_type_expr(expr_parser)?;
                let refinement = self.parse_parameter_refinement();
                return Some((
                    name.0,
                    Parameter::new(
                        name.1,
                        ParameterKind::Tagged(Box::new(Spanned::new(
                            Expr::IntegerDomain(Box::new(Typed::infer(ty.value, ty.span_id))),
                            ty.span_id,
                        ))),
                    ),
                    convention,
                    explicit,
                    group_name,
                    refinement,
                ));
            }
            let refinement = self.parse_symbolic_range().map(|(min, max)| {
                let mut predicates = vec![PredicateExpr::Ge(min), PredicateExpr::Le(max)];
                while self.eat(&Token::And) {
                    predicates.push(self.parse_one_predicate());
                }
                PredicateExpr::And(predicates)
            });
            return Some((
                name.0,
                Parameter::new(name.1, ParameterKind::Generic),
                convention,
                explicit,
                group_name,
                refinement,
            ));
        }

        let (name, mut parameter) =
            self.parse_param_after_name(expr_parser, name.0, name.1, false)?;
        if !explicit
            && let Some(legacy_convention) = match self.peek() {
                Some(Token::Own) => Some((ParamConvention::Own, "own")),
                Some(Token::Ref) => Some((ParamConvention::Observe, "ref")),
                Some(Token::Mut) => Some((ParamConvention::Mutate, "mut")),
                _ => None,
            }
        {
            let (legacy_convention, spelling) = legacy_convention;
            // parse-trailing-parameter-convention
            self.error(
                DiagnosticCode::Parse(ParseCode::Syntax(SyntaxCode::TrailingParameterConvention)),
                format!(
                    "parameter convention `{spelling}` must follow the parameter name; use `{name} {spelling} <type>`"
                ),
                self.current_span(),
            );
            self.advance();
            convention = legacy_convention;
        }
        let kind = match (convention, parameter.kind) {
            (ParamConvention::Observe | ParamConvention::Mutate, ParameterKind::Tagged(inner)) => {
                let span_id = inner.span_id;
                ParameterKind::Tagged(Box::new(Spanned {
                    value: Expr::Ref {
                        inner: Box::new(Typed::infer(inner.value, inner.span_id)),
                        mutable: convention == ParamConvention::Mutate,
                        group: group_name.clone(),
                    },
                    span_id,
                }))
            }
            (_, kind) => kind,
        };
        parameter.kind = kind;
        let refinement = self.parse_parameter_refinement();
        Some((
            name, parameter, convention, explicit, group_name, refinement,
        ))
    }

    fn parse_parameter_refinement(&mut self) -> Option<PredicateExpr> {
        if self.eat(&Token::And) {
            let mut predicates = vec![self.parse_one_predicate()];
            while self.eat(&Token::And) {
                predicates.push(self.parse_one_predicate());
            }
            Some(if predicates.len() == 1 {
                predicates.pop().expect("one predicate")
            } else {
                PredicateExpr::And(predicates)
            })
        } else {
            None
        }
    }

    pub(crate) fn parse_same_line_doc(&mut self) -> Option<DocComment> {
        if matches!(self.raw_peek(), Some(Token::DocComment(_))) {
            self.parse_doc_comment()
        } else {
            None
        }
    }

    pub(crate) fn parse_doc_comment(&mut self) -> Option<DocComment> {
        let start = self.peek_span()?;
        let first = match self.peek()? {
            Token::DocComment(text) => {
                let stripped = text
                    .strip_prefix("---")
                    .map(|s| s.trim_start())
                    .unwrap_or(text)
                    .to_owned();
                self.advance();
                stripped
            }
            _ => return None,
        };

        // Fast path: single-line doc comment (most common)
        if !matches!(self.peek(), Some(Token::DocComment(_))) {
            let doc = DocComment::new(first).with_span(start);
            return if doc.is_empty() { None } else { Some(doc) };
        }

        let mut lines = vec![first];

        while let Some(Token::DocComment(text)) = self.peek() {
            let stripped = text
                .strip_prefix("---")
                .map(|s| s.trim_start())
                .unwrap_or(text)
                .to_owned();
            self.advance();
            lines.push(stripped);
        }

        let span = self.merge_span(start, self.last_consumed_span());
        let doc = DocComment::new(lines.join("\n")).with_span(span);
        if doc.is_empty() { None } else { Some(doc) }
    }
}
