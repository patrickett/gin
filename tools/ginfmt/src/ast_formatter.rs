//! AST-based formatter for Gin source code.
//!
//! Walks the Gin `FileAst` and builds formatted output using AST node properties
//! instead of operating on raw tree-sitter nodes or source text slices.
//!
//! TODO(ginfmt): Order members in `has` bodies:
//! 1. Required/computed zero-input properties (no params, stored/computed fields)
//! 2. `Self.` associated members (static methods)
//! 3. Argument-taking instance methods (methods with params)
//!    Within each group, members should be sorted for stable output.

use ast::span::HasSpanId;
use ast::{
    AttributeItem, BindOperator, BindValue, BundleExportImport, Declare, DeclareValue, DocComment,
    Expr, FileAst, HasMember, HasMemberBody, HashFloat, ImportSource, Literal, ModuleImport,
    ParameterKind, Parameters, ProvidedTrait, SpanTable, Spanned, Typed, Variant,
};
use ast_format::type_expr::ExprFormatExt;
use internment::Intern;
use parser::query::SourceComment;

use crate::config::Config;

/// The AST-based formatter.
pub struct AstFormatter<'a> {
    source: &'a str,
    buffer: String,
    config: &'a Config,
    span_table: &'a SpanTable,
    indent_level: usize,
    comments: &'a [SourceComment],
    next_comment: usize,
    line_breaks: &'a [usize],
    blank_lines: &'a [usize],
    source_end: usize,
}

impl<'a> AstFormatter<'a> {
    pub fn new(source: &'a str, config: &'a Config, span_table: &'a SpanTable) -> Self {
        Self {
            source,
            buffer: String::with_capacity(source.len()),
            config,
            span_table,
            indent_level: 0,
            comments: &[],
            next_comment: 0,
            line_breaks: &[],
            blank_lines: &[],
            source_end: 0,
        }
    }

    pub fn with_comments(mut self, comments: &'a [SourceComment]) -> Self {
        self.comments = comments;
        self
    }

    fn emit_comments_before(&mut self, position: usize) {
        while let Some(comment) = self.comments.get(self.next_comment)
            && self.span_table.get(comment.span).start() < position
        {
            let span = self.span_table.get(comment.span);
            self.preserve_blank_line_before(span.start());
            self.buffer.push_str(&comment.text);
            self.buffer.push('\n');
            self.source_end = span.end();
            self.next_comment += 1;
        }
    }

    /// Format the entire file AST.
    pub fn format_file(&mut self, ast: &'a FileAst) -> String {
        self.line_breaks = &ast.line_breaks;
        self.blank_lines = &ast.blank_lines;
        self.format_file_inner(ast);
        space_rebind_operators(&std::mem::take(&mut self.buffer))
    }

    fn preserve_blank_line_before(&mut self, position: usize) {
        let index = self
            .blank_lines
            .partition_point(|position| *position < self.source_end);
        if !self.buffer.is_empty()
            && !self.buffer.ends_with("\n\n")
            && self
                .blank_lines
                .get(index)
                .is_some_and(|end| *end <= position)
        {
            self.buffer.push('\n');
        }
    }

    fn has_line_break(&self, start: usize, end: usize) -> bool {
        let index = self
            .line_breaks
            .partition_point(|position| *position < start);
        self.line_breaks
            .get(index)
            .is_some_and(|position| *position < end)
    }

