//! AST-based formatter for Gin source code.
//!
//! Walks the Gin `FileAst` and builds formatted output using AST node properties
//! instead of operating on raw tree-sitter nodes or source text slices.

use std::collections::HashMap;

use ast::{
    BindValue, Declare, DeclareValue, DocComment, Expr, FileAst, ImportSource, Literal,
    ParameterKind, Parameters, SpanTable, Spanned, TypeExpr, Variant,
};
use internment::Intern;

use crate::align_ast::{AlignableNode, DelimiterKind, group_alignable_nodes};
use crate::config::Config;

struct AlignmentGroup {
    max_prefix_width: usize,
}

/// The AST-based formatter.
pub struct AstFormatter<'a> {
    source: &'a str,
    buffer: String,
    config: &'a Config,
    span_table: &'a SpanTable,
    indent_level: usize,
    alignable_nodes: Vec<AlignableNode>,
    next_node_id: usize,
    alignment_groups: HashMap<(usize, DelimiterKind), AlignmentGroup>,
}

impl<'a> AstFormatter<'a> {
    pub fn new(source: &'a str, config: &'a Config, span_table: &'a SpanTable) -> Self {
        Self {
            source,
            buffer: String::with_capacity(source.len()),
            config,
            span_table,
            indent_level: 0,
            alignable_nodes: Vec::new(),
            next_node_id: 0,
            alignment_groups: HashMap::new(),
        }
    }

    fn next_id(&mut self) -> usize {
        let id = self.next_node_id;
        self.next_node_id += 1;
        id
    }

