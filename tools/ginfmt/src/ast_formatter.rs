//! AST-based formatter for Gin source code.
//!
//! Walks the Gin `FileAst` and builds formatted output using AST node properties
//! instead of operating on raw tree-sitter nodes or source text slices.
//!
//! # Future: structural multiline layout
//!
//! The parser now accepts newline-separated lists in many contexts:
//! function params, type params, interface members, call args, list literals,
//! tuple lits, and patterns. The formatter currently always emits compact
//! comma-separated inline lists and relies on `wrap_lines` post-processing
//! for line-length conformance.
//!
//! TODO: Structural multiline formatting — when an inline list exceeds
//! `max_line_width`, emit newline-separated layout instead:
//! ```gin
//! // instead of:
//! fn(a Int, b Int, c Int, d Int) Int := a + b + c + d
//! // emit:
//! fn(
//!     a Int
//!     b Int
//!     c Int
//!     d Int
//! ) Int := a + b + c + d
//! ```
//! This requires:
//! - Pre-measuring list width before emitting
//! - Choosing between inline (comma) vs block (newline) layout per list
//! - Tracking indentation depth across multiline delimiter pairs
//!
//! TODO(ginfmt): Order members in `has` bodies:
//! 1. Required/computed zero-input properties (no params, stored/computed fields)
//! 2. `Self.` associated members (static methods)
//! 3. Argument-taking instance methods (methods with params)
//!    Within each group, members should be sorted for stable output.

use ast::{
    BindValue, BundleExportImport, Declare, DeclareValue, DocComment, Expr, FileAst, HasMember,
    HasMemberBody, HashFloat, ImportSource, Literal, ModuleImport, ParameterKind, Parameters,
    SpanTable, Spanned, Typed, Variant,
};
use ast_format::type_expr::{ExprFormatExt, PatternFormatExt};
use internment::Intern;

use crate::config::Config;

/// The AST-based formatter.
pub struct AstFormatter<'a> {
    source: &'a str,
    buffer: String,
    config: &'a Config,
    span_table: &'a SpanTable,
    indent_level: usize,
}

impl<'a> AstFormatter<'a> {
    pub fn new(source: &'a str, config: &'a Config, span_table: &'a SpanTable) -> Self {
        Self {
            source,
            buffer: String::with_capacity(source.len()),
            config,
            span_table,
            indent_level: 0,
        }
    }

