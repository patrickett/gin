use indexmap::IndexMap;
use internment::Intern;
use lexer::Token;

use ast::{
    AttributeItem, Bind, BindAttributes, BindOperator, BindValue, COMPILER_QUERY_NAMES, DocComment,
    Expr, GroupParam, GroupPath, ModPath, NamedResult, ParamConvention, Parameter, ParameterKind,
    Parameters, Pattern, PredicateExpr, ProofProposition, ProofRelation, ProofTerm,
    ResultAlternative, Return, SpanId, Spanned, Typed,
};

use super::ExprFn;
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
    ) -> Vec<Spanned<ResultAlternative>> {
        let mut anonymous_result_alternatives = Vec::new();
        if (self.is_at(&Token::Or) || self.is_at(&Token::Thus))
            && let Some(first) = return_tag.as_deref()
            && let Expr::AnonymousTag(label) = first.value
        {
            let first_span = first.span_id;
            let proposition = self
                .eat(&Token::Thus)
                .then(|| self.parse_proof_proposition());
            anonymous_result_alternatives.push(Spanned {
                value: ResultAlternative { label, proposition },
                span_id: first_span,
            });
            *return_tag = None;
            while self.eat(&Token::Or) {
                let Some(Token::Tag(label)) = self.peek() else {
                    self.error(
                        "parse-expected-result-alternative",
                        "expected an anonymous result alternative after `or`",
                        self.current_span(),
                    );
                    break;
                };
                let span_id = self.peek_span().unwrap_or(SpanId::INVALID);
                let label = self.intern(label);
                self.advance();
                let proposition = self
                    .eat(&Token::Thus)
                    .then(|| self.parse_proof_proposition());
                anonymous_result_alternatives.push(Spanned {
                    value: ResultAlternative { label, proposition },
                    span_id,
                });
            }
        }
        anonymous_result_alternatives
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

        let mut return_refinement = if self.eat(&Token::Is) {
            let mut predicates = if self.eat(&Token::In) {
                match self.parse_symbolic_range() {
                    Some((min, max)) => vec![PredicateExpr::Ge(min), PredicateExpr::Le(max)],
                    None => {
                        self.error(
                            "parse-expected-integer-range",
                            "expected integer range after 'in'",
                            self.current_span(),
                        );
                        Vec::new()
                    }
                }
            } else {
                vec![self.parse_one_predicate()]
            };
            while self.eat(&Token::And) {
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

        let (return_type_name, mut return_tag, type_annotation, type_annotation_qual) =
            self.parse_return_type_part(expr_parser);
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
                self.error(
                    "parse-expected-result-type",
                    "expected a result type after ','",
                    self.current_span(),
                );
                break;
            };
            if name.is_some() && named_results.iter().any(|result| result.name == name) {
                self.error(
                    "parse-duplicate-result-name",
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
        let anonymous_result_alternatives = self.parse_result_alternatives(&mut return_tag);
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

        let assembly_items = attrs.as_ref().and_then(|attrs| {
            attrs.raw_attributes.as_ref().map(|items| {
                items
                    .iter()
                    .filter_map(|item| match item {
                        AttributeItem::Call { name, args, .. } if name.as_str() == "asm" => {
                            Some(args.clone())
                        }
                        AttributeItem::Flag { name, .. } if name.as_str() == "asm" => {
                            Some(Vec::new())
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
        });
        if let Some(items) = assembly_items
            && !items.is_empty()
        {
            self.error(
                "parse-asm-declaration-migration",
                "declaration-level `#asm` is removed; put `#asm(specification, name: value, ...)` in the function expression body",
                name_span,
            );
            if self.is_at(&Token::Colon) || self.is_at(&Token::ColonEq) {
                self.error(
                    "parse-asm-with-body",
                    "`#asm` callables cannot also have a Gin body",
                    name_span,
                );
            }
            if self.is_at(&Token::Extern) {
                self.error(
                    "parse-asm-with-extern",
                    "`#asm` callables must not also use `extern`",
                    name_span,
                );
                self.advance();
            }
            if items.len() > 1 {
                self.error(
                    "parse-duplicate-asm-attribute",
                    "a callable may have only one `#asm` attribute",
                    name_span,
                );
            }
            let Some(_expr) = items[0].first().cloned() else {
                self.error(
                    "parse-asm-missing-argument",
                    "`#asm` requires exactly one expression argument",
                    name_span,
                );
                return None;
            };
            if items[0].len() != 1 {
                self.error(
                    "parse-asm-argument-count",
                    "`#asm` requires exactly one expression argument",
                    name_span,
                );
                return None;
            }
            let operator = BindOperator::FreshMutable;
            let mut bind = Bind::new(name, name_span, BindValue::Unassigned)
                .with_params(params)
                .with_return_type_name(return_type_name)
                .with_named_results(named_results.clone())
                .with_doc(doc_before);
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
            bind.type_annotation = type_annotation;
            bind.type_annotation_qual = type_annotation_qual;
            return Some(bind);
        }

        let (value, postfix_doc) = if self.eat(&Token::Extern) {
            self.error(
                "parse-removed-extern",
                "`extern` has been removed; provide a Gin or assembly body",
                name_span,
            );
            (BindValue::Unassigned, None)
        } else if return_tag.is_some() && !self.is_at(&Token::Colon) && !self.is_at(&Token::ColonEq)
        {
            // `name Type` on its own line — declared, assigned later via `name: value`.
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
                self.error(
                    "parse-expected-bind-operator",
                    "expected ':' or ':='",
                    self.current_span(),
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
        bind.type_annotation = type_annotation;
        bind.type_annotation_qual = type_annotation_qual;

        Some(bind)
    }

    pub(crate) fn parse_proof_proposition(&mut self) -> ProofProposition {
        self.parse_proof_or()
    }

    fn parse_proof_or(&mut self) -> ProofProposition {
        let mut proposition = self.parse_proof_and();
        while self.is_at(&Token::Or)
            && !(matches!(self.peek_at(1), Some(Token::Tag(_)))
                && matches!(
                    self.peek_at(2),
                    Some(Token::Thus) | Some(Token::Or) | Some(Token::Colon)
                ))
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
                self.error(
                    "parse-expected-proof-close",
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
                self.error(
                    "parse-expected-proof-range",
                    "expected `to` in proof range",
                    self.current_span(),
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
                self.error(
                    "parse-expected-proof-relation",
                    "expected a mathematical relation in proof proposition",
                    self.current_span(),
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
            self.error(
                "parse-expected-proof-term",
                "expected an integer term in proof proposition",
                self.current_span(),
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
                        self.error(
                            "parse-expected-proof-close",
                            "expected `)` after `PowerOfTwo` term",
                            self.current_span(),
                        );
                    }
                    ProofTerm::PowerOfTwo(Box::new(argument))
                } else {
                    ProofTerm::Name(name)
                }
            }
            Token::SelfInstance => ProofTerm::Name(Intern::from_ref("self")),
            Token::Pound => {
                let kind = match self.advance() {
                    Some((Token::Id("size"), _)) => ast::TargetQueryKind::Size,
                    Some((Token::Id("alignment"), _)) => ast::TargetQueryKind::Alignment,
                    Some((Token::Id("index_bits"), _)) => ast::TargetQueryKind::IndexBits,
                    _ => {
                        self.error(
                            "parse-expected-query-name",
                            "expected compile-time query name after '#'",
                            self.current_span(),
                        );
                        ast::TargetQueryKind::Size
                    }
                };
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
                    self.error(
                        "parse-expected-proof-close",
                        "expected `)` after proof term",
                        self.current_span(),
                    );
                }
                term
            }
            _ => {
                self.error(
                    "parse-expected-proof-term",
                    "expected an integer term in proof proposition",
                    self.current_span(),
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
            self.error(
                "parse-removed-grouped-attributes",
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
                self.error(
                    "parse-expected-attribute",
                    "expected attribute name after `#`",
                    self.current_span(),
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
                        self.error(
                            "parse-detached-attribute",
                            "attribute must be on the line directly above the item it attaches to",
                            self.current_span(),
                        );
                    }
                    break;
                }
                Some(Token::Pound) => continue,
                _ => {
                    self.error(
                        "parse-attribute-line",
                        "attribute must appear on the line directly above the item it attaches to",
                        self.current_span(),
                    );
                    break;
                }
            }
        }

        Some(items)
    }

    pub(crate) fn parse_one_attribute_item(&mut self) -> Option<AttributeItem> {
        match self.peek()? {
            Token::Id(name) | Token::Tag(name) => {
                let name_interned = self.intern(name);
                let name_span = self.peek_span()?;
                self.advance();

                if name_interned.as_str() == "intrinsic" {
                    self.error(
                        "parse-removed-intrinsic",
                        "`#intrinsic` has been removed; use Gin operations or assembly",
                        name_span,
                    );
                } else if !COMPILER_QUERY_NAMES.contains(&name_interned.as_str()) {
                    self.error(
                        "parse-unknown-compiler-query",
                        format!("unknown compiler query `{}`", name_interned.as_str()),
                        name_span,
                    );
                }

                // Id(...) → function call attribute
                if self.is_at(&Token::ParenOpen) {
                    let args = self.parse_paren_args().unwrap_or_default();
                    if name_interned.as_str() == "default" {
                        self.error(
                            "parse-removed-default-attribute",
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
                        self.error(
                            "parse-removed-auto-attribute",
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
            if let Some(path) = self.parse_tag_variant_path() {
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
                        value: Expr::FnCall(ast::FnCall { path, args: None }),
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

                args.push(expr_parser(self));

                if self.is_at(&Token::ParenClose) {
                    break;
                }

                if self.eat_list_separator() {
                    continue;
                }

                self.error(
                    "parse-expected-annotation-separator",
                    "expected ',' or newline between type annotation arguments",
                    self.current_span(),
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
            self.error(
                "parse-removed-extern",
                "`extern` has been removed; provide a Gin or assembly body",
                span,
            );
            return (BindValue::Unassigned, None);
        }

        if self.is_at(&Token::Newline) && matches!(self.peek_at(1), Some(Token::Return)) {
            self.advance();
        }

        if self.is_at(&Token::Return) {
            let ret = self
                .parse_return(expr_parser)
                .expect("return token checked");
            return (
                BindValue::Body {
                    exprs: Vec::new(),
                    ret,
                },
                None,
            );
        }

        // Indent → multi-line body
        if self.is_at(&Token::Indent) {
            let _has_indent = self.eat(&Token::Indent);

            let exprs = self.parse_body_exprs(expr_parser);

            let dedented = self.eat(&Token::Dedent);
            if !dedented && self.is_at(&Token::Return) {
                self.error(
                    "parse-indented-return",
                    "return must align with the bind name; dedent it to close the bind",
                    self.peek_span().unwrap_or_else(|| self.current_span()),
                );
            }

            let ret = self.parse_return(expr_parser).unwrap_or_else(|| {
                self.error(
                    "parse-expected-return",
                    "expected 'return' after bind body",
                    self.current_span(),
                );
                Return {
                    values: Vec::new(),
                    span_id: self.current_span(),
                }
            });
            if !dedented {
                self.eat(&Token::Dedent);
            }
            return (BindValue::Body { exprs, ret }, None);
        }

        // Otherwise → single expression with optional postfix doc comment
        let expr = self.parse_sequence_value();
        // Only consume a doc comment if it's on the same line (no intervening newline),
        // otherwise it belongs to the next top-level declaration.
        let doc = if matches!(self.peek_at(0), Some(Token::DocComment(_))) {
            self.parse_doc_comment()
        } else {
            None
        };

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
            params.insert(key, kind);
            if explicit {
                conventions.insert(key, conv);
            }
            if let Some(gn) = group {
                param_groups.insert(key, gn);
            }
            if let Some(refinement) = refinement {
                param_refinements.insert(key, refinement);
            }
        };

        loop {
            if let Some((key, kind, conv, explicit, group, refinement)) =
                self.parse_one_param(expr_parser)
            {
                if seen_default && !matches!(kind.kind, ParameterKind::Default(_)) {
                    self.error(
                        "parse-parameter-after-default",
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
                ingest(key, kind, conv, explicit, group, refinement);
            }

            if self.is_at(&Token::ParenClose) {
                break;
            }

            if self.eat_list_separator() {
                continue;
            }

            self.error(
                "parse-expected-param-separator",
                "expected ',' or newline between parameters",
                self.current_span(),
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
            self.error(
                "parse-removed-parameter-convention-syntax",
                "removed syntax in parameter position: use `x` for terminal or `x ref`/`x mut`/`x own`",
                self.current_span(),
            );
            self.advance();
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
                    self.error(
                        "parse-removed-parameter-convention-syntax",
                        "removed syntax in parameter position: use `x` for terminal or `x ref`/`x mut`/`x own`",
                        self.current_span(),
                    );
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
                self.error(
                    "parse-expected-integer-range",
                    "expected 'in' after parameter 'is'",
                    self.current_span(),
                );
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
            self.error(
                "parse-trailing-parameter-convention",
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
        let refinement = if self.eat(&Token::And) {
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
        };
        Some((
            name, parameter, convention, explicit, group_name, refinement,
        ))
    }

    pub(crate) fn parse_doc_comment(&mut self) -> Option<DocComment> {
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
            let doc = DocComment { value: first };
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

        let doc = DocComment {
            value: lines.join("\n"),
        };
        if doc.is_empty() { None } else { Some(doc) }
    }
}
