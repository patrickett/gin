use ast::span::{SpanId, SpanTable};
use lexer::Token;
use std::collections::HashSet;

use ast::warnings::BindWarningsExt;
use ast::{
    Bind, BindValue, Declare, DeclareValue, Expr, FileAst, HasSpanId, ImplBlock, ParameterKind,
    Spanned, TypeExpr, Typed, Variant,
};
use indexmap::IndexMap;
use internment::Intern;

use crate::cursor::TokenCursor;
use crate::expr::ExprFn;

enum TopLevelValue {
    Tag(Declare),
    Bind(Box<Bind>),
    ImplBlock(ImplBlock),
    BlanketImpl(ast::BlanketImpl),
    ProvidedImpl(Intern<String>, ast::ProvidedTrait),
    Expr(Typed<Expr>),
}

impl TokenCursor<'_, '_> {
    pub fn parse_file(&mut self, expr_parser: ExprFn) -> FileAst {
        let module_doc = self.parse_module_doc();
        let imports = self.parse_imports();

        let mut public_elements = Vec::new();
        loop {
            self.advance_push();
            match self.parse_element_line(expr_parser) {
                Some(el) => {
                    self.advance_pop();
                    public_elements.push(el);
                }
                None => {
                    self.advance_drop();
                    break;
                }
            }
        }

        let mut private_elements = Vec::new();
        self.skip_newlines();
        if self.eat(&Token::Private) {
            self.skip_newlines();
            loop {
                self.advance_push();
                match self.parse_element_line(expr_parser) {
                    Some(el) => {
                        self.advance_pop();
                        private_elements.push(el);
                    }
                    None => {
                        self.advance_drop();
                        break;
                    }
                }
            }
        }

        let mut tags_scratch: IndexMap<Intern<String>, Vec<Declare>> = IndexMap::new();
        let mut defs_scratch: IndexMap<Intern<String>, Vec<Bind>> = IndexMap::new();
        let mut private_defs = HashSet::new();
        let mut private_tags = HashSet::new();
        let mut exprs = Vec::new();
        let mut blanket_impls = Vec::new();
        let mut provided_impls: IndexMap<Intern<String>, Vec<ast::ProvidedTrait>> = IndexMap::new();

        for el in public_elements {
            Self::collect_top_level(
                el,
                &mut tags_scratch,
                &mut defs_scratch,
                &mut exprs,
                &mut blanket_impls,
                &mut provided_impls,
            );
        }

        for el in private_elements {
            match &el {
                TopLevelValue::Tag(decl) => {
                    private_tags.insert(decl.name);
                }
                TopLevelValue::Bind(bind) => {
                    private_defs.insert(bind.name);
                }
                TopLevelValue::ImplBlock(block) => {
                    for method_name in block.methods.keys() {
                        let mangled = Intern::<String>::new(format!(
                            "{}.{}",
                            block.type_name.as_str(),
                            method_name.as_str()
                        ));
                        private_defs.insert(mangled);
                    }
                }
                TopLevelValue::BlanketImpl(..)
                | TopLevelValue::ProvidedImpl(..)
                | TopLevelValue::Expr(..) => {}
            }
            Self::collect_top_level(
                el,
                &mut tags_scratch,
                &mut defs_scratch,
                &mut exprs,
                &mut blanket_impls,
                &mut provided_impls,
            );
        }

        let mut tags = ast::TagMap::new();
        for (name, declares) in tags_scratch {
            if let Some(mut decl) = declares.into_iter().next() {
                if let Some(mut impls) = provided_impls.swap_remove(&name) {
                    decl.provided_traits.append(&mut impls);
                }
                tags.insert(name, decl);
            }
        }
        for (name, impls) in provided_impls {
            tags.insert(
                name,
                Declare::new(name, SpanId::INVALID, DeclareValue::Interface(Vec::new()))
                    .with_provided_traits(impls),
            );
        }
        Self::expand_composed_provided_traits(&mut tags);
        let mut defs = ast::DefMap::new();
        let mut parse_warnings = Vec::new();
        for (name, binds) in defs_scratch {
            parse_warnings.extend(
                binds
                    .iter()
                    .filter_map(|bind| bind.name_case_warning(self.span_table())),
            );
            parse_warnings.extend(binds.const_bind_after_declare_warnings(self.span_table()));
            if let Some(bind) = binds.into_iter().last() {
                defs.insert(name, bind);
            }
        }
        Self::generate_return_type_unions(&defs, &mut tags, &private_defs);

        FileAst {
            module_doc,
            uses: imports,
            tags,
            defs,
            private_defs,
            private_tags,
            exprs,
            symbol_aliases: Vec::new(),
            symbol_alias_spans: Vec::new(),
            span_table: SpanTable::new(),
            blanket_impls,
            parse_warnings,
        }
    }