    /// Format the entire file AST.
    pub fn format_file(&mut self, ast: &FileAst) -> String {
        self.format_file_inner(ast);
        let result = std::mem::take(&mut self.buffer);
        wrap_lines(&result, self.config.max_line_width)
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
            self.format_import(import);
            self.buffer.push('\n');
        }
        if has_uses && (has_tags || has_defs || has_exprs) {
            self.buffer.push('\n');
        }
        // Sort tags by source position for deterministic output
        let mut tag_order: Vec<(usize, &Intern<String>, &Declare)> = ast
            .tags
            .iter()
            .map(|(n, d)| {
                let span = self.span_table.get(d.name_span);
                (span.start(), n, d)
            })
            .collect();
        tag_order.sort_by_key(|&(pos, _, _)| pos);
        for (_, name, declare) in &tag_order {
            self.format_declare(name, declare);
            self.buffer.push('\n');
        }
        if has_tags && (has_defs || has_exprs) {
            self.buffer.push('\n');
        }
        // Sort defs by name_span for deterministic output
        let mut def_order: Vec<(usize, &Intern<String>, &ast::Bind)> = ast
            .defs
            .iter()
            .filter(|(_, bind)| !ast.method_binds.contains(bind))
            .map(|(n, b)| {
                let span = self.span_table.get(b.name_span);
                (span.start(), n, b)
            })
            .collect();
        def_order.sort_by_key(|&(pos, _, _)| pos);
        for (_, name, bind) in &def_order {
            self.format_bind(name, bind);
            self.buffer.push('\n');
        }
        if has_defs && has_exprs {
            self.buffer.push('\n');
        }
        for (_expr, span_id) in &ast.exprs {
            let span = self.span_table.get(*span_id);
            self.buffer.push_str(span.extract(self.source));
            self.buffer.push('\n');
        }
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
        if let Some(doc) = declare.doc_comment.as_ref() {
            self.emit_doc_comment(doc);
        }
        self.buffer.push_str(name.as_str());
        if let Some(params) = &declare.params {
            self.format_params(params);
        }
        let st = self.span_table;
        let src = self.source;
        match &declare.value {
            DeclareValue::Union { variants } => {
                self.buffer.push_str(" is ");
                self.format_union_variants(variants);
            }
            DeclareValue::Has(members) => {
                self.buffer.push_str(" has");
                let composed_traits: Vec<_> = declare
                    .provided_traits
                    .iter()
                    .filter(|provided| provided.fields.is_empty())
                    .collect();
                for (index, provided) in composed_traits.iter().enumerate() {
                    if index > 0 {
                        self.buffer.push_str(" and");
                    }
                    self.buffer.push(' ');
                    self.buffer.push_str(provided.trait_name.as_str());
                }
                for (index, member) in members.iter().enumerate() {
                    if composed_traits.is_empty() {
                        if index > 0 {
                            self.buffer.push(',');
                        }
                        self.buffer.push(' ');
                    } else {
                        self.buffer.push('\n');
                        self.buffer.push_str("    ");
                    }
                    match member {
                        HasMember::Property(p) => {
                            self.buffer.push_str(p.name.as_str());
                            if let Some(ty) = &p.ty {
                                self.buffer.push(' ');
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
                            self.format_params(&f.params);
                            if let Some(rt) = &f.return_ty {
                                self.buffer.push(' ');
                                let span = st.get(rt.span_id());
                                self.buffer.push_str(span.extract(src));
                            }
                            if let Some(et) = &f.error_ty {
                                self.buffer.push_str(" or ");
                                let span = st.get(et.span_id());
                                self.buffer.push_str(span.extract(src));
                            }
                            if let Some(body) = &f.body {
                                match body {
                                    HasMemberBody::Overrideable(_) => self.buffer.push_str(": ..."),
                                    HasMemberBody::Final(_) => self.buffer.push_str(":= ..."),
                                }
                            }
                        }
                    }
                }
            }

            DeclareValue::Alias(target) => {
                self.buffer.push_str(" is ");
                let span = st.get(target.span_id());
                self.buffer.push_str(span.extract(src));
            }
            DeclareValue::Set(..) => {
                self.buffer.push_str(" is set");
            }
            DeclareValue::Range(_, _)
            | DeclareValue::InRange(_, _)
            | DeclareValue::Refinement(_)
            | DeclareValue::When(_) => {
                self.buffer.push_str(" is ");
                let span = st.get(declare.name_span);
                let end = src[span.start()..]
                    .find('\n')
                    .map(|p| span.start() + p)
                    .unwrap_or(src.len());
                self.buffer.push_str(&src[span.start()..end]);
            }
        }
    }

    fn format_params(&mut self, params: &Parameters) {
        self.buffer.push('(');
        let st = self.span_table;
        let src = self.source;
        for (i, (name, param)) in params.iter().enumerate() {
            if i > 0 {
                self.buffer.push_str(", ");
            }
            self.buffer.push_str(name.as_str());
            match &param.kind {
                ParameterKind::Generic => {}
                ParameterKind::Tagged(sp) | ParameterKind::ValueParam { ty: sp } => {
                    self.buffer.push(' ');
                    self.buffer.push_str(&sp.value.format_surface());
                }
                ParameterKind::Inferred { ty } => {
                    self.buffer.push(' ');
                    self.buffer.push_str(&ty.value.format_surface());
                    self.buffer.push_str(": ?");
                }
                ParameterKind::Default(expr) => {
                    let text = span_text(expr, st, src);
                    self.buffer.push_str(": ");
                    self.buffer.push_str(&text);
                }
            }
        }
        self.buffer.push(')');
    }