    fn format_file_inner(&mut self, ast: &FileAst) {
        if let Some(doc) = &ast.module_doc {
            self.emit_module_doc(doc);
        }
        let has_uses = !ast.uses.is_empty();
        let has_tags = !ast.tags.is_empty();
        let has_defs = !ast.defs.is_empty();
        let has_exprs = !ast.exprs.is_empty();

        for import in &ast.uses {
            if let Some(module) = import.iter().next() {
                self.emit_comments_before(self.span_table.get(module.span_id()).start());
                self.preserve_blank_line_before(self.span_table.get(module.span_id()).start());
            }
            self.format_import(import);
            self.buffer.push('\n');
            self.source_end = import
                .iter()
                .map(|module| self.span_table.get(module.span_id()).end())
                .max()
                .unwrap_or(self.source_end);
        }
        if has_uses && (has_tags || has_defs || has_exprs) {
            self.buffer.push('\n');
        }
        enum Definition<'a> {
            Type(&'a Intern<String>, &'a Declare),
            Bind(&'a ast::Bind),
        }
        let mut definitions = ast
            .tags
            .iter()
            .map(|(name, declare)| {
                (
                    self.definition_start(declare.name_span, declare.doc_comment.as_ref()),
                    Definition::Type(name, declare),
                )
            })
            .chain(
                ast.defs
                    .iter()
                    .filter(|(_, bind)| !ast.method_binds.contains(bind))
                    .map(|(_, bind)| {
                        (
                            self.definition_start(bind.name_span, bind.doc_comment.as_ref()),
                            Definition::Bind(bind),
                        )
                    }),
            )
            .collect::<Vec<_>>();
        definitions.sort_by_key(|(position, _)| *position);
        let mut previous_bind = false;
        for (index, (position, definition)) in definitions.iter().enumerate() {
            let is_bind = matches!(definition, Definition::Bind(..));
            if index > 0 && (previous_bind || is_bind) {
                self.buffer.push('\n');
            }
            self.emit_comments_before(*position);
            self.preserve_blank_line_before(*position);
            match definition {
                Definition::Type(name, declare) => self.format_declare(name, declare),
                Definition::Bind(bind) => self.format_bind(bind),
            }
            self.source_end = match definition {
                Definition::Type(_, declare) => std::iter::once(*declare)
                    .chain(&declare.extensions)
                    .map(|declare| self.span_table.get(declare.span).end())
                    .max()
                    .unwrap_or(*position),
                Definition::Bind(bind) => match &bind.value {
                    BindValue::Expr(expr) => self.span_table.get(expr.span_id).end(),
                    BindValue::Body { exprs, ret } => exprs
                        .iter()
                        .map(|expr| self.span_table.get(expr.span_id).end())
                        .chain(std::iter::once(self.span_table.get(ret.span_id).end()))
                        .max()
                        .unwrap_or(*position),
                    BindValue::Unassigned => self.span_table.get(bind.name_span).end(),
                },
            };
            self.buffer.push('\n');
            previous_bind = is_bind;
        }
        if has_defs && has_exprs {
            self.buffer.push('\n');
        }
        for (_expr, span_id) in &ast.exprs {
            let span = self.span_table.get(*span_id);
            self.emit_comments_before(span.start());
            self.preserve_blank_line_before(span.start());
            self.buffer.push_str(span.extract(self.source).trim_end());
            self.buffer.push('\n');
            self.source_end = span.end();
        }
        self.emit_comments_before(usize::MAX);
    }

    fn definition_start(&self, name_span: ast::SpanId, doc: Option<&DocComment>) -> usize {
        let start = self.span_table.get(name_span).start();
        doc.and_then(|doc| doc.span)
            .map_or(start, |span| start.min(self.span_table.get(span).start()))
    }

    fn emit_module_doc(&mut self, doc: &DocComment) {
        for line in doc.value.lines() {
            if line.trim().is_empty() {
                self.buffer.push_str("--|\n");
            } else {
                self.buffer.push_str("--| ");
                self.buffer.push_str(line.trim());
                self.buffer.push('\n');
            }
        }
        self.buffer.push('\n');
    }