    fn parse_module_doc(&mut self) -> Option<ast::DocComment> {
        self.skip_newlines();

        let first = match self.peek()? {
            Token::ModuleDocComment(text) => {
                let stripped = text
                    .strip_prefix("--|")
                    .map(|s| s.trim_start())
                    .unwrap_or(text)
                    .to_owned();
                self.advance();
                stripped
            }
            _ => return None,
        };

        // Fast path: single-line module doc
        if !matches!(self.peek(), Some(Token::ModuleDocComment(_))) {
            let doc = ast::DocComment { value: first };
            return if doc.is_empty() { None } else { Some(doc) };
        }

        let mut lines = vec![first];
        while let Some(Token::ModuleDocComment(text)) = self.peek() {
            let stripped = text
                .strip_prefix("--|")
                .map(|s| s.trim_start())
                .unwrap_or(text)
                .to_owned();
            self.advance();
            lines.push(stripped);
        }

        let doc = ast::DocComment {
            value: lines.join("\n"),
        };
        if doc.is_empty() { None } else { Some(doc) }
    }

    fn parse_imports(&mut self) -> Vec<ast::Import> {
        let mut imports = Vec::new();
        while self.is_at(&Token::Use) {
            self.advance_push();
            match self.parse_import() {
                Some(import) => {
                    self.advance_pop();
                    imports.push(import);
                }
                None => {
                    self.advance_drop();
                    break;
                }
            }
        }
        imports
    }

    fn can_start_top_level_after_dedent(&self) -> bool {
        let start = self.skip_metadata_offset();
        match self.peek_at(start) {
            Some(Token::Tag(_)) => true,
            Some(Token::Id(_)) => matches!(
                self.peek_at(start + 1),
                Some(Token::Colon)
                    | Some(Token::ColonEq)
                    | Some(Token::ParenOpen)
                    | Some(Token::Has)
                    | Some(Token::Dot)
            ),
            _ => false,
        }
    }