    fn format_union_variants(&mut self, variants: &[Variant]) {
        for (i, variant) in variants.iter().enumerate() {
            if i > 0 {
                self.buffer.push_str(" or ");
            }
            let shape = variant.shape();
            self.buffer.push_str(&shape.value.format_variant_shape());
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
                self.buffer.push_str(", ");
            }
            match &mi.source {
                ImportSource::Package(path) => {
                    self.buffer.push_str(path.root.as_str());
                    for seg in &path.segments {
                        self.buffer.push('.');
                        self.buffer.push_str(seg.as_str());
                    }
                }
                ImportSource::Local(p, _) => {
                    self.buffer.push('\'');
                    self.buffer.push_str(&p.to_string_lossy());
                    self.buffer.push('\'');
                }
                ImportSource::LocalBundle(b) => {
                    self.buffer.push_str(b.root.as_str());
                    for seg in &b.path_segments {
                        self.buffer.push('.');
                        self.buffer.push_str(seg.as_str());
                    }
                    self.buffer.push_str(".(");
                    let mut members: Vec<&BundleExportImport> = b.members.iter().collect();
                    members.sort_by_key(|m| m.export.as_str().to_lowercase());
                    for (j, member) in members.iter().enumerate() {
                        if j > 0 {
                            self.buffer.push_str(", ");
                        }
                        self.buffer.push_str(member.export.as_str());
                        if let Some(alias) = &member.alias {
                            self.buffer.push_str(" as ");
                            self.buffer.push_str(alias.as_str());
                        }
                    }
                    self.buffer.push(')');
                }
                ImportSource::CurrentModule { member } => {
                    self.buffer.push_str(member.export.as_str());
                    if let Some(alias) = &member.alias {
                        self.buffer.push_str(" as ");
                        self.buffer.push_str(alias.as_str());
                    }
                }
                ImportSource::LocalMember(m) => {
                    if let Some(p) = &m.local_path {
                        self.buffer.push('\'');
                        self.buffer.push_str(&p.to_string_lossy());
                        self.buffer.push('\'');
                        self.buffer.push('.');
                    }
                    self.buffer.push_str(m.member.export.as_str());
                    if let Some(alias) = &m.member.alias {
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

    fn format_bind(&mut self, name: &Intern<String>, bind: &ast::Bind) {
        let _ = name;
        if let Some(doc) = bind.doc_comment.as_ref() {
            self.emit_doc_comment(doc);
        }
        self.buffer.push_str(bind.name.as_str());
        if let Some(params) = &bind.params {
            self.format_params(params);
        }
        let st = self.span_table;
        let src = self.source;
        if let Some(ret_tag) = &bind.return_tag {
            let text = span_text_spanned_expr(ret_tag, st, src);
            self.buffer.push(' ');
            self.buffer.push_str(&text);
        }
        let has_colon = !matches!(&bind.value, BindValue::Unassigned);
        if has_colon {
            self.buffer.push_str(": ");
        }

        match &bind.value {
            BindValue::Expr(expr) => {
                let text = span_text(expr, st, src);
                self.buffer.push_str(&text);
            }
            BindValue::Body { exprs, ret } => {
                self.buffer.push('\n');
                self.indent_level += 1;
                for e in exprs {
                    self.emit_indent();
                    let text = span_text(e, st, src);
                    self.buffer.push_str(&text);
                    self.buffer.push('\n');
                }
                if let Some(ret_expr) = &ret.value {
                    let text = span_text(ret_expr, st, src);
                    self.emit_indent();
                    self.buffer.push_str("return ");
                    self.buffer.push_str(&text);
                    self.buffer.push('\n');
                } else {
                    self.emit_indent();
                    self.buffer.push_str("return\n");
                }
                self.indent_level -= 1;
            }
            BindValue::Extern => {
                self.buffer.push_str("extern");
            }
            BindValue::Unassigned => {
                // No value for unassigned binds — type annotation already printed.
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
fn import_display_key(mi: &ModuleImport) -> String {
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
        ImportSource::LocalBundle(b) => b.root.as_str().to_lowercase(),
        ImportSource::CurrentModule { member } => {
            let mut s = member.export.as_str().to_lowercase();
            if let Some(alias) = &member.alias {
                s.push_str(" as ");
                s.push_str(alias.as_str().to_lowercase().as_str());
            }
            s
        }
        ImportSource::LocalMember(m) => {
            let mut s = m
                .local_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            s.push('.');
            s.push_str(m.member.export.as_str().to_lowercase().as_str());
            s
        }
    }
}

/// Get the source text for a spanned expression using the span table.
fn span_text(expr: &Typed<Expr>, st: &SpanTable, source: &str) -> String {
    let span = st.get(expr.span_id);
    if !span.is_empty() {
        span.extract(source).to_string()
    } else {
        expr_fallback(&expr.value, st, source)
    }
}

fn span_text_spanned_expr(expr: &Spanned<Expr>, st: &SpanTable, source: &str) -> String {
    let span = st.get(expr.span_id());
    if !span.is_empty() {
        span.extract(source).to_string()
    } else {
        expr.value.format_surface()
    }
}

/// Fallback expression formatting when span is zero-length.
fn expr_fallback(expr: &Expr, _st: &SpanTable, _source: &str) -> String {
    match expr {
        Expr::Lit(lit) => match lit {
            Literal::Number(n) => n.to_string(),
            Literal::Int(i) => i.to_string(),
            Literal::Float(HashFloat(f)) => f.to_string(),
            Literal::String(s) => format!("'{}'", s),
        },
        Expr::AnonymousTag(name) => name.as_str().to_string(),
        Expr::SelfRef => "self".to_string(),
        _ => String::new(),
    }
}

/// Line wrapping: break lines that exceed `max_width`.
fn wrap_lines(text: &str, max_width: usize) -> String {
    let mut result = String::with_capacity(text.len());
    let mut line_start = 0;
    for (i, ch) in text.char_indices() {
        if ch == '\n' {
            let line = &text[line_start..i];
            if line.len() > max_width {
                let indent: String = line.chars().take_while(|&c| c == ' ').collect();
                result.push_str(&wrap_single_line(line, max_width, &indent));
            } else {
                result.push_str(line);
            }
            result.push('\n');
            line_start = i + 1;
        }
    }
    if line_start < text.len() {
        result.push_str(&text[line_start..]);
        if !text.ends_with('\n') {
            result.push('\n');
        }
    }
    result
}

fn wrap_single_line(line: &str, max_width: usize, indent: &str) -> String {
    if line.len() <= max_width {
        return line.to_string();
    }
    let break_pos = find_break_point(line, max_width);
    let first = &line[..break_pos];
    let rest = line[break_pos..].trim_start();
    let ci = format!("{}    ", indent);
    if rest.len() + ci.len() <= max_width {
        format!("{}\n{}{}", first, ci, rest)
    } else {
        format!(
            "{}\n{}",
            first,
            wrap_single_line(&format!("{}{}", ci, rest), max_width, indent)
        )
    }
}

fn find_break_point(line: &str, max_width: usize) -> usize {
    let ops = [" + ", " - ", " == ", " != ", " < ", " > "];
    let target = max_width.min(line.len());
    for op in &ops {
        if let Some(pos) = line[..target].rfind(op) {
            let bp = pos + op.len();
            if bp < line.len() {
                return bp;
            }
        }
    }
    if let Some(pos) = line[..target].rfind(", ") {
        return pos + 2;
    }
    target
}
#[cfg(test)]
#[path = "tests/ast_formatter_tests.rs"]
mod tests;
