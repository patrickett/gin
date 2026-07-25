use indexmap::IndexMap;
use internment::Intern;
use lexer::Token;

use ast::{
    AttributeItem, Bind, BindAttributes, BindValue, DocComment, Expr, GroupParam, ModPath,
    ParamConvention, ParameterKind, Parameters, PredicateExpr, Return, Spanned, TypeExpr, Typed,
};

use super::ExprFn;
use crate::cursor::TokenCursor;

type ReturnTypePart = (
    Option<Intern<String>>,
    Option<Box<Spanned<TypeExpr>>>,
    Option<(Intern<String>, Vec<Typed<Expr>>)>,
    Option<Spanned<ModPath>>,
);

pub(crate) type ParsedParams = (
    Option<Parameters>,
    IndexMap<Intern<String>, ParamConvention>,
    IndexMap<Intern<String>, Intern<String>>,
    IndexMap<Intern<String>, PredicateExpr>,
);

pub(crate) type ParsedOneParam = (
    Intern<String>,
    ParameterKind,
    ParamConvention,
    Option<Intern<String>>,
    Option<PredicateExpr>,
);

impl<'src, 't> TokenCursor<'src, 't> {
    pub fn parse_bind(&mut self, expr_parser: ExprFn) -> Option<Bind> {
        self.skip_newlines();

        let doc_before = self.parse_doc_comment();
        if doc_before.is_some() {
            self.skip_newlines();
        }
        let attrs = self.parse_bind_attributes();

        self.eat(&Token::Indent);

        let binding_ref_mutability = if self.eat(&Token::Ref) {
            Some(false)
        } else if self.eat(&Token::Mut) {
            Some(true)
        } else {
            None
        };

        let (name, name_span) = match self.peek() {
            Some(Token::Id(n)) => {
                let name = self.intern(n);
                let span = self.peek_span()?;
                self.advance();
                (name, span)
            }
            _ => return None,
        };

        let (params, conventions, param_groups, param_refinements) = self.parse_params(expr_parser);

        let (return_type_name, mut return_tag, type_annotation, type_annotation_qual) =
            self.parse_return_type_part(expr_parser);
        if let Some(mutable) = binding_ref_mutability
            && let Some(inner) = return_tag.take()
        {
            let span_id = inner.span_id;
            return_tag = Some(Box::new(Spanned {
                value: TypeExpr::Ref {
                    inner,
                    mutable,
                    group: None,
                },
                span_id,
            }));
        }
        let group_params = params
            .iter()
            .flat_map(|params| params.values())
            .filter_map(|kind| match kind {
                ParameterKind::Tagged(sp) => match &sp.value {
                    TypeExpr::Ref {
                        inner,
                        mutable,
                        group: Some(name),
                    } => Some(GroupParam {
                        name: *name,
                        ty_name: Intern::from_ref(inner.value.surface_mangle_name()),
                        mutable: *mutable,
                    }),
                    _ => None,
                },
                _ => None,
            })
            .fold(Vec::new(), |mut groups, group| {
                if !groups
                    .iter()
                    .any(|existing: &GroupParam| existing.name == group.name)
                {
                    groups.push(group);
                }
                groups
            });

        // Handle `extern` binds: `name(params) extern` without `:` or `:=`
        let (value, postfix_doc) = if self.eat(&Token::Extern) {
            (BindValue::Extern, None)
        } else if return_tag.is_some() && !self.is_at(&Token::Colon) && !self.is_at(&Token::ColonEq)
        {
            // `name Type` on its own line — declared, assigned later via `name: value`.
            (BindValue::Unassigned, None)
        } else {
            let is_constant = if self.eat(&Token::ColonEq) {
                true
            } else if self.eat(&Token::Colon) {
                false
            } else {
                self.error(
                    "parse-expected-bind-operator",
                    "expected ':', ':=' or 'extern'",
                    self.current_span(),
                );
                return None;
            };
            let (value, postfix_doc) = self.parse_bind_value(expr_parser);
            let doc = postfix_doc.or(doc_before);
            let mut bind = Bind::new(name, name_span, value)
                .with_params(params)
                .with_return_type_name(return_type_name)
                .with_doc(doc);
            bind.is_constant = is_constant;
            if let Some(attrs) = attrs {
                bind = bind.with_attributes(attrs);
            }
            bind.param_conventions = conventions;
            bind.group_params = group_params;
            bind.param_groups = param_groups;
            bind.param_refinements = param_refinements;
            bind.return_tag = return_tag;
            bind.type_annotation = type_annotation;
            bind.type_annotation_qual = type_annotation_qual;
            return Some(bind);
        };

        let doc = postfix_doc.or(doc_before);

        let mut bind = Bind::new(name, name_span, value)
            .with_params(params)
            .with_return_type_name(return_type_name)
            .with_doc(doc);

        if let Some(attrs) = attrs {
            bind = bind.with_attributes(attrs);
        }
        bind.param_conventions = conventions;
        bind.group_params = group_params;
        bind.param_groups = param_groups;
        bind.param_refinements = param_refinements;
        bind.return_tag = return_tag;
        bind.type_annotation = type_annotation;
        bind.type_annotation_qual = type_annotation_qual;

        Some(bind)
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
                    if self.blank_line_between_last_consumed_and_raw_peek()
                        || matches!(
                            self.raw_peek(),
                            Some(Token::Newline | Token::Comment(_) | Token::DocComment(_))
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
            Token::Id(name) => {
                let name_interned = self.intern(name);
                let name_span = self.peek_span()?;
                self.advance();

                // Id(...) → function call attribute
                if self.is_at(&Token::ParenOpen) {
                    let args = self.parse_paren_args().unwrap_or_default();
                    Some(AttributeItem::Call {
                        name: name_interned,
                        name_span,
                        args,
                    })
                } else {
                    // bare Id → flag attribute
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

        if matches!(self.peek(), Some(Token::Ref) | Some(Token::Mut)) {
            return match self.parse_type_expr(expr_parser) {
                Some(sp) => (None, Some(Box::new(sp)), None, None),
                None => (None, None, None, None),
            };
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
                        value: TypeExpr::Qualified(path),
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
                // to a `TypeGeneric` return tag so the typechecker sees a uniform
                // type-surface shape (matching how receivers are stored). Examples:
                //   `Range[x]` → return_tag = TypeGeneric { Range, [x: Generic] }
                //   `Maybe(3)` → falls through to `type_annotation` (value annotation)
                if let Some(type_params) = Self::try_args_as_type_params(&args) {
                    return (
                        None,
                        Some(Box::new(Spanned {
                            value: TypeExpr::Generic {
                                name,
                                params: type_params,
                                param_spans: Vec::new(),
                                span: name_span,
                            },
                            span_id: name_span,
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
                value: TypeExpr::Nominal(name, name_span),
                span_id: name_span,
            })),
            None,
            None,
        )
    }

    /// If every arg is a "type-like" expression (bare lowercase identifier or bare
    /// tag), convert to declaration parameters suitable for `TypeExpr::Generic`.
    /// Otherwise return `None` so the caller can fall back to the value-annotation
    /// path (e.g., `Maybe(3)`).
    fn try_args_as_type_params(
        args: &[Typed<Expr>],
    ) -> Option<Vec<(Intern<String>, ParameterKind)>> {
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
                    out.push((call.path.root, ParameterKind::Generic));
                }
                // Bare capitalized tag, e.g. `Int` in `Range(Int)` → concrete
                // instantiation. Tagged so the typechecker resolves and validates it.
                Expr::AnonymousTag(n) => {
                    let sp = Spanned {
                        value: TypeExpr::Nominal(*n, *span),
                        span_id: *span,
                    };
                    out.push((*n, ParameterKind::Tagged(Box::new(sp))));
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
        // extern → BindValue::Extern
        if self.eat(&Token::Extern) {
            return (BindValue::Extern, None);
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

            // Dedent is optional — return may appear inside or outside the indented block
            self.eat(&Token::Dedent);

            let ret = self.parse_return(expr_parser).unwrap_or_else(|| {
                self.error(
                    "parse-expected-return",
                    "expected 'return' after bind body",
                    self.current_span(),
                );
                Return {
                    value: None,
                    span_id: self.current_span(),
                }
            });

            return (BindValue::Body { exprs, ret }, None);
        }

        // Otherwise → single expression with optional postfix doc comment
        let expr = expr_parser(self);
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
                          kind: ParameterKind,
                          conv: ParamConvention,
                          group: Option<Intern<String>>,
                          refinement: Option<PredicateExpr>| {
            params.insert(key, kind);
            if conv != ParamConvention::Own {
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
            if let Some((key, kind, conv, group, refinement)) = self.parse_one_param(expr_parser) {
                if seen_default && !matches!(kind, ParameterKind::Default(_)) {
                    self.error(
                        "parse-parameter-after-default",
                        format!(
                            "positional parameter `{}` appears after a default parameter",
                            key.as_str()
                        ),
                        self.current_span(),
                    );
                }
                if matches!(kind, ParameterKind::Default(_)) {
                    seen_default = true;
                }
                ingest(key, kind, conv, group, refinement);
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

    pub(crate) fn parse_param_convention_prefix(&mut self) -> ParamConvention {
        if self.eat(&Token::Tilde) || self.eat(&Token::Eat) {
            ParamConvention::Consume
        } else if self.eat(&Token::Ref) {
            ParamConvention::Observe
        } else if self.eat(&Token::Mut) {
            ParamConvention::Mutate
        } else {
            ParamConvention::Own
        }
    }

    pub(crate) fn parse_one_param(&mut self, expr_parser: ExprFn) -> Option<ParsedOneParam> {
        // Positional: bare Tag → (tag_name, ParameterKind::Tagged(tag))
        if matches!(self.peek(), Some(Token::Tag(_))) {
            let sp = self.parse_type_expr(expr_parser)?;
            let key = Intern::<String>::from_ref(sp.value.surface_mangle_name());
            return Some((
                key,
                ParameterKind::Tagged(Box::new(Spanned {
                    value: sp.value,
                    span_id: sp.span_id,
                })),
                ParamConvention::Own,
                None,
                None,
            ));
        }

        let convention = self.parse_param_convention_prefix();

        let group_name = if matches!(
            convention,
            ParamConvention::Observe | ParamConvention::Mutate
        ) && self.eat(&Token::CurlyOpen)
        {
            let group = self.parse_id()?;
            self.expect(&Token::CurlyClose)?;
            Some(group)
        } else {
            None
        };

        // Named: id [Tag | id | : expr]
        let name = match self.peek() {
            Some(Token::Id(n)) => {
                let id = self.intern(n);
                self.advance();
                id
            }
            Some(Token::SelfInstance) => {
                self.advance();
                Intern::from_ref("self")
            }
            _ => return None,
        };

        let (name, kind) = self.parse_param_after_name(expr_parser, name, false)?;
        let kind = match (convention, kind) {
            (ParamConvention::Observe | ParamConvention::Mutate, ParameterKind::Tagged(inner)) => {
                let span_id = inner.span_id;
                ParameterKind::Tagged(Box::new(Spanned {
                    value: TypeExpr::Ref {
                        inner,
                        mutable: convention == ParamConvention::Mutate,
                        group: group_name,
                    },
                    span_id,
                }))
            }
            (_, kind) => kind,
        };
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
        Some((name, kind, convention, group_name, refinement))
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