    fn parse_top_level_element(&mut self, expr_parser: ExprFn) -> Option<TopLevelValue> {
        if self.is_eof() {
            return None;
        }

        if matches!(self.peek(), Some(Token::ModuleDocComment(_))) {
            self.error(
                "parse-module-doc-comment-position",
                "module doc comments (--|) are only allowed at the start of the file",
                self.peek_span().unwrap_or(SpanId::INVALID),
            );
            self.advance();
            self.consume_trailing_newline();
            return None;
        }

        if self.is_at(&Token::Dedent) {
            self.advance();
            if self.can_start_top_level_after_dedent() {
                return self.parse_top_level_element(expr_parser);
            }
            return None;
        }

        match self.peek() {
            Some(Token::Private) | Some(Token::ParenClose) | Some(Token::Indent) => {
                return None;
            }
            _ => {}
        }

        let start_offset = self.skip_metadata_offset();
        let effective = self.peek_at(start_offset)?;

        match effective {
            Token::Tag(_) => self.dispatch_tag_element(expr_parser, start_offset),
            Token::Id(_) => {
                if self.is_dot_blanket_impl_start() {
                    if let Some(blanket) = self.parse_dot_blanket_impl(expr_parser) {
                        return Some(TopLevelValue::BlanketImpl(blanket));
                    }
                    return None;
                }
                if self.is_blanket_impl_start() {
                    if let Some(blanket) = self.parse_blanket_impl(expr_parser) {
                        return Some(TopLevelValue::BlanketImpl(blanket));
                    }
                    return None;
                }
                // Deterministic dispatch: if next token after id is : or :=, it's definitely a bind.
                // No checkpoint/rewind needed for the common case (x: expr, x := expr).
                // For id(...) and id Tag, use speculative parsing only for the truly ambiguous cases.
                if matches!(
                    self.peek_at(start_offset + 1),
                    Some(Token::Colon) | Some(Token::ColonEq)
                ) {
                    // id: or id:= → bind, no speculation needed
                    if let Some(bind) = self.parse_bind(expr_parser) {
                        return Some(TopLevelValue::Bind(Box::new(bind)));
                    }
                } else if matches!(self.peek_at(start_offset + 1), Some(Token::ParenOpen)) {
                    // Could be function def (id(...) RetType:) — speculative
                    let checkpoint = self.checkpoint();
                    if let Some(bind) = self.parse_bind(expr_parser) {
                        return Some(TopLevelValue::Bind(Box::new(bind)));
                    }
                    self.rewind(checkpoint);
                } else if matches!(self.peek_at(start_offset + 1), Some(Token::BracketOpen)) {
                    // id[...](...) or id[...] Tag: — group annotations + optional params
                    let checkpoint = self.checkpoint();
                    if let Some(bind) = self.parse_bind(expr_parser) {
                        return Some(TopLevelValue::Bind(Box::new(bind)));
                    }
                    self.rewind(checkpoint);
                } else if matches!(self.peek_at(start_offset + 1), Some(Token::Tag(_))) {
                    // `arch Architecture` — declare without value (same as unassigned bind).
                    let is_type_only_declare = !matches!(
                        self.peek_at(start_offset + 2),
                        Some(Token::Colon) | Some(Token::ColonEq)
                    );
                    if is_type_only_declare {
                        if let Some(bind) = self.parse_bind(expr_parser) {
                            return Some(TopLevelValue::Bind(Box::new(bind)));
                        }
                    } else {
                        // `id Tag: value` — typed bind with inline type annotation
                        let checkpoint = self.checkpoint();
                        if let Some(bind) = self.parse_bind(expr_parser) {
                            return Some(TopLevelValue::Bind(Box::new(bind)));
                        }
                        self.rewind(checkpoint);
                    }
                }
                // else: bare identifier or expression — no bind speculation needed
                let expr = expr_parser(self);
                // Feature 3: Detect `{expr} {expr}` on the same line without an operator.
                if let Some(next_tok) = self.peek_at(0)
                    && Self::can_start_expr(next_tok)
                {
                    self.error(
                        "parse-expected-operator",
                        format!("expected operator between expressions, found {next_tok:?}"),
                        self.peek_span().unwrap_or(expr.span_id),
                    );
                }
                Some(TopLevelValue::Expr(expr))
            }
            Token::Pound => {
                // #[...] always starts a bind, no speculation needed
                if let Some(bind) = self.parse_bind(expr_parser) {
                    return Some(TopLevelValue::Bind(Box::new(bind)));
                }
                let expr = expr_parser(self);
                Some(TopLevelValue::Expr(expr))
            }
            _ => {
                let expr = expr_parser(self);
                Some(TopLevelValue::Expr(expr))
            }
        }
    }

