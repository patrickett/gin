//! This module exposes pure functions that parse source text and extract
//! information from ASTs without relying on incremental compilation
//! infrastructure.

use ast::span::{HasSpanId, SpanId};
use diagnostic::{Category, Diagnostic};
use lexer::{Lexer, Token};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::cursor::TokenCursor;
use ast::{FileAst, ImportSource};
use flask::{FlaskPathExt, NestedPackageTarget, PACKAGE_CONFIG_NAME};

/// Extension trait providing source-text parsing on `str`.
pub trait SourceParseExt {
    /// Parse source text and return full results including all diagnostics.
    ///
    /// This is the primary entry point for parsing when you need access to
    /// all diagnostic information (errors, warnings, hints).
    fn parse_source_full(&self) -> ParseOutput;
}

impl SourceParseExt for str {
    fn parse_source_full(&self) -> ParseOutput {
        let mut lexer = Lexer::new(self);
        let tokens: Vec<_> = std::iter::from_fn(|| lexer.next_raw()).collect();
        let lex_errors = std::mem::take(&mut lexer.errors);
        let mut span_table = lexer.take_span_table();

        let mut comments = Vec::new();
        let mut depth = 0usize;
        let mut line_start = true;
        for (token, span) in &tokens {
            match token {
                Token::Indent => depth += 1,
                Token::Dedent => depth = depth.saturating_sub(1),
                Token::Newline => line_start = true,
                Token::Comment(text) => comments.push(SourceComment {
                    text: (*text).to_owned(),
                    span: *span,
                    top_level: depth == 0 && line_start,
                }),
                _ => line_start = false,
            }
        }

        // Filter out comments for the handwritten parser
        let filtered_tokens: Vec<_> = tokens
            .iter()
            .filter(|(t, _)| !matches!(t, Token::Comment(_)))
            .copied()
            .collect();

        let (mut ast, hw_parse_errors) =
            TokenCursor::parse_tokens_with_errors(&filtered_tokens, &mut span_table);

        let mut symptoms: Vec<Diagnostic> = Vec::new();

        // Unterminated strings
        for (_, span_id) in tokens
            .iter()
            .filter(|(t, _)| matches!(t, Token::UnterminatedString(_)))
        {
            symptoms.push(
                Diagnostic::new("lex-unclosed-string", "unclosed string literal")
                    .at_span_id(*span_id, &span_table),
            );
        }

        // Lex errors (already Diagnostics from the lexer)
        symptoms.extend(lex_errors);

        // Parse errors -- some carry magic prefixes that indicate non-fatal hints.
        for err in &hw_parse_errors {
            if let Some(hint) = err.message().strip_prefix("__gin_hint__:") {
                match hint.trim() {
                    "unnecessary-comma" => {
                        symptoms.push(
                            Diagnostic::new("parse-unnecessary-comma", "unnecessary comma")
                                .with_category(Category::Warn)
                                .with_help(
                                    "this comma is not needed since Gin uses newlines \
                                 to separate items; you can remove it",
                                )
                                .at_span_id(err.span(), &span_table),
                        );
                    }
                    _hint => {
                        symptoms.push(
                            Diagnostic::new(
                                "parse-indented-return",
                                "return is indented inside the if body",
                            )
                            .with_category(Category::Warn)
                            .with_help("dedent this return so it closes the if expression")
                            .at_span_id(err.span(), &span_table),
                        );
                    }
                }
            } else {
                let mut diagnostic = Diagnostic::new(err.code(), err.message().to_string())
                    .at_span_id(err.span(), &span_table);
                if err.code() == "parse-reserved-name" {
                    diagnostic = diagnostic
                        .with_help("choose a different name, such as `value` or `result`");
                }
                symptoms.push(diagnostic);
            }
        }

        ast.span_table = span_table;
        symptoms.extend(ast.parse_warnings.iter().cloned());

        // Help hints (empty-paren suggestions) -- compute before spans borrows ast.
        let hints = ast.collect_empty_paren_hints();

        let spans = &ast.span_table;

        // Import validation
        for import in &ast.uses {
            for module_import in &import.0 {
                match &module_import.source {
                    ImportSource::LocalBundle(b) if module_import.alias.is_some() => {
                        symptoms.push(
                            Diagnostic::new(
                                "parse-unsupported-import-alias",
                                "alias individual exports with `export as alias`, rather than aliasing the whole import list",
                            )
                            .at_span_id(b.span_id(), spans),
                        );
                    }
                    ImportSource::Local(path, span) if module_import.alias.is_none() => {
                        symptoms.push(
                            Diagnostic::new(
                                "parse-missing-import-alias",
                                format!(
                                    "folder module import requires `as` (e.g. `use '{}' as name`)",
                                    path.display()
                                ),
                            )
                            .at_span_id(*span, spans),
                        );
                    }
                    ImportSource::LocalFile(_, _) => {}
                    _ => {}
                }
            }
        }

        for (suggested, span_id) in &hints {
            symptoms.push(
                Diagnostic::new("parse-empty-parens", "unnecessary parentheses")
                    .with_help(["use ", suggested, " instead of ", suggested, "()"].concat())
                    .with_category(Category::Warn)
                    .at_span_id(*span_id, spans),
            );
        }

        // Unused value info diagnostics
        for (value, span_id) in ast.collect_unused_values() {
            symptoms.push(
                Diagnostic::new("parse-unused-value", ["unused value ", &value].concat())
                    .with_arg("value", value.clone())
                    .with_category(Category::Help)
                    .at_span_id(span_id, spans),
            );
        }

        let is_incomplete = tokens_are_incomplete(&filtered_tokens);
        ParseOutput {
            ast,
            comments,
            symptoms,
            is_incomplete,
        }
    }
}