    fn format_declare(&mut self, name: &Intern<String>, declare: &Declare) {
        self.format_attributes(declare.attributes.raw_attributes.as_deref());
        if let Some(doc) = declare.doc_comment.as_ref() {
            self.emit_doc_comment(doc);
        }
        if let DeclareValue::Has(members) = &declare.value
            && members.iter().any(|member| match member {
                HasMember::Property(property) => {
                    property.body.is_some()
                        || property.doc_comment.is_some()
                        || property.attributes.raw_attributes.is_some()
                }
                HasMember::Function(function) => {
                    function.body.is_some()
                        || function.doc_comment.is_some()
                        || function.attributes.raw_attributes.is_some()
                }
            })
        {
            self.format_preserved_has(declare, members);
            for extension in &declare.extensions {
                self.buffer.push_str("\n\n");
                self.format_declare(name, extension);
            }
            return;
        }
        let header_start = self.buffer.len();
        self.buffer.push_str(name.as_str());
        if let Some(params) = &declare.params {
            self.format_params(params, None, declare.name_span, 4);
        }
        let st = self.span_table;
        let src = self.source;
        match &declare.value {
            DeclareValue::Union { variants } => {
                self.buffer.push_str(" is");
                self.format_union_variants(variants, declare.keyword_span);
            }
            DeclareValue::Has(members) => {
                self.buffer.push_str(" has");
                let property_padding = self.property_padding(members);
                let mut multiline_members = members.last().is_some_and(|member| {
                    let last = match member {
                        HasMember::Property(property) => property.name_span,
                        HasMember::Function(function) => function.name_span,
                    };
                    self.has_line_break(st.get(declare.keyword_span).end(), st.get(last).start())
                });
                let composed_traits: Vec<_> = declare
                    .provided_traits
                    .iter()
                    .filter(|provided| provided.fields.is_empty())
                    .collect();
                self.format_trait_chain(&composed_traits);
                let mut measure = Self::new(src, self.config, st);
                measure.line_breaks = self.line_breaks;
                for member in members {
                    measure.format_has_member(member, 1);
                    measure.buffer.push_str(", ");
                }
                let header_width = self
                    .buffer
                    .rsplit('\n')
                    .next()
                    .map_or(0, |line| line.chars().count());
                multiline_members |= self.buffer[header_start..].contains('\n')
                    || !composed_traits.is_empty()
                    || measure.buffer.contains('\n')
                    || header_width + 1 + measure.buffer.trim_end_matches(", ").chars().count()
                        > self.config.max_line_width;
                for (index, member) in members.iter().enumerate() {
                    if composed_traits.is_empty() && !multiline_members {
                        if index > 0 {
                            self.buffer.push(',');
                        }
                        self.buffer.push(' ');
                    } else {
                        self.buffer.push('\n');
                        if index > 0 {
                            let previous = match &members[index - 1] {
                                HasMember::Property(property) => property
                                    .ty
                                    .as_ref()
                                    .map_or(property.name_span, |ty| ty.span_id()),
                                HasMember::Function(function) => function
                                    .return_ty
                                    .as_ref()
                                    .map_or(function.name_span, |ty| ty.span_id()),
                            };
                            let current = match member {
                                HasMember::Property(property) => property.name_span,
                                HasMember::Function(function) => function.name_span,
                            };
                            let index = self
                                .blank_lines
                                .partition_point(|position| *position < st.get(previous).end());
                            if self
                                .blank_lines
                                .get(index)
                                .is_some_and(|end| *end <= st.get(current).start())
                            {
                                self.buffer.push('\n');
                            }
                        }
                        self.buffer.push_str("    ");
                    }
                    let padding = if multiline_members {
                        property_padding[index]
                    } else {
                        1
                    };
                    self.indent_level += usize::from(multiline_members);
                    self.format_has_member(member, padding);
                    self.indent_level -= usize::from(multiline_members);
                }
            }

            DeclareValue::Alias(target) => {
                self.buffer.push_str(" is ");
                if let Expr::IntegerDomain(inner) = &target.value {
                    self.buffer.push_str("in ");
                    self.buffer.push_str(&inner.value.format_surface());
                } else {
                    let span = st.get(target.span_id());
                    self.buffer.push_str(span.extract(src));
                }
            }
            DeclareValue::Set(..) => {
                self.buffer.push_str(" is set");
            }
            DeclareValue::Range(start, end) => {
                self.buffer.push_str(" is ");
                self.buffer.push_str(&format!("{start} to {end}"));
            }
            DeclareValue::InRange(start, end) => {
                self.buffer.push_str(" is in ");
                self.buffer.push_str(&format!("{start} to {end}"));
            }
            DeclareValue::Refinement(_) | DeclareValue::When(_) => {
                self.buffer.push_str(" is ");
                let span = st.get(declare.name_span);
                let end = src[span.start()..]
                    .find('\n')
                    .map(|p| span.start() + p)
                    .unwrap_or(src.len());
                self.buffer.push_str(&src[span.start()..end]);
            }
        }
        for extension in &declare.extensions {
            self.buffer.push_str("\n\n");
            self.format_declare(name, extension);
        }
    }

    fn format_has_member(&mut self, member: &HasMember, padding: usize) {
        let st = self.span_table;
        let src = self.source;
        match member {
            HasMember::Property(p) => {
                if let Some(qualifier) = &p.qualifier {
                    self.buffer.push_str(qualifier.name.as_str());
                    self.buffer.push('.');
                }
                self.buffer.push_str(p.name.as_str());
                if let Some(ty) = &p.ty {
                    self.buffer.extend(std::iter::repeat_n(' ', padding));
                    let span = st.get(ty.span_id());
                    self.buffer.push_str(span.extract(src));
                }
                if let Some(body) = &p.body {
                    match body {
                        HasMemberBody::Overrideable(_) => self.buffer.push_str(": ..."),
                        HasMemberBody::Final(_) => self.buffer.push_str(":= ..."),
                    }
                }
            }
            HasMember::Function(f) => {
                if let Some(qualifier) = &f.qualifier {
                    self.buffer.push_str(qualifier.name.as_str());
                    self.buffer.push('.');
                }
                self.buffer.push_str(f.name.as_str());
                let suffix_start = self.buffer.len();
                for (index, alternative) in f.anonymous_result_alternatives.iter().enumerate() {
                    self.buffer.push_str(if index == 0 { " " } else { " or " });
                    self.buffer.push_str(alternative.value.label.as_str());
                    if let Some(proposition) = &alternative.value.proposition {
                        self.buffer.push_str(&format!(" thus {proposition}"));
                    }
                }
                if let Some(rt) = &f.return_ty {
                    self.buffer.push(' ');
                    if f.payload_postconditions.is_empty() {
                        let span = st.get(rt.span_id());
                        self.buffer.push_str(span.extract(src));
                    } else {
                        self.buffer
                            .push_str(&ast_format::declare::payload_result_surface(
                                &rt.value,
                                &f.payload_postconditions,
                            ));
                    }
                }
                if let Some(body) = &f.body {
                    match body {
                        HasMemberBody::Overrideable(_) => self.buffer.push_str(": ..."),
                        HasMemberBody::Final(_) => self.buffer.push_str(":= ..."),
                    }
                }
                let suffix = self.buffer.split_off(suffix_start);
                self.format_params(
                    &f.params,
                    Some(&f.conventions),
                    f.name_span,
                    suffix.chars().count(),
                );
                self.buffer.push_str(&suffix);
            }
        }
    }