    fn dispatch_tag_element(
        &mut self,
        expr_parser: ExprFn,
        tag_offset: usize,
    ) -> Option<TopLevelValue> {
        let after_tag = tag_offset + 1;

        // Helper: given an offset that may be at a method separator (. or ::),
        // return the offset just past it if found.
        let sep_past = |offset: usize| -> Option<usize> {
            if self.peek_at(offset) == Some(&Token::Dot) {
                Some(offset + 1)
            } else if matches!(self.peek_at(offset), Some(Token::Colon))
                && self.peek_at(offset + 1) == Some(&Token::Colon)
            {
                Some(offset + 2)
            } else {
                None
            }
        };

        // Tag.Tag → provided interface implementation, with legacy impl-block fallback.
        if matches!(self.peek_at(after_tag), Some(Token::Dot))
            && matches!(self.peek_at(after_tag + 1), Some(Token::Tag(_)))
        {
            let checkpoint = self.checkpoint();
            if let Some((type_name, provided_trait)) = self.parse_dot_provided_impl(expr_parser) {
                return Some(TopLevelValue::ProvidedImpl(type_name, provided_trait));
            }
            self.rewind(checkpoint);
            return self
                .parse_impl_block(expr_parser)
                .map(TopLevelValue::ImplBlock);
        }

        // Tag.Id or Tag::Id → method_bind (deterministic: no checkpoint/rewind needed)
        if let Some(past_sep) = sep_past(after_tag)
            && matches!(
                self.peek_at(past_sep),
                Some(Token::Id(_)) | Some(Token::Tag(_))
            )
        {
            return self.parse_method_bind(expr_parser);
        }

        // Tag(...).Id / Tag(...)::Id or Tag[...].Id / Tag[...]::Id → generic-receiver method_bind
        if let Some(after_parens) = self.skip_balanced_parens_offset(after_tag)
            && let Some(past_sep) = sep_past(after_parens)
            && matches!(
                self.peek_at(past_sep),
                Some(Token::Id(_)) | Some(Token::Tag(_))
            )
        {
            return self.parse_method_bind(expr_parser);
        }

        if let Some(after_brackets) = self.skip_balanced_brackets_offset(after_tag)
            && let Some(past_sep) = sep_past(after_brackets)
            && matches!(
                self.peek_at(past_sep),
                Some(Token::Id(_)) | Some(Token::Tag(_))
            )
        {
            return self.parse_method_bind(expr_parser);
        }

        // Tag [params] is/has → declare (deterministic: no checkpoint/rewind needed)
        if self.is_declare_from_offset(tag_offset) {
            return self.parse_declare(expr_parser).map(TopLevelValue::Tag);
        }

        // fallback: expression (bare Tag, Tag(args), etc.)
        let expr = expr_parser(self);

        // Check for destructure bind: Tag(args) := value
        if let Expr::TagCall(tc) = &expr.value
            && self.is_at(&Token::ColonEq)
        {
            self.advance(); // :=
            let value = expr_parser(self);
            let value_span = value.span_id();
            let field_bindings: Vec<(Intern<String>, Intern<String>)> = tc
                .args
                .iter()
                .map(|arg| match &arg.value {
                    Expr::Bind(b) => {
                        let field_name = b.name;
                        let bind_name = match &b.value {
                            BindValue::Expr(e) => match &e.value {
                                Expr::FnCall(fc) => fc.path.root,
                                _ => b.name,
                            },
                            _ => b.name,
                        };
                        (field_name, bind_name)
                    }
                    // Shorthand: bare Id in arg position
                    Expr::FnCall(fc) => (fc.path.root, fc.path.root),
                    _ => (Intern::new(String::new()), Intern::new(String::new())),
                })
                .collect();
            return Some(TopLevelValue::Expr(Typed::infer(
                Expr::Destructure {
                    tag_name: tc.name,
                    field_bindings,
                    value: Box::new(value),
                },
                self.merge_span(expr.span_id, value_span),
            )));
        }

        Some(TopLevelValue::Expr(expr))
    }

    /// If the token at `offset` is an opening delimiter, return the offset just
    /// past the matching close. Returns `None` if there is no opener at `offset`
    /// or the delimiters are unbalanced.
    fn skip_balanced_delimiters_offset(
        &self,
        offset: usize,
        open: Token<'static>,
        close: Token<'static>,
    ) -> Option<usize> {
        if !matches!(self.peek_at(offset), Some(t) if *t == open) {
            return None;
        }
        let mut o = offset + 1;
        let mut depth = 1;
        while depth > 0 {
            match self.peek_at(o) {
                Some(t) if *t == open => {
                    depth += 1;
                    o += 1;
                }
                Some(t) if *t == close => {
                    depth -= 1;
                    o += 1;
                }
                None => return None,
                _ => {
                    o += 1;
                }
            }
        }
        Some(o)
    }

    fn skip_balanced_parens_offset(&self, offset: usize) -> Option<usize> {
        self.skip_balanced_delimiters_offset(offset, Token::ParenOpen, Token::ParenClose)
    }

    fn skip_balanced_brackets_offset(&self, offset: usize) -> Option<usize> {
        self.skip_balanced_delimiters_offset(offset, Token::BracketOpen, Token::BracketClose)
    }

    fn parse_element_line(&mut self, expr_parser: ExprFn) -> Option<TopLevelValue> {
        let el = self.parse_top_level_element(expr_parser)?;
        Some(el)
    }