    /// Format the entire file AST.
    pub fn format_file(&mut self, ast: &FileAst) -> String {
        self.format_file_inner(ast);
        self.set_alignment_groups();
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
            .tags()
            .iter()
            .map(|(n, d)| {
                let span = self.span_table.get(d.name_span);
                (span.start, n, d)
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
            .defs()
            .iter()
            .map(|(n, b)| {
                let span = self.span_table.get(b.name_span);
                (span.start, n, b)
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
        if let Some(doc) = declare.doc_comment() {
            self.emit_doc_comment(doc);
        }
        self.buffer.push_str(name.as_str());
        if let Some(params) = declare.params() {
            self.format_params(params);
        }
        let prefix_end = self.buffer.len();
        let st = self.span_table;
        let src = self.source;
        match declare.value() {
            DeclareValue::Union { variants } => {
                self.buffer.push_str(" is ");
                self.format_union_variants(variants);
            }
            DeclareValue::Record(fields) => {
                self.buffer.push_str(" is (");
                self.format_record_fields(fields);
                self.buffer.push(')');
            }
            DeclareValue::Alias(target) => {
                self.buffer.push_str(" is ");
                let span = st.get(target.span_id());
                self.buffer.push_str(span.extract(src));
            }
            DeclareValue::Set(..) => {
                self.buffer.push_str(" is set");
            }
            DeclareValue::Range(_, _) | DeclareValue::InRange(_, _) => {
                self.buffer.push_str(" is ");
                let span = st.get(declare.name_span);
                let end = src[span.start..]
                    .find('\n')
                    .map(|p| span.start + p)
                    .unwrap_or(src.len());
                self.buffer.push_str(&src[span.start..end]);
            }
        }
        let is_single_line = !self.buffer[prefix_end.saturating_sub(1)..].contains('\n');
        let sl = self.buffer[..prefix_end].matches('\n').count();
        if self.config.align_declarations && is_single_line {
            let nid = self.next_id();
            let il = self.indent_level;
            self.alignable_nodes.push(AlignableNode {
                node_id: nid,
                prefix_display_width: prefix_end,
                kind: DelimiterKind::Is,
                indent_level: il,
                source_line: sl,
            });
        }
    }

    fn format_params(&mut self, params: &Parameters) {
        self.buffer.push('[');
        let st = self.span_table;
        let src = self.source;
        for (i, (name, kind)) in params.iter().enumerate() {
            if i > 0 {
                self.buffer.push_str(", ");
            }
            self.buffer.push_str(name.as_str());
            match kind {
                ParameterKind::Generic => {}
                ParameterKind::Tagged(expr) => {
                    self.buffer.push(' ');
                    if let Some(te) = expr.value.as_type_expr() {
                        self.buffer.push_str(&type_text(&te));
                    }
                }
                ParameterKind::Default(expr) => {
                    let text = span_text(expr, st, src);
                    self.buffer.push_str(": ");
                    self.buffer.push_str(&text);
                }
            }
        }
        self.buffer.push(']');
    }

    fn format_union_variants(&mut self, variants: &[Variant]) {
        let st = self.span_table;
        let src = self.source;
        for (i, variant) in variants.iter().enumerate() {
            if i > 0 {
                self.buffer.push_str(" or ");
            }
            let shape = variant.shape();
            let name = variant_name(&shape.value);
            self.buffer.push_str(&name);
            if let TypeExpr::Generic { params, .. } = &shape.value
                && !params.is_empty()
            {
                self.buffer.push('(');
                for (j, (fname, fkind)) in params.iter().enumerate() {
                    if j > 0 {
                        self.buffer.push_str(", ");
                    }
                    self.buffer.push_str(fname.as_str());
                    self.buffer.push(' ');
                    match fkind {
                        ParameterKind::Generic => {}
                        ParameterKind::Tagged(expr) => {
                            if let Some(te) = expr.value.as_type_expr() {
                                self.buffer.push_str(&type_text(&te));
                            }
                        }
                        ParameterKind::Default(expr) => {
                            let text = span_text(expr, st, src);
                            self.buffer.push_str(&text);
                        }
                    }
                }
                self.buffer.push(')');
            }
        }
    }

    fn format_record_fields(&mut self, fields: &Parameters) {
        let st = self.span_table;
        let src = self.source;
        for (i, (name, kind)) in fields.iter().enumerate() {
            if i > 0 {
                self.buffer.push_str(", ");
            }
            self.buffer.push_str(name.as_str());
            match kind {
                ParameterKind::Generic => {}
                ParameterKind::Tagged(expr) => {
                    self.buffer.push(' ');
                    if let Some(te) = expr.value.as_type_expr() {
                        self.buffer.push_str(&type_text(&te));
                    }
                }
                ParameterKind::Default(expr) => {
                    let text = span_text(expr, st, src);
                    self.buffer.push_str(": ");
                    self.buffer.push_str(&text);
                }
            }
        }
    }

    fn format_import(&mut self, import: &ast::Import) {
        for mi in &import.0 {
            self.buffer.push_str("use ");
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
                    self.buffer.push_str(".(");
                    for (j, member) in b.members.iter().enumerate() {
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
            }
            if let Some(alias) = &mi.alias {
                self.buffer.push_str(" as ");
                self.buffer.push_str(alias.as_str());
            }
        }
    }

    fn format_bind(&mut self, name: &Intern<String>, bind: &ast::Bind) {
        let _ = name;
        if let Some(doc) = bind.doc_comment() {
            self.emit_doc_comment(doc);
        }
        self.buffer.push_str(bind.name().as_str());
        if let Some(params) = bind.params() {
            self.format_params(params);
        }
        let st = self.span_table;
        let src = self.source;
        if let Some(ret_tag) = &bind.return_tag {
            let text = span_text_type(ret_tag, st, src);
            self.buffer.push(' ');
            self.buffer.push_str(&text);
        }
        if bind.is_const {
            self.buffer.push_str(" := ");
        } else {
            self.buffer.push_str(": ");
        }
        let prefix_end = self.buffer.len();

        match bind.value() {
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
        }

        let is_single_line = !self.buffer[prefix_end.saturating_sub(1)..].contains('\n');
        let sl = self.buffer[..prefix_end].matches('\n').count();
        if self.config.align_binds && matches!(bind.value(), BindValue::Expr(_)) && is_single_line {
            let nid = self.next_id();
            let il = self.indent_level;
            self.alignable_nodes.push(AlignableNode {
                node_id: nid,
                prefix_display_width: prefix_end,
                kind: DelimiterKind::Colon,
                indent_level: il,
                source_line: sl,
            });
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
                if self.config.align_comments {
                    let trimmed_len = trimmed.len();
                    let pw = self.buffer.len().saturating_sub(trimmed_len + 4);
                    let sl = self.buffer[..pw].matches('\n').count();
                    let node_id = self.next_id();
                    let il = self.indent_level;
                    self.alignable_nodes.push(AlignableNode {
                        node_id,
                        prefix_display_width: pw,
                        kind: DelimiterKind::Dash,
                        indent_level: il,
                        source_line: sl,
                    });
                }
            }
        }
    }

    fn emit_indent(&mut self) {
        for _ in 0..self.indent_level {
            self.buffer.push_str("    ");
        }
    }

    fn set_alignment_groups(&mut self) {
        use DelimiterKind::*;
        let ord = |k: DelimiterKind| -> u8 {
            match k {
                Is => 0,
                Colon => 1,
                Dash => 2,
            }
        };
        self.alignable_nodes.sort_by(|a, b| {
            (ord(a.kind), a.indent_level, a.source_line).cmp(&(
                ord(b.kind),
                b.indent_level,
                b.source_line,
            ))
        });
        for group in &group_alignable_nodes(&self.alignable_nodes) {
            if group.len() < 2 {
                continue;
            }
            let max = group
                .iter()
                .map(|&i| self.alignable_nodes[i].prefix_display_width)
                .max()
                .unwrap_or(0);
            for &idx in group {
                let n = &self.alignable_nodes[idx];
                self.alignment_groups
                    .entry((n.source_line, n.kind))
                    .and_modify(|g| g.max_prefix_width = g.max_prefix_width.max(max))
                    .or_insert(AlignmentGroup {
                        max_prefix_width: max,
                    });
            }
        }
    }
}

/// Get the source text for a spanned expression using the span table.
fn span_text(expr: &Spanned<Expr>, st: &SpanTable, source: &str) -> String {
    let span = st.get(expr.span_id());
    if !span.is_empty() {
        span.extract(source).to_string()
    } else {
        expr_fallback(&expr.value, st, source)
    }
}

fn span_text_type(expr: &Spanned<TypeExpr>, st: &SpanTable, source: &str) -> String {
    let span = st.get(expr.span_id());
    if !span.is_empty() {
        span.extract(source).to_string()
    } else {
        type_text(&expr.value)
    }
}

/// Extract the variant name from a shape TypeExpr.
fn variant_name(expr: &TypeExpr) -> String {
    match expr {
        TypeExpr::Nominal(name, _) => name.as_str().to_string(),
        TypeExpr::Qualified(path) => path
            .segments
            .last()
            .map(|s| s.as_str().to_string())
            .unwrap_or_else(|| path.root.as_str().to_string()),
        TypeExpr::Generic { name, .. } => name.as_str().to_string(),
        TypeExpr::Literal(..) => String::new(),
    }
}

/// Format a type expression name.
fn type_text(expr: &TypeExpr) -> String {
    match expr {
        TypeExpr::Nominal(name, _) => name.as_str().to_string(),
        TypeExpr::Qualified(path) => {
            let mut s = path.root.as_str().to_string();
            for seg in &path.segments {
                s.push('.');
                s.push_str(seg.as_str());
            }
            s
        }
        TypeExpr::Generic { name, .. } => name.as_str().to_string(),
        TypeExpr::Literal(..) => String::new(),
    }
}

/// Fallback expression formatting when span is zero-length.
fn expr_fallback(expr: &Expr, st: &SpanTable, source: &str) -> String {
    match expr {
        Expr::Lit(lit) => match lit {
            Literal::Number(n) => n.to_string(),
            Literal::Int(i) => i.to_string(),
            Literal::Float(f) => f.to_string(),
            Literal::String(s) => format!("'{}'", s),
        },
        Expr::AnonymousTag(name, span) => {
            let s = st.get(*span);
            if !s.is_empty() {
                s.extract(source).to_string()
            } else {
                name.as_str().to_string()
            }
        }
        Expr::SelfRef(_) => "self".to_string(),
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
mod tests {
    use super::*;
    use parser::parse_source_full;

    #[test]
    fn test_basic_declare() {
        let source = "Maybe is Some or None\nResult is Ok or Error\n";
        let out = parse_source_full(source);
        let cfg = Config::default();
        let mut f = AstFormatter::new(source, &cfg, &out.ast.span_table);
        let r = f.format_file(&out.ast);
        assert_eq!(r, "Maybe is Some or None\nResult is Ok or Error\n");
    }

    #[test]
    fn test_simple_bind() {
        let source = "main:\n    print('hello')\nreturn\n";
        let out = parse_source_full(source);
        let cfg = Config::default();
        let mut f = AstFormatter::new(source, &cfg, &out.ast.span_table);
        let r = f.format_file(&out.ast);
        assert!(r.contains("main:"));
        assert!(r.contains("print('hello')"));
        assert!(r.contains("return"));
    }

    #[test]
    fn test_empty() {
        let source = "";
        let out = parse_source_full(source);
        let cfg = Config::default();
        let mut f = AstFormatter::new(source, &cfg, &out.ast.span_table);
        let r = f.format_file(&out.ast);
        assert_eq!(r, "");
    }
}