    fn property_padding(&self, members: &[HasMember]) -> Vec<usize> {
        let mut padding = vec![1; members.len()];
        let mut group = Vec::<(usize, usize, usize)>::new();
        let mut previous_type_end = None;
        let mut previous_indent = None;
        let finish = |group: &mut Vec<(usize, usize, usize)>, padding: &mut Vec<usize>| {
            let width = group.iter().map(|(_, width, _)| *width).max().unwrap_or(0);
            for (index, name_width, _) in group.drain(..) {
                padding[index] = width - name_width + 1;
            }
        };
        for (index, member) in members.iter().enumerate() {
            let HasMember::Property(property) = member else {
                finish(&mut group, &mut padding);
                continue;
            };
            let Some(ty) = &property.ty else {
                finish(&mut group, &mut padding);
                continue;
            };
            if property.qualifier.is_some()
                || property.refinement.is_some()
                || property.body.is_some()
            {
                finish(&mut group, &mut padding);
                continue;
            }
            let type_span = self.span_table.get(ty.span_id());
            let name_start = self.span_table.get(property.name_span).start();
            let line_start = self.source[..name_start]
                .rfind('\n')
                .map_or(0, |end| end + 1);
            let indent = self.source[line_start..name_start]
                .chars()
                .take_while(|character| matches!(character, ' ' | '\t'))
                .map(|character| if character == '\t' { 4 } else { 1 })
                .sum::<usize>()
                .max(4);
            if previous_indent.is_some_and(|previous| previous != indent) {
                finish(&mut group, &mut padding);
            }
            previous_indent = Some(indent);
            if previous_type_end.is_some_and(|end| {
                let gap = &self.source[end..name_start];
                gap.contains('\n')
                    && (gap.bytes().filter(|byte| *byte == b'\n').count() != 1
                        || !gap.chars().all(char::is_whitespace))
            }) {
                finish(&mut group, &mut padding);
            }
            previous_type_end = Some(type_span.end());
            if self.has_line_break(type_span.start(), type_span.end()) {
                finish(&mut group, &mut padding);
                continue;
            }
            let name_width = property.name.chars().count();
            let type_width = span_text_spanned_expr(ty, self.span_table, self.source)
                .chars()
                .count();
            let width = group
                .iter()
                .map(|(_, width, _)| *width)
                .max()
                .unwrap_or(0)
                .max(name_width);
            if group
                .iter()
                .copied()
                .chain(std::iter::once((index, name_width, type_width)))
                .any(|(_, name, ty)| {
                    width - name > self.config.max_type_alignment_padding
                        || indent + width + 1 + ty > self.config.max_line_width
                })
            {
                finish(&mut group, &mut padding);
            }
            group.push((index, name_width, type_width));
        }
        finish(&mut group, &mut padding);
        padding
    }

