use ast::folder::{Folder, ParameterSubstitution};
use ast::span::{SpanId, SpanTable};
use diagnostic::DiagnosticCode;
use diagnostic::code::{ExpectedSyntaxCode, ParseCode, SyntaxCode};
use lexer::Token;
use std::collections::{HashMap, HashSet};

use ast::warnings::BindWarningsExt;
use ast::{
    Bind, BindValue, Declare, DeclareValue, Expr, FileAst, HasSpanId, Spanned, Typed, Variant,
};
use indexmap::IndexMap;
use internment::Intern;

use crate::SourceDeclaration;
use crate::cursor::TokenCursor;
use crate::expr::ExprFn;

enum TopLevelValue {
    Tag(Box<Declare>),
    Bind(Box<Bind>),
    Expr(Box<Typed<Expr>>),
}

fn binding_name(bind: &Bind) -> Intern<String> {
    let Some(receiver) = bind.receiver_type_surface() else {
        return bind.name;
    };
    let name = match &receiver.value {
        Expr::AnonymousTag(name) => *name,
        Expr::TagCall(call) => call.name,
        Expr::FnCall(call) => call
            .path
            .value
            .segments
            .last()
            .copied()
            .unwrap_or(call.path.value.root),
        _ => Intern::from_ref("<receiver>"),
    };
    Intern::new(format!("{name}.{}", bind.name))
}

impl TokenCursor<'_, '_> {
    /// Parse a file and apply parser-level lowering, including canonical method
    /// bind key rewriting.
    ///
    /// This is the public parser entrypoint for this cursor and always emits a
    /// canonicalized `FileAst` where method call keys match the parser lowering
    /// contract.
    pub fn parse_file(&mut self, expr_parser: ExprFn) -> FileAst {
        let mut ast = self.parse_file_without_lowering(expr_parser);
        crate::lowering::lower(&mut ast);
        ast
    }

    /// Parse a raw, pre-lowering AST for parser-internal and test usage only.
    /// Callers should use [`parse_file`](Self::parse_file) for canonicalized
    /// outputs.
    pub(crate) fn parse_file_without_lowering(&mut self, expr_parser: ExprFn) -> FileAst {
        self.parse_file_recording(expr_parser, None)
    }

