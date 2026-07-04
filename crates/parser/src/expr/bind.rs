use indexmap::IndexMap;
use internment::Intern;
use lexer::Token;

use ast::{
    AttributeItem, Bind, BindAttributes, BindValue, DocComment, Expr, GroupParam, ModPath,
    ParamConvention, ParameterKind, Parameters, Return, Spanned, TypeExpr, Typed,
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
);

pub(crate) type ParsedOneParam = (
    Intern<String>,
    ParameterKind,
    ParamConvention,
    Option<Intern<String>>,
);

impl<'src, 't> TokenCursor<'src, 't> {
    pub fn parse_bind(&mut self, expr_parser: ExprFn) -> Option<Bind> {
        self.skip_newlines();

        // Doc comments may appear before or after attributes; try both positions.
        let doc_before_attrs = self.parse_doc_comment();
        let attrs = self.parse_bind_attributes();
        let doc_before = self.parse_doc_comment().or(doc_before_attrs);

        self.eat(&Token::Indent);

        let (name, name_span) = match self.peek() {
            Some(Token::Id(n)) => {
                let name = self.intern(n);
                let span = self.peek_span()?;
                self.advance();
                (name, span)
            }
            _ => return None,
        };

        let group_params = self.parse_group_params();
        let (params, conventions, param_groups) = self.parse_params(expr_parser);

        let (return_type_name, return_tag, type_annotation, type_annotation_qual) =
            self.parse_return_type_part(expr_parser);

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
                self.error("parse-expected-bind-operator", "expected ':', ':=' or 'extern'", self.current_span());
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
        bind.return_tag = return_tag;
        bind.type_annotation = type_annotation;
        bind.type_annotation_qual = type_annotation_qual;

        Some(bind)
    }

    pub fn parse_bind_attributes(&mut self) -> Option<BindAttributes> {
        if !self.is_at(&Token::Pound) {
            return None;
        }
        self.advance();

        self.expect(&Token::BracketOpen)?;

        let mut items: Vec<AttributeItem> = Vec::new();

        if !self.is_at(&Token::BracketClose) {
            loop {
                if let Some(item) = self.parse_one_attribute_item() {
                    items.push(item);
                }
                if self.eat(&Token::Comma) {
                    continue;
                }
                break;
            }
        }

        self.expect(&Token::BracketClose);

        let mut attrs = BindAttributes {
            raw_attributes: Some(items),
            ..Default::default()
        };
        attrs.extract_intrinsic_attributes();
        Some(attrs)
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

        // `ref T` / `mut T` return type (e.g. `r ref Entity:` in a body bind)
        if matches!(self.peek(), Some(Token::Ref) | Some(Token::Mut)) {
            let mutable = self.eat(&Token::Mut);
            if !mutable {
                self.eat(&Token::Ref);
            }
            if let Some(sp) = self.parse_type_expr(expr_parser) {
                return (
                    None,
                    Some(Box::new(Spanned {
                        value: TypeExpr::Ref {
                            inner: Box::new(Spanned {
                                value: sp.value,
                                span_id: sp.span_id,
                            }),
                            mutable,
                        },
                        span_id: sp.span_id,
                    })),
                    None,
                    None,
                );
            }
            return (None, None, None, None);
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

    fn parse_bind_value(&mut self, expr_parser: ExprFn) -> (BindValue, Option<DocComment>) {
        // extern → BindValue::Extern
        if self.eat(&Token::Extern) {
            return (BindValue::Extern, None);
        }

        // Indent → multi-line body
        if self.is_at(&Token::Indent) {
            let _has_indent = self.eat(&Token::Indent);

            let exprs = self.parse_body_exprs(expr_parser);

            // Dedent is optional — return may appear inside or outside the indented block
            self.eat(&Token::Dedent);

            let ret = self.parse_return(expr_parser).unwrap_or_else(|| {
                self.error("parse-expected-return", "expected 'return' after bind body", self.current_span());
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

    fn parse_group_params(&mut self) -> Vec<GroupParam> {
        if !self.is_at(&Token::BracketOpen) {
            return Vec::new();
        }
        self.advance();

        let mut groups = Vec::new();
        if !self.is_at(&Token::BracketClose) {
            loop {
                let mutable = self.eat(&Token::Mut);
                let Some(name) = self.parse_id() else {
                    break;
                };
                let ty_name = match self.peek() {
                    Some(Token::Tag(t)) => {
                        let ty = self.intern(t);
                        self.advance();
                        ty
                    }
                    Some(Token::Id(t)) => {
                        let ty = self.intern(t);
                        self.advance();
                        ty
                    }
                    _ => break,
                };
                groups.push(GroupParam {
                    name,
                    ty_name,
                    mutable,
                });
                if !self.eat(&Token::Comma) {
                    break;
                }
            }
        }

        self.expect(&Token::BracketClose);
        groups
    }

    pub(crate) fn parse_params(&mut self, expr_parser: ExprFn) -> ParsedParams {
        if !self.is_at(&Token::ParenOpen) {
            return (None, IndexMap::new(), IndexMap::new());
        }

        let mut params = Parameters::new();
        let mut conventions = IndexMap::new();
        let mut param_groups = IndexMap::new();
        let mut seen_default = false;
        self.advance();

        // Skip layout after opening paren: allows `(\n  a Int, b Int)` style.
        self.skip_layout();

        if self.is_at(&Token::ParenClose) {
            self.advance();
            return (Some(params), conventions, param_groups);
        }

        let mut ingest = |key: Intern<String>,
                          kind: ParameterKind,
                          conv: ParamConvention,
                          group: Option<Intern<String>>| {
            params.insert(key, kind);
            if conv != ParamConvention::Inferred {
                conventions.insert(key, conv);
            }
            if let Some(gn) = group {
                param_groups.insert(key, gn);
            }
        };

        loop {
            if let Some((key, kind, conv, group)) = self.parse_one_param(expr_parser) {
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
                ingest(key, kind, conv, group);
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
        (Some(params), conventions, param_groups)
    }

    pub(crate) fn parse_param_convention_prefix(&mut self) -> ParamConvention {
        if self.eat(&Token::Tilde) || self.eat(&Token::Eat) {
            ParamConvention::Eat
        } else if self.eat(&Token::Ref) {
            ParamConvention::Ref(false)
        } else if self.eat(&Token::Mut) {
            ParamConvention::Ref(true)
        } else {
            ParamConvention::Inferred
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
                ParamConvention::Inferred,
                None,
            ));
        }

        let convention = self.parse_param_convention_prefix();

        let group_name = if self.is_at(&Token::BracketOpen) {
            self.advance();
            let gn = self.parse_id()?;
            self.expect(&Token::BracketClose)?;
            Some(gn)
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
        Some((name, kind, convention, group_name))
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