    fn format_trait_chain(&mut self, traits: &[&ProvidedTrait]) {
        let mut width = self
            .buffer
            .rsplit('\n')
            .next()
            .map_or(0, |line| line.chars().count());
        let indent = width.saturating_sub(3);
        for (index, provided) in traits.iter().enumerate() {
            let mut rendered = provided.trait_name.to_string();
            if !provided.args.is_empty() {
                rendered.push('(');
                rendered.push_str(
                    &provided
                        .args
                        .iter()
                        .map(|a| a.value.format_surface())
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                rendered.push(')');
            }
            let separator = if index == 0 { " " } else { " and " };
            let next_width = separator.len() + rendered.chars().count();
            if index > 0 && width + next_width > self.config.max_line_width {
                self.buffer.push('\n');
                self.buffer.extend(std::iter::repeat_n(' ', indent));
                self.buffer.push_str("and ");
                width = indent + 4 + rendered.chars().count();
            } else {
                self.buffer.push_str(separator);
                width += next_width;
            }
            self.buffer.push_str(&rendered);
        }
    }

    fn format_preserved_has(&mut self, declare: &Declare, members: &[HasMember]) {
        let span = self.span_table.get(declare.span);
        let mut position = span.start();
        if let Some(traits_span) = declare.provided_traits_span
            && declare
                .provided_traits
                .iter()
                .all(|provided| provided.fields.is_empty())
        {
            let traits_span = self.span_table.get(traits_span);
            self.buffer
                .push_str(self.source[position..traits_span.start()].trim_end());
            let traits = declare.provided_traits.iter().collect::<Vec<_>>();
            self.format_trait_chain(&traits);
            position = traits_span.end();
        }

        for (member, padding) in members.iter().zip(self.property_padding(members)) {
            let HasMember::Property(property) = member else {
                continue;
            };
            let Some(ty) = &property.ty else { continue };
            if property.qualifier.is_some()
                || property.refinement.is_some()
                || property.body.is_some()
            {
                continue;
            }
            let start = self.span_table.get(property.name_span).end();
            let end = self.span_table.get(ty.span_id()).start();
            if start < end
                && self.source[start..end]
                    .chars()
                    .all(|ch| ch == ' ' || ch == '\t')
            {
                self.buffer.push_str(&self.source[position..start]);
                self.buffer.extend(std::iter::repeat_n(' ', padding));
                position = end;
            }
        }
        self.buffer
            .push_str(self.source[position..span.end()].trim_end());
    }

    fn format_params(
        &mut self,
        params: &Parameters,
        conventions: Option<&indexmap::IndexMap<Intern<String>, ast::ParamConvention>>,
        name_span: ast::SpanId,
        suffix_width: usize,
    ) {
        let st = self.span_table;
        let src = self.source;
        let rendered: Vec<String> = params
            .iter()
            .map(|(name, param)| {
                if let Some(surface) = self.parameter_domain_surface(param.name_span) {
                    return surface;
                }
                if let Some(pattern) = &param.range_pattern {
                    return format!("{} to {}", pattern.start, pattern.end);
                }
                let mut rendered = name.as_str().trim().to_string();
                let convention = conventions.and_then(|map| map.get(name));
                let type_has_convention = match (&param.kind, convention) {
                    (
                        ParameterKind::Tagged(ty),
                        Some(ast::ParamConvention::Observe | ast::ParamConvention::Mutate),
                    ) => matches!(ty.value, Expr::Ref { .. }),
                    _ => false,
                };
                if let Some(convention) = convention.filter(|_| !type_has_convention) {
                    match convention {
                        ast::ParamConvention::Terminal => {}
                        ast::ParamConvention::Own => rendered.push_str(" own"),
                        ast::ParamConvention::Observe => rendered.push_str(" ref"),
                        ast::ParamConvention::Mutate => rendered.push_str(" mut"),
                    }
                }
                match &param.kind {
                    ParameterKind::Generic => {}
                    ParameterKind::Proposition(predicate) => {
                        rendered.push_str(&format!(" is {predicate}"))
                    }
                    ParameterKind::Tagged(sp) | ParameterKind::ValueParam { ty: sp } => {
                        rendered.push(' ');
                        rendered.push_str(&sp.value.format_surface());
                    }
                    ParameterKind::Inferred { ty } => {
                        rendered.push(' ');
                        rendered.push_str(&ty.value.format_surface());
                        rendered.push_str(": ?");
                    }
                    ParameterKind::Default(expr) => {
                        rendered.push_str(": ");
                        rendered.push_str(&span_text(expr, st, src));
                    }
                }
                if !param.constraints.is_empty() {
                    rendered.push_str(" has ");
                    rendered.push_str(&ast_format::declare::format_parameter_constraints(param));
                }
                rendered.trim().to_string()
            })
            .collect();
        let inline = rendered.join(", ");
        let inline_len = self
            .buffer
            .rsplit('\n')
            .next()
            .map_or(0, |line| line.chars().count())
            + 2
            + inline.chars().count()
            + suffix_width;
        let explicit_multiline = params.values().last().is_some_and(|param| {
            self.has_line_break(st.get(name_span).end(), st.get(param.name_span).start())
        });
        self.buffer.push('(');
        if !rendered.is_empty() && (explicit_multiline || inline_len > self.config.max_line_width) {
            for item in rendered {
                self.buffer.push('\n');
                self.emit_indent();
                self.buffer.push_str("    ");
                self.buffer.push_str(&item);
            }
            self.buffer.push('\n');
            self.emit_indent();
        } else {
            self.buffer.push_str(&inline);
        }
        self.buffer.push(')');
    }

    fn parameter_domain_surface(&self, name_span: ast::SpanId) -> Option<String> {
        let start = self.span_table.get(name_span).start();
        let source = &self.source[start..];
        let mut lexer = lexer::Lexer::new(source);
        let mut depth = 0;
        let mut end = 0;
        let mut domain = false;
        while let Some((token, span)) = lexer.next() {
            if depth == 0
                && matches!(
                    token,
                    lexer::Token::Comma | lexer::Token::ParenClose | lexer::Token::Newline
                )
            {
                break;
            }
            domain |= depth == 0 && matches!(token, lexer::Token::Is | lexer::Token::In);
            match token {
                lexer::Token::ParenOpen | lexer::Token::BracketOpen | lexer::Token::CurlyOpen => {
                    depth += 1
                }
                lexer::Token::ParenClose
                | lexer::Token::BracketClose
                | lexer::Token::CurlyClose => depth -= 1,
                _ => {}
            }
            end = lexer.get_span(span).end();
        }
        domain.then(|| source[..end].trim().to_owned())
    }

    fn format_union_variants(&mut self, variants: &[Variant], keyword_span: ast::SpanId) {
        let Some(first) = variants.first() else {
            return;
        };
        let first_span = self.span_table.get(first.shape().span_id());
        let first_on_new_line =
            self.has_line_break(self.span_table.get(keyword_span).end(), first_span.start());
        let multiline = first_on_new_line
            || variants.last().is_some_and(|last| {
                self.has_line_break(
                    first_span.start(),
                    self.span_table.get(last.shape().span_id()).start(),
                )
            });
        for (i, variant) in variants.iter().enumerate() {
            if (i == 0 && first_on_new_line) || (i > 0 && multiline) {
                self.buffer.push_str("\n    ");
            } else {
                self.buffer.push(' ');
            }
            if i > 0 {
                self.buffer.push_str("or ");
            }
            self.buffer
                .push_str(&ast_format::declare::format_variant(variant));
        }
    }

    fn format_import(&mut self, import: &ast::Import) {
        if import.0.is_empty() {
            return;
        }

        let mut sorted: Vec<&ModuleImport> = import.0.iter().collect();
        sorted.sort_by_key(|mi| import_display_key(mi));

        self.buffer.push_str("use ");
        for (i, mi) in sorted.iter().enumerate() {
            if i > 0 {
                if sorted
                    .iter()
                    .all(|m| matches!(m.source, ImportSource::CurrentModule { .. }))
                {
                    self.buffer.push_str(", ");
                } else {
                    self.buffer.push_str("\nuse ");
                }
            }
            match &mi.source {
                ImportSource::Package(path) => {
                    self.buffer.push_str(path.root.as_str());
                    for seg in &path.segments {
                        self.buffer.push('.');
                        self.buffer.push_str(seg.as_str());
                    }
                }
                ImportSource::Local(p, _) | ImportSource::LocalFile(p, _) => {
                    self.buffer.push('\'');
                    self.buffer.push_str(&p.to_string_lossy());
                    self.buffer.push('\'');
                }
                ImportSource::LocalBundle(b) => {
                    if let Some(path) = &b.local_path {
                        self.buffer.push('\'');
                        self.buffer.push_str(&path.to_string_lossy());
                        self.buffer.push('\'');
                    } else {
                        self.buffer.push_str(b.root.as_str());
                    }
                    for seg in &b.path_segments {
                        self.buffer.push('.');
                        self.buffer.push_str(seg.as_str());
                    }
                    let mut members: Vec<&BundleExportImport> = b.members.iter().collect();
                    members.sort_by_key(|m| m.export.as_str().to_lowercase());
                    let width: usize = members
                        .iter()
                        .map(|m| m.export.len() + m.alias.as_ref().map_or(0, |a| 4 + a.len()))
                        .sum::<usize>()
                        + members.len().saturating_sub(1) * 2;
                    let mut line_width = self.buffer.rsplit('\n').next().map_or(0, str::len);
                    let multiline = line_width + 1 + width > self.config.max_line_width;
                    for (j, member) in members.iter().enumerate() {
                        let member_width =
                            member.export.len() + member.alias.as_ref().map_or(0, |a| 4 + a.len());
                        let trailing_comma = usize::from(j + 1 < members.len());
                        if multiline && j == 0 {
                            self.buffer.push_str("\n    ");
                            line_width = 4;
                        } else if multiline
                            && line_width + 2 + member_width + trailing_comma
                                > self.config.max_line_width
                        {
                            self.buffer.push_str(",\n    ");
                            line_width = 4;
                        } else if j == 0 {
                            self.buffer.push(' ');
                        } else {
                            self.buffer.push_str(", ");
                            line_width += 2;
                        }
                        line_width += member_width;
                        self.buffer.push_str(member.export.as_str());
                        if let Some(alias) = &member.alias {
                            self.buffer.push_str(" as ");
                            self.buffer.push_str(alias.as_str());
                        }
                    }
                }
                ImportSource::CurrentModule { member } => {
                    self.buffer.push_str(member.export.as_str());
                    if let Some(alias) = &member.alias {
                        self.buffer.push_str(" as ");
                        self.buffer.push_str(alias.as_str());
                    }
                }
            }
            if let Some(alias) = &mi.alias {
                self.buffer.push_str(" as ");
                self.buffer.push_str(alias.as_str());
            }
        }
    }

    fn format_bind(&mut self, bind: &ast::Bind) {
        self.format_attributes(bind.attributes.raw_attributes.as_deref());
        if let Some(doc) = bind.doc_comment.as_ref() {
            self.emit_doc_comment(doc);
        }
        if let Some(receiver) = bind.receiver_type_surface() {
            self.buffer.push_str(&span_text_spanned_expr(
                receiver,
                self.span_table,
                self.source,
            ));
            self.buffer.push('.');
        }
        self.buffer.push_str(bind.name.as_str());
        let st = self.span_table;
        let src = self.source;
        let value = match &bind.value {
            BindValue::Expr(expr) => Some(expr.as_ref()),
            _ => None,
        };
        let redundant_type = match (bind.return_tag.as_deref(), value) {
            (Some(tag), Some(value)) => match (&tag.value, &value.value) {
                (Expr::AnonymousTag(name), Expr::TagCall(call)) => {
                    call.qual_path.is_none() && *name == call.name
                }
                _ => false,
            },
            _ => false,
        };
        let suffix_start = self.buffer.len();
        if !bind.named_results.is_empty() {
            self.buffer.push(' ');
            for (index, result) in bind.named_results.iter().enumerate() {
                if index > 0 {
                    self.buffer.push_str(", ");
                }
                if let Some(name) = result.name {
                    self.buffer.push_str(name.as_str());
                    self.buffer.push(' ');
                }
                self.buffer
                    .push_str(&span_text_spanned_expr(&result.ty, st, src));
            }
        } else if let Some((name, args)) = &bind.type_annotation {
            self.buffer.push(' ');
            if let Some(path) = &bind.type_annotation_qual {
                self.buffer.push_str(&path.value.to_qualified_string());
            } else {
                self.buffer.push_str(name.as_str());
            }
            self.buffer.push('(');
            self.buffer.push_str(
                &args
                    .iter()
                    .map(|arg| span_text(arg, st, src).trim().to_owned())
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            self.buffer.push(')');
        } else if let Some(ret_tag) = &bind.return_tag
            && !redundant_type
        {
            let text = span_text_spanned_expr(ret_tag, st, src);
            self.buffer.push(' ');
            self.buffer.push_str(&text);
        }
        let has_colon = !matches!(&bind.value, BindValue::Unassigned);
        if has_colon {
            match &bind.operator {
                BindOperator::FreshMutable => self.buffer.push_str(": "),
                BindOperator::FreshImmutable => self.buffer.push_str(" := "),
                BindOperator::Rebind => self.buffer.push_str(" :: "),
                BindOperator::Compound(op) => {
                    self.buffer.push_str(&format!("{}= ", op.symbol()));
                }
            }
        }

        let suffix = self.buffer.split_off(suffix_start);
        if let Some(params) = &bind.params
            && !params.is_empty()
        {
            self.format_params(
                params,
                Some(&bind.param_conventions),
                bind.name_span,
                suffix.trim_end().chars().count(),
            );
        }
        self.buffer.push_str(&suffix);

        if let Some(expr) = value {
            if let Expr::Block(items) = &expr.value {
                while self.buffer.ends_with(' ') {
                    self.buffer.pop();
                }
                self.indent_level += 1;
                for item in items {
                    self.buffer.push('\n');
                    self.emit_indent();
                    self.buffer.push_str(&span_text(item, st, src));
                }
                self.indent_level -= 1;
            } else {
                self.buffer.push_str(&span_text(expr, st, src));
            }
            return;
        }

        if let BindValue::Body { exprs, ret } = &bind.value {
            let start = st.get(ret.span_id).start();
            let prefix = src[..start.min(src.len())]
                .rsplit('\n')
                .next()
                .unwrap_or_default()
                .trim_end();
            let inline = prefix.ends_with(':') || prefix.ends_with(":=");
            let multiline_when = ret.values.iter().any(|value| {
                matches!(value.value, Expr::When(_)) && span_text(value, st, src).contains('\n')
            });
            let compact = ret.values.len() == 1 && !multiline_when;
            if exprs.is_empty() && ret.postcondition.is_none() && (compact || inline) {
                self.buffer.push_str("return");
                if let Some(value) = ret.values.first() {
                    self.buffer.push(' ');
                    self.buffer.push_str(&span_text(value, st, src));
                }
                return;
            }
        }

        if let BindValue::Body { exprs, ret } = &bind.value {
            while self.buffer.ends_with(' ') {
                self.buffer.pop();
            }
            self.buffer.push('\n');
            self.indent_level += 1;
            for e in exprs {
                self.emit_indent();
                let text = span_text(e, st, src);
                self.buffer.push_str(&text);
                self.buffer.push('\n');
            }
            if !ret.values.is_empty() {
                let text = ret
                    .values
                    .iter()
                    .map(|ret_expr| span_text(ret_expr, st, src))
                    .collect::<Vec<_>>()
                    .join(", ");
                self.emit_indent();
                self.buffer.push_str("return ");
                self.buffer.push_str(&text);
            } else {
                self.emit_indent();
                self.buffer.push_str("return");
            }
            if let Some(proof) = &ret.postcondition {
                self.buffer.push_str(&format!(" thus {}", proof.value));
            }
            self.indent_level -= 1;
        }
    }

    fn format_attributes(&mut self, items: Option<&[AttributeItem]>) {
        let Some(items) = items else {
            return;
        };

        let rendered: Vec<String> = items
            .iter()
            .map(|item| self.format_attribute(item))
            .collect();
        if rendered.is_empty() {
            return;
        }

        let inline = rendered.join(", ");
        if inline.len() <= self.config.max_line_width {
            self.buffer.push_str(&inline);
            self.buffer.push('\n');
        } else {
            for attribute in rendered {
                self.buffer.push_str(&attribute);
                self.buffer.push('\n');
            }
        }
    }

    fn format_attribute(&self, attribute: &AttributeItem) -> String {
        match attribute {
            AttributeItem::Flag { name, .. } => format!("#{}", name.as_str()),
            AttributeItem::Call { name, args, .. } => {
                let args = args
                    .iter()
                    .map(|arg| span_text(arg, self.span_table, self.source))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("#{}({args})", name.as_str())
            }
        }
    }

    fn emit_doc_comment(&mut self, doc: &DocComment) {
        for line in doc.value.lines() {
            if line.trim().is_empty() {
                self.buffer.push_str("---\n");
            } else {
                let trimmed = line.trim_start();
                self.buffer.push_str("--- ");
                self.buffer.push_str(trimmed);
                self.buffer.push('\n');
            }
        }
    }

    fn emit_indent(&mut self) {
        for _ in 0..self.indent_level {
            self.buffer.push_str("    ");
        }
    }
}

/// Return a case-insensitive sort key for a module import.
pub fn import_display_key(mi: &ModuleImport) -> String {
    match &mi.source {
        ImportSource::Package(path) => {
            let mut s = path.root.as_str().to_lowercase();
            for seg in &path.segments {
                s.push('.');
                s.push_str(seg.as_str().to_lowercase().as_str());
            }
            s
        }
        ImportSource::Local(p, _) => p.to_string_lossy().to_lowercase(),
        ImportSource::LocalFile(p, _) => p.to_string_lossy().to_lowercase(),
        ImportSource::LocalBundle(b) => b.local_path.as_ref().map_or_else(
            || b.qualifier_prefix().to_lowercase(),
            |path| path.to_string_lossy().to_lowercase(),
        ),
        ImportSource::CurrentModule { member } => {
            let mut s = member.export.as_str().to_lowercase();
            if let Some(alias) = &member.alias {
                s.push_str(" as ");
                s.push_str(alias.as_str().to_lowercase().as_str());
            }
            s
        }
    }
}

/// Get the source text for a spanned expression using the span table.
fn span_text(expr: &Typed<Expr>, st: &SpanTable, source: &str) -> String {
    let span = st.get(expr.span_id);
    if !span.is_empty() {
        span.extract(source).trim().to_string()
    } else {
        expr_fallback(&expr.value, st, source)
    }
}

fn span_text_spanned_expr(expr: &Spanned<Expr>, st: &SpanTable, source: &str) -> String {
    if matches!(
        expr.value,
        Expr::IntegerDomain(_) | Expr::AnonymousTag(_) | Expr::TagCall(_) | Expr::FnCall(_)
    ) {
        return expr.value.format_surface();
    }
    let span = st.get(expr.span_id());
    if !span.is_empty() {
        span.extract(source).trim_end().to_string()
    } else {
        expr.value.format_surface()
    }
}

/// Fallback expression formatting when span is zero-length.
fn expr_fallback(expr: &Expr, _st: &SpanTable, _source: &str) -> String {
    match expr {
        Expr::Lit(lit) => match lit {
            Literal::Int(n) => n.to_string(),
            Literal::Float(HashFloat(f)) => f.to_string(),
            Literal::String(s) => format!("'{}'", s),
        },
        Expr::AnonymousTag(name) => name.as_str().to_string(),
        Expr::SelfRef => "self".to_string(),
        _ => String::new(),
    }
}

fn space_rebind_operators(source: &str) -> String {
    let mut lexer = lexer::Lexer::new(source);
    let mut formatted = String::with_capacity(source.len());
    let mut position = 0;
    while let Some((token, span)) = lexer.next() {
        if token != lexer::Token::ColonColon {
            continue;
        }
        let span = lexer.get_span(span);
        let start = source[..span.start()].trim_end_matches([' ', '\t']).len();
        let rest = source[span.end()..].trim_start_matches([' ', '\t']);
        let end = source.len() - rest.len();
        formatted.push_str(&source[position..start]);
        if start > 0 && !source[..start].ends_with(['\n', '\r']) {
            formatted.push(' ');
        }
        formatted.push_str("::");
        if !rest.is_empty() && !rest.starts_with(['\n', '\r']) {
            formatted.push(' ');
        }
        position = end;
    }
    formatted.push_str(&source[position..]);
    formatted
}

#[cfg(test)]
#[path = "tests/ast_formatter_tests.rs"]
mod tests;