    pub(crate) fn parse_file_recording(
        &mut self,
        expr_parser: ExprFn,
        mut declarations: Option<&mut Vec<SourceDeclaration>>,
    ) -> FileAst {
        let module_doc = self.parse_module_doc();
        let imports = self.parse_imports(declarations.as_deref_mut());

        let mut public_elements = Vec::new();
        loop {
            self.advance_push();
            let start = self.pos();
            match self.parse_element_line(expr_parser) {
                Some(el) => {
                    self.advance_pop();
                    if let Some(declarations) = &mut declarations {
                        self.record_source_declaration(start, &el, declarations);
                    }
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
                let start = self.pos();
                match self.parse_element_line(expr_parser) {
                    Some(el) => {
                        self.advance_pop();
                        if let Some(declarations) = &mut declarations {
                            self.record_source_declaration(start, &el, declarations);
                        }
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

        for el in public_elements {
            Self::collect_top_level(el, &mut tags_scratch, &mut defs_scratch, &mut exprs);
        }

        for el in private_elements {
            match &el {
                TopLevelValue::Tag(decl) => {
                    private_tags.insert(decl.name);
                }
                TopLevelValue::Bind(bind) => {
                    private_defs.insert(bind.name);
                }
                TopLevelValue::Expr(..) => {}
            }
            Self::collect_top_level(el, &mut tags_scratch, &mut defs_scratch, &mut exprs);
        }

        let method_binds = Self::materialize_has_method_binds(
            tags_scratch
                .values()
                .flat_map(|declarations| declarations.iter()),
        );
        let mut tags = ast::TagMap::new();
        for (name, declares) in tags_scratch {
            let mut declarations = declares.into_iter();
            if let Some(mut declaration) = declarations.next() {
                for extension in declarations {
                    declaration
                        .provided_traits
                        .extend(extension.provided_traits.iter().cloned());
                    declaration.extensions.push(extension);
                }
                tags.insert(name, declaration);
            }
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
            if binds.len() > 1
                && binds.iter().all(|bind| {
                    bind.params.is_some() && !matches!(bind.value, BindValue::Unassigned)
                })
            {
                for (index, bind) in binds.into_iter().enumerate() {
                    let symbol = ast::DefinitionId::member(name, index as u32);
                    defs.insert(symbol, bind);
                }
            } else if let Some(bind) = binds.into_iter().last() {
                defs.insert(name, bind);
            }
        }
        for bind in &method_binds {
            parse_warnings.extend(bind.name_case_warning(self.span_table()));
            parse_warnings.extend(
                std::slice::from_ref(bind).const_bind_after_declare_warnings(self.span_table()),
            );
        }
        Self::generate_return_type_unions(&defs, &mut tags, &private_defs);

        self.todo_holes.sort_by_key(|span| span.start());
        self.todo_holes.dedup();
        let mut ast = FileAst {
            semantic_origin: None,
            module_doc,
            uses: imports,
            tags,
            defs,
            method_binds,

            private_defs,
            private_tags,
            exprs,
            symbol_aliases: Vec::new(),
            symbol_alias_spans: Vec::new(),
            span_table: SpanTable::new(),
            line_breaks: self.source_line_breaks(),
            blank_lines: Vec::new(),
            parse_warnings,
            todo_holes: std::mem::take(&mut self.todo_holes),
        };
        ast.parse_warnings
            .extend(ast.hole_diagnostics(diagnostic::Category::Warn));
        ast
    }

    fn parse_module_doc(&mut self) -> Option<ast::DocComment> {
        self.skip_newlines();

        let start = self.peek_span()?;
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
            let doc = ast::DocComment::new(first).with_span(start);
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

        let span = self.merge_span(start, self.last_consumed_span());
        let doc = ast::DocComment::new(lines.join("\n")).with_span(span);
        if doc.is_empty() { None } else { Some(doc) }
    }

    fn parse_imports(
        &mut self,
        mut declarations: Option<&mut Vec<SourceDeclaration>>,
    ) -> Vec<ast::Import> {
        let mut imports = Vec::new();
        while self.is_at(&Token::Use) && !self.starts_reserved_binding() {
            self.advance_push();
            let start = self.pos();
            match self.parse_import() {
                Some(import) => {
                    self.advance_pop();
                    if let Some(declarations) = &mut declarations {
                        self.record_source_import(start, &import, declarations);
                    }
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

    fn record_source_declaration(
        &self,
        start: usize,
        element: &TopLevelValue,
        declarations: &mut Vec<SourceDeclaration>,
    ) {
        let (owner, mut names) = match element {
            TopLevelValue::Tag(declaration) => {
                let owner = declaration.name.to_string();
                let mut names = vec![owner.clone()];
                names.extend(
                    Self::materialize_has_method_binds(std::iter::once(declaration.as_ref()))
                        .iter()
                        .map(|bind| binding_name(bind).to_string()),
                );
                (owner, names)
            }
            TopLevelValue::Bind(bind) => {
                let owner = binding_name(bind).to_string();
                (owner.clone(), vec![owner])
            }
            TopLevelValue::Expr(_) => return,
        };
        names.sort();
        names.dedup();
        declarations.push(SourceDeclaration {
            owner: Some(owner),
            names,
            range: self
                .consumed_source_range(start)
                .expect("named declaration consumed tokens"),
        });
    }

    fn record_source_import(
        &self,
        start: usize,
        import: &ast::Import,
        declarations: &mut Vec<SourceDeclaration>,
    ) {
        let mut names = Vec::new();
        for module in &import.0 {
            match module.source {
                ast::ImportSource::LocalBundle(_) | ast::ImportSource::CurrentModule { .. } => {
                    names.extend(
                        module
                            .bound_name_spans()
                            .into_iter()
                            .map(|(name, _)| name.to_string()),
                    );
                }
                _ => names.push(
                    module
                        .alias
                        .map(|name| name.to_string())
                        .unwrap_or_else(|| module.effective_name()),
                ),
            }
        }
        names.sort();
        names.dedup();
        declarations.push(SourceDeclaration {
            owner: None,
            names,
            range: self
                .consumed_source_range(start)
                .expect("import consumed tokens"),
        });
    }

    fn can_start_top_level_after_dedent(&self) -> bool {
        let start = self.skip_metadata_offset();
        match self.peek_at(start) {
            Some(Token::Tag(_) | Token::Pound) => true,
            Some(Token::Id(_)) => matches!(
                self.peek_at(start + 1),
                Some(Token::Colon)
                    | Some(Token::ColonEq)
                    | Some(Token::At)
                    | Some(Token::ParenOpen)
                    | Some(Token::Has)
                    | Some(Token::Dot)
            ),
            _ => false,
        }
    }

    fn parse_top_level_element(&mut self, expr_parser: ExprFn) -> Option<TopLevelValue> {
        self.skip_newlines();
        if self.is_eof() {
            return None;
        }

        if matches!(self.peek(), Some(Token::ModuleDocComment(_))) {
            // parse-module-doc-comment-position
            self.error(
                DiagnosticCode::Parse(ParseCode::Syntax(SyntaxCode::ModuleDocumentationPosition)),
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

        if self.starts_reserved_binding() {
            return self
                .parse_bind(expr_parser)
                .map(|bind| TopLevelValue::Bind(Box::new(bind)));
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
                // Deterministic dispatch: if next token after id starts a type annotation or is :/:=, it's definitely a bind.
                // No checkpoint/rewind needed for the common case (x: expr, x := expr).
                // For id(...) and id Tag, use speculative parsing only for the truly ambiguous cases.
                if matches!(
                    self.peek_at(start_offset + 1),
                    Some(Token::At) | Some(Token::Colon) | Some(Token::ColonEq)
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
                } else if matches!(self.peek_at(start_offset + 1), Some(Token::Is)) {
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
                    // parse-expected-operator
                    self.error(
                        DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::Operator)),
                        format!("expected operator between expressions, found {next_tok:?}"),
                        self.peek_span().unwrap_or(expr.span_id),
                    );
                }
                Some(TopLevelValue::Expr(Box::new(expr)))
            }
            Token::Pound if self.peek_at(start_offset + 1) == Some(&Token::Id("todo")) => {
                Some(TopLevelValue::Expr(Box::new(expr_parser(self))))
            }
            Token::Pound => {
                let checkpoint = self.checkpoint();
                if let Some(bind) = self.parse_bind(expr_parser) {
                    return Some(TopLevelValue::Bind(Box::new(bind)));
                }
                self.rewind(checkpoint);
                if let Some(decl) = self.parse_declare(expr_parser) {
                    return Some(TopLevelValue::Tag(Box::new(decl)));
                }
                Some(TopLevelValue::Expr(Box::new(expr_parser(self))))
            }
            _ => {
                let expr = expr_parser(self);
                Some(TopLevelValue::Expr(Box::new(expr)))
            }
        }
    }

    fn dispatch_tag_element(
        &mut self,
        expr_parser: ExprFn,
        tag_offset: usize,
    ) -> Option<TopLevelValue> {
        let after_tag = tag_offset + 1;

        // Tag.Id or Tag::Id → method_bind (deterministic: no checkpoint/rewind needed)
        if let Some(past_sep) = self.method_separator_past(after_tag)
            && matches!(
                self.peek_at(past_sep),
                Some(Token::Id(_)) | Some(Token::Tag(_))
            )
        {
            return self.parse_method_bind(expr_parser);
        }

        // Tag(...).Tag → provided interface implementation (generic receiver)
        if let Some(after_parens) = self.skip_balanced_parens_offset(after_tag) {
            let past_sep = if self.peek_at(after_parens) == Some(&Token::Dot) {
                Some(after_parens + 1)
            } else if matches!(self.peek_at(after_parens), Some(Token::Colon))
                && self.peek_at(after_parens + 1) == Some(&Token::Colon)
            {
                Some(after_parens + 2)
            } else {
                None
            };
            if let Some(past_sep) = past_sep
                && matches!(self.peek_at(past_sep), Some(Token::Id(_)))
            {
                return self.parse_method_bind(expr_parser);
            }
        }

        if let Some(after_brackets) = self.skip_balanced_brackets_offset(after_tag)
            && let Some(past_sep) = self.method_separator_past(after_brackets)
            && matches!(
                self.peek_at(past_sep),
                Some(Token::Id(_)) | Some(Token::Tag(_))
            )
        {
            return self.parse_method_bind(expr_parser);
        }

        // Tag [params] is/has → declare (deterministic: no checkpoint/rewind needed)
        if self.is_declare_from_offset(tag_offset) {
            return self
                .parse_declare(expr_parser)
                .map(Box::new)
                .map(TopLevelValue::Tag);
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
            return Some(TopLevelValue::Expr(Box::new(Typed::infer(
                Expr::Destructure {
                    tag_name: tc.name,
                    field_bindings,
                    value: Box::new(value),
                },
                self.merge_span(expr.span_id, value_span),
            ))));
        }

        Some(TopLevelValue::Expr(Box::new(expr)))
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
        if self.is_at(&Token::Return) {
            // parse-return-outside-function
            self.error_here(
                DiagnosticCode::Parse(ParseCode::Syntax(SyntaxCode::ReturnOutsideFunction)),
                "return must appear inside a function body",
            );
        }
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

    fn materialize_has_method_binds<'a>(
        declarations: impl Iterator<Item = &'a Declare>,
    ) -> Vec<Bind> {
        let mut binds = Vec::new();
        for declare in declarations {
            let DeclareValue::Has(members) = &declare.value else {
                continue;
            };
            let receiver = declare.receiver_surface();

            for member in members {
                let ast::HasMember::Function(function) = member else {
                    continue;
                };
                if let Some(bind) = function.implementation(receiver.clone()) {
                    binds.push(bind);
                }
            }
        }
        binds
    }

    fn collect_top_level(
        el: TopLevelValue,
        tags: &mut IndexMap<Intern<String>, Vec<Declare>>,
        defs: &mut IndexMap<Intern<String>, Vec<Bind>>,
        exprs: &mut Vec<(Expr, SpanId)>,
    ) {
        match el {
            TopLevelValue::Tag(decl) => {
                let decl = *decl;
                let name = decl.name;
                tags.entry(name).or_default().push(decl);
            }
            TopLevelValue::Bind(bind) => {
                let name = binding_name(&bind);
                defs.entry(name).or_default().push(*bind);
            }
            TopLevelValue::Expr(expr) => {
                let expr = *expr;
                exprs.push((expr.value, expr.span_id));
            }
        }
    }

    fn method_separator_past(&self, offset: usize) -> Option<usize> {
        if self.peek_at(offset) == Some(&Token::Dot) {
            Some(offset + 1)
        } else if matches!(self.peek_at(offset), Some(Token::Colon))
            && self.peek_at(offset + 1) == Some(&Token::Colon)
        {
            Some(offset + 2)
        } else {
            None
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
                    let subst: HashMap<_, _> = composed
                        .params
                        .iter()
                        .flat_map(|p| p.keys())
                        .zip(&decl.provided_traits[i].args)
                        .map(|(name, arg)| (*name, arg.value.clone()))
                        .collect();
                    for component in &composed.provided_traits {
                        if seen.insert(component.trait_name) {
                            let mut component = component.clone();
                            for arg in &mut component.args {
                                let _ =
                                    ParameterSubstitution::new(&subst).visit_expr(&mut arg.value);
                            }
                            decl.provided_traits.push(component);
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
                        value: ast::Pattern::Nominal(name, span),
                        span_id: span,
                    }),
                    discriminant: None,
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
                    return offset;
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
            Self::collect_expr_type_surface_tags(&sp.value, tags);
        }
        if let Some(sp) = &bind.return_tag {
            Self::collect_expr_type_surface_tags(&sp.value, tags);
        }
    }

    fn collect_expr_type_surface_tags(expr: &ast::Expr, tags: &mut Vec<(Intern<String>, SpanId)>) {
        match expr {
            ast::Expr::AnonymousTag(name) => tags.push((*name, SpanId::INVALID)),
            ast::Expr::FnCall(call)
                if call.args.as_ref().is_none_or(Vec::is_empty)
                    && call.path.value.segments.is_empty() =>
            {
                tags.push((call.path.value.root, call.path.span_id));
            }
            ast::Expr::TagCall(call) => {
                for arg in &call.args {
                    Self::collect_expr_type_surface_tags(&arg.value, tags);
                }
            }
            ast::Expr::Ref { inner, .. } | ast::Expr::TakePtr(inner) => {
                Self::collect_expr_type_surface_tags(&inner.value, tags)
            }
            ast::Expr::ArrayLit(values) | ast::Expr::List(values) => {
                for value in values {
                    Self::collect_expr_type_surface_tags(&value.value, tags);
                }
            }
            _ => {}
        }
    }
}