    fn parse_method_bind(&mut self, expr_parser: ExprFn) -> Option<TopLevelValue> {
        // skip past any metadata (indent, doc comments, attributes) to reach the Tag
        let mut doc_before = None;
        loop {
            match self.peek() {
                Some(Token::Indent) => {
                    self.advance();
                }
                Some(Token::DocComment(_)) => {
                    if let Some(doc) = self.parse_doc_comment() {
                        doc_before = Some(doc);
                    }
                }
                Some(Token::Pound) => {
                    self.advance();
                    loop {
                        match self.peek() {
                            Some(Token::BracketClose) => {
                                self.advance();
                                break;
                            }
                            None => return None,
                            _ => {
                                self.advance();
                            }
                        }
                    }
                }
                _ => break,
            }
        }

        // Receiver may be a bare Tag, a generic Tag(args), or a qualified Mod.Tag.
        let recv = self.parse_type_expr(expr_parser)?;

        // Accept `.` (Type.method) or `::` (Type::method) as the separator.
        let has_sep = self.eat(&Token::Dot) || (self.eat(&Token::Colon) && self.eat(&Token::Colon));
        if !has_sep {
            return None;
        }

        let mut bind = self.parse_bind(expr_parser)?;
        let doc = bind.doc_comment.as_ref().cloned().or(doc_before);
        bind = bind.with_doc(doc);
        let bind = bind.with_receiver_type(Some(Box::new(recv)));

        Some(TopLevelValue::Bind(Box::new(bind)))
    }

    fn collect_top_level(
        el: TopLevelValue,
        tags: &mut IndexMap<Intern<String>, Vec<Declare>>,
        defs: &mut IndexMap<Intern<String>, Vec<Bind>>,
        exprs: &mut Vec<(Expr, SpanId)>,
        blanket_impls: &mut Vec<ast::BlanketImpl>,
        provided_impls: &mut IndexMap<Intern<String>, Vec<ast::ProvidedTrait>>,
    ) {
        match el {
            TopLevelValue::BlanketImpl(b) => blanket_impls.push(b),
            TopLevelValue::ProvidedImpl(type_name, provided_trait) => {
                provided_impls
                    .entry(type_name)
                    .or_default()
                    .push(provided_trait);
            }
            TopLevelValue::Tag(decl) => {
                let name = decl.name;
                tags.entry(name).or_default().push(decl);
            }
            TopLevelValue::Bind(bind) => {
                let name = if let Some(sp) = bind.receiver_type_surface() {
                    Intern::<String>::new(format!(
                        "{}.{}",
                        sp.value.surface_mangle_name(),
                        bind.name
                    ))
                } else {
                    bind.name
                };
                defs.entry(name).or_default().push(*bind);
            }
            TopLevelValue::ImplBlock(block) => {
                let recv = Box::new(Spanned {
                    value: TypeExpr::Nominal(block.type_name, block.type_name_span),
                    span_id: block.type_name_span,
                });
                for (method_name, bind) in block.methods {
                    let bind = bind.with_receiver_type(Some(recv.clone()));
                    let mangled = Intern::<String>::new(format!(
                        "{}.{}",
                        block.type_name.as_str(),
                        method_name.as_str()
                    ));
                    defs.entry(mangled).or_default().push(bind);
                }
            }
            TopLevelValue::Expr(expr) => {
                exprs.push((expr.value, expr.span_id));
            }
        }
    }

    fn expand_composed_provided_traits(tags: &mut ast::TagMap) {
        let snapshot = tags.clone();
        for decl in tags.values_mut() {
            let mut seen: HashSet<Intern<String>> = decl
                .provided_traits
                .iter()
                .map(|pt| pt.trait_name)
                .collect();
            let mut i = 0;
            while i < decl.provided_traits.len() {
                let trait_name = decl.provided_traits[i].trait_name;
                if let Some(composed) = snapshot.get(&trait_name) {
                    for component in &composed.provided_traits {
                        if seen.insert(component.trait_name) {
                            decl.provided_traits.push(component.clone());
                        }
                    }
                }
                i += 1;
            }
        }
    }