fn tokens_are_incomplete(tokens: &[(Token<'_>, SpanId)]) -> bool {
    let mut delimiters = Vec::new();
    let mut last = None;

    for (token, _) in tokens {
        match token {
            Token::ParenOpen => delimiters.push(Token::ParenClose),
            Token::BracketOpen => delimiters.push(Token::BracketClose),
            Token::CurlyOpen => delimiters.push(Token::CurlyClose),
            Token::ParenClose | Token::BracketClose | Token::CurlyClose => {
                if delimiters.last() == Some(token) {
                    delimiters.pop();
                }
            }
            Token::UnterminatedString(_) | Token::UnterminatedFormatString => return true,
            _ => {}
        }

        if !matches!(
            token,
            Token::Newline | Token::Indent | Token::Dedent | Token::Comment(_)
        ) {
            last = Some(token);
        }
    }

    !delimiters.is_empty()
        || matches!(
            last,
            Some(
                Token::Ampersand
                    | Token::And
                    | Token::ArrowLeft
                    | Token::Caret
                    | Token::Colon
                    | Token::ColonEq
                    | Token::Comma
                    | Token::Dot
                    | Token::Eq
                    | Token::EqEq
                    | Token::Greater
                    | Token::GreaterEq
                    | Token::In
                    | Token::Is
                    | Token::Less
                    | Token::LessEq
                    | Token::Minus
                    | Token::NotEq
                    | Token::Or
                    | Token::Percent
                    | Token::Pipe
                    | Token::Plus
                    | Token::ShiftLeft
                    | Token::ShiftRight
                    | Token::Slash
                    | Token::SlashOr
                    | Token::Star
                    | Token::Then
            )
        )
}

#[derive(Clone, PartialEq, Eq)]
pub struct SourceComment {
    pub text: String,
    pub span: SpanId,
    pub top_level: bool,
}

/// Full output from parsing source text, including all diagnostic info.
#[derive(Clone, PartialEq, Eq)]
pub struct ParseOutput {
    /// The parsed abstract syntax tree.
    pub ast: FileAst,
    pub comments: Vec<SourceComment>,
    /// All diagnostics collected during lexing and parsing.
    pub symptoms: Vec<Diagnostic>,
    is_incomplete: bool,
}

impl ParseOutput {
    pub fn is_incomplete(&self) -> bool {
        self.is_incomplete
    }
}

/// Extension trait providing hint/helper methods on [`FileAst`].
pub(crate) trait FileAstHintExt {
    /// Collect empty-paren call hints from the AST.
    fn collect_empty_paren_hints(&mut self) -> Vec<(String, SpanId)>;

    /// Collect unused top-level expressions as info diagnostics.
    fn collect_unused_values(&self) -> Vec<(String, SpanId)>;
}

impl FileAstHintExt for FileAst {
    fn collect_empty_paren_hints(&mut self) -> Vec<(String, SpanId)> {
        let mut collector = hints::EmptyParenCollector { hints: Vec::new() };
        let _ = ast::folder::walk_file_ast_mut(&mut collector, self);
        collector.hints
    }

    fn collect_unused_values(&self) -> Vec<(String, SpanId)> {
        let mut unused = Vec::new();

        for (expr, expr_span) in &self.exprs {
            let (value_str, span) = match expr {
                ast::Expr::Lit(lit) => {
                    let s = format!("{lit:?}");
                    (s, *expr_span)
                }
                ast::Expr::Binary(_b) => (String::from("binary expression"), *expr_span),
                ast::Expr::FnCall(call) => {
                    let name = hints::EmptyParenCollector::fmt_call(call);
                    (name, call.path.span_id())
                }
                ast::Expr::AnonymousTag(tag) => (tag.as_str().to_string(), *expr_span),
                _ => (String::from("expression"), *expr_span),
            };

            unused.push((value_str, span));
        }

        unused
    }
}

pub(crate) mod hints {
    use ast::FnCall;
    use ast::folder::{Folder, walk_bind_mut, walk_fn_call_mut};
    use ast::span::SpanId;
    use std::ops::ControlFlow;

    pub(crate) struct EmptyParenCollector {
        pub(crate) hints: Vec<(String, SpanId)>,
    }

    impl EmptyParenCollector {
        /// Format a function call name without trailing parentheses.
        pub(crate) fn fmt_call(call: &FnCall) -> String {
            if call.path.segments.is_empty() {
                call.path.root.as_str().to_string()
            } else {
                let segs: Vec<&str> = call.path.segments.iter().map(|s| s.as_str()).collect();
                format!("{}.{}", call.path.root.as_str(), segs.join("."))
            }
        }
    }

    impl Folder for EmptyParenCollector {
        fn visit_bind(&mut self, bind: &mut ast::Bind) -> ControlFlow<()> {
            if bind.params.as_ref().is_some_and(|params| params.is_empty()) {
                self.hints.push((bind.name.to_string(), bind.name_span));
            }
            walk_bind_mut(self, bind)
        }

        fn visit_fn_call(&mut self, call: &mut ast::FnCall) -> ControlFlow<()> {
            if let Some(args) = &call.args
                && args.is_empty()
            {
                self.hints.push((Self::fmt_call(call), call.path.span_id()));
            }
            walk_fn_call_mut(self, call)
        }
    }
}

/// Extension trait providing [`FileAst`] query methods for import resolution.
///
/// These methods parse the use-statements in a [`FileAst`] to resolve local
/// and package `.gin` file paths. Import the trait to call them as methods:
///
/// ```ignore
/// use parser::query::FileAstQueryExt;
/// let paths = ast.extract_local_import_paths(base_dir);
/// ```
pub trait FileAstQueryExt {
    /// Extract locally-imported `.gin` file paths from the AST (quoted `use '...gin'` only).
    ///
    /// Package imports, dependency bundles (`use dep ...`), and unquoted local roots are not included here.
    fn extract_local_import_paths(&self, base_dir: &Path) -> Vec<(PathBuf, SpanId)>;

    /// Resolve package imports (e.g. `use core`, `use core.subpkg`) against a dependency map.
    ///
    /// - `use dep`: every `*.gin` in `dep/` next to `flask.jsonc` (non-recursive).
    /// - `use dep.a.b`: nested folder modules `dep/a/b/` with `flask.jsonc`, then flat `*.gin` there.
    /// - `use dep a, b`: each listed name is a nested folder module under `dep/`.
    ///
    /// Returns `(path, span)` pairs for each resolved `.gin` file.
    fn extract_package_import_paths(
        &self,
        dependencies: &HashMap<String, PathBuf>,
    ) -> Vec<(PathBuf, SpanId)>;
}

impl FileAstQueryExt for FileAst {
    fn extract_local_import_paths(&self, base_dir: &Path) -> Vec<(PathBuf, SpanId)> {
        let mut paths = Vec::new();

        for import in &self.uses {
            for module_import in &import.0 {
                match &module_import.source {
                    ImportSource::Package(_path) => {}
                    ImportSource::LocalBundle(_b) => {}
                    ImportSource::CurrentModule { .. } => {}
                    ImportSource::Local(path, span) => {
                        let p = base_dir.join(path);
                        if p.is_file() && p.extension().is_some_and(|e| e == "gin") {
                            paths.push((p, *span));
                        } else if p.is_dir() && p.join(PACKAGE_CONFIG_NAME).is_file() {
                            for g in p.list_package_gin_files() {
                                paths.push((g, *span));
                            }
                        }
                    }
                    ImportSource::LocalFile(path, span) => {
                        let p = base_dir.join(path);
                        if p.is_file() {
                            paths.push((p, *span));
                        }
                    }
                }
            }
        }

        paths
    }

    fn extract_package_import_paths(
        &self,
        dependencies: &HashMap<String, PathBuf>,
    ) -> Vec<(PathBuf, SpanId)> {
        let mut paths = Vec::new();

        for import in &self.uses {
            for module_import in &import.0 {
                match &module_import.source {
                    ImportSource::Package(mod_path) => {
                        let dep_dir = match dependencies.get(mod_path.root.as_str()) {
                            Some(dir) => dir,
                            None => continue,
                        };

                        let span = mod_path.span_id();
                        if mod_path.segments.is_empty() {
                            if dep_dir.join(PACKAGE_CONFIG_NAME).is_file() {
                                for p in dep_dir.list_package_gin_files() {
                                    paths.push((p, span));
                                }
                            }
                            continue;
                        }

                        let segs: Vec<&str> =
                            mod_path.segments.iter().map(|s| s.as_str()).collect();
                        if let Ok(NestedPackageTarget::FolderModule(dir)) =
                            dep_dir.resolve_nested_package_path(&segs)
                        {
                            for p in dir.list_package_gin_files() {
                                paths.push((p, span));
                            }
                        }
                    }
                    ImportSource::LocalBundle(b) => {
                        let dep_dir = match dependencies.get(b.root.as_str()) {
                            Some(dir) => dir,
                            None => continue,
                        };
                        if !dep_dir.join(PACKAGE_CONFIG_NAME).is_file() {
                            continue;
                        }
                        let module_dir = b
                            .path_segments
                            .iter()
                            .fold(dep_dir.clone(), |dir, seg| dir.join(seg.as_str()));
                        if !module_dir.is_dir() {
                            continue;
                        }
                        let span = b.span_id();
                        for m in &b.members {
                            let nested = module_dir.join(m.export.as_str());
                            if nested.join(PACKAGE_CONFIG_NAME).is_file() {
                                for p in nested.list_package_gin_files() {
                                    paths.push((p, span));
                                }
                            }
                        }
                    }
                    ImportSource::Local(_, _) => {}
                    ImportSource::LocalFile(_, _) => {}
                    ImportSource::CurrentModule { .. } => {}
                }
            }
        }

        paths
    }
}