    fn generate_return_type_unions(
        defs: &ast::DefMap,
        tags: &mut ast::TagMap,
        _private_defs: &HashSet<Intern<String>>,
    ) {
        // Only adds new tags from bind return types; existing tag map is unchanged.
        let mut tag_buffer = Vec::new(); // reused across iterations
        for bind in defs.values() {
            tag_buffer.clear();
            Self::extract_anonymous_tags_from_bind(bind, &mut tag_buffer);
            if tag_buffer.is_empty() {
                continue;
            }

            let unique_tags: HashSet<_> = tag_buffer.drain(..).collect();
            let variants: Vec<Variant> = unique_tags
                .into_iter()
                .map(|(name, span)| Variant::External {
                    shape: Box::new(Spanned {
                        value: TypeExpr::Nominal(name, span),
                        span_id: span,
                    }),
                    result_ty: None,
                })
                .collect();

            if let Some(name) = bind.return_type_name() {
                let decl = Declare::new(*name, SpanId::INVALID, DeclareValue::Union { variants });
                tags.insert(decl.name, decl);
            }
        }
    }

    fn skip_metadata_offset(&self) -> usize {
        let mut offset = 0;
        loop {
            match self.peek_at(offset) {
                Some(Token::DocComment(_)) | Some(Token::Newline) | Some(Token::Indent) => {
                    offset += 1;
                }
                Some(Token::Pound) => {
                    offset += 1;
                    // Track bracket nesting to handle `#[attr([inner])]` correctly.
                    // Depth starts at 0; the `BracketOpen` immediately after `#`
                    // increments to 1, and we break when it returns to 0.
                    let mut depth = 0u32;
                    loop {
                        match self.peek_at(offset) {
                            Some(Token::BracketOpen) => {
                                depth += 1;
                                offset += 1;
                            }
                            Some(Token::BracketClose) => {
                                depth -= 1;
                                offset += 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            None => return offset,
                            _ => {
                                offset += 1;
                            }
                        }
                    }
                }
                _ => return offset,
            }
        }
    }

    fn is_declare_from_offset(&self, tag_offset: usize) -> bool {
        let mut offset = tag_offset + 1;
        if let Some(next) = self.skip_balanced_parens_offset(offset) {
            offset = next;
        } else if let Some(next) = self.skip_balanced_brackets_offset(offset) {
            offset = next;
        }

        matches!(self.peek_at(offset), Some(Token::Is) | Some(Token::Has))
    }

    fn extract_anonymous_tags_from_bind(bind: &Bind, tags: &mut Vec<(Intern<String>, SpanId)>) {
        if let Some(sp) = bind.receiver_type_surface() {
            Self::collect_type_surface_tags(&sp.value, tags);
        }
        if let Some(sp) = &bind.return_tag {
            Self::collect_type_surface_tags(&sp.value, tags);
        }
    }

    fn collect_type_surface_tags(expr: &TypeExpr, tags: &mut Vec<(Intern<String>, SpanId)>) {
        match expr {
            TypeExpr::Nominal(name, span) => {
                tags.push((*name, *span));
            }
            TypeExpr::Qualified(_) => {}
            TypeExpr::Literal(..) => {}
            TypeExpr::Pointer(_) | TypeExpr::Unit | TypeExpr::ListEmpty => {}
            TypeExpr::ListCons { head, tail } => {
                Self::collect_type_surface_tags(&head.value, tags);
                Self::collect_type_surface_tags(&tail.value, tags);
            }
            TypeExpr::Tuple(elems) => {
                for e in elems {
                    Self::collect_type_surface_tags(&e.value, tags);
                }
            }
            TypeExpr::InRange { bounds, span } => {
                if let ast::InRangeBounds::Tag(name) = bounds {
                    tags.push((*name, *span));
                }
            }
            TypeExpr::Ref { inner, .. } => Self::collect_type_surface_tags(&inner.value, tags),
            TypeExpr::Generic { params, .. } => {
                for (_, pk) in params {
                    match pk {
                        ParameterKind::Default(_e) => {
                            // Anonymous tags in default expressions are handled
                            // by collect_type_surface_tags elsewhere.
                        }
                        ParameterKind::Tagged(sp) | ParameterKind::ValueParam { ty: sp } => {
                            Self::collect_type_surface_tags(&sp.value, tags);
                        }
                        ParameterKind::Generic => {}
                    }
                }
            }
        }
    }
}
