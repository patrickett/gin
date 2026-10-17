use ast::source::SourceExt;
use ast::{FileAst, HasSpanId, ImportSource, Ty};
use ast_format::declare::{BindFormatExt, DeclareFormatExt};
use diagnostic::Span;
use std::path::Path;
use typecheck::completions::FileAstCompletionExt;
use typecheck::{CompletionCandidate, CompletionKind, DefId, PackageSemanticIndex, TypedFileAst};

use crate::{HoverContent, format_hover_markdown};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Value,
    Function,
    Type,
    Module,
    Imported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeSymbol {
    pub name: String,
    pub signature: String,
    pub kind: SymbolKind,
    pub ty: Option<Ty>,
    pub span: Span,
    pub documentation: Option<String>,
}

pub struct SemanticQueries<'a> {
    source: &'a str,
    ast: &'a FileAst,
    typed: &'a TypedFileAst,
    package: &'a PackageSemanticIndex,
}

impl<'a> SemanticQueries<'a> {
    pub fn new(
        source: &'a str,
        ast: &'a FileAst,
        typed: &'a TypedFileAst,
        package: &'a PackageSemanticIndex,
    ) -> Self {
        Self {
            source,
            ast,
            typed,
            package,
        }
    }

    pub fn return_type(&self, definition: DefId) -> Option<&'a Ty> {
        self.typed
            .defs
            .get(&definition)
            .map(|bind| &bind.return_type)
    }

    pub fn hover(&self, byte_offset: usize) -> Option<HoverContent> {
        if let Some(hover) = keyword_hover(self.source, byte_offset) {
            return Some(hover);
        }
        let hover = self
            .typed
            .hover_at_byte(self.source, byte_offset, Some(self.package))?;
        Some(HoverContent {
            markdown: format_hover_markdown(&hover.markdown, hover.module_prefix.as_deref()),
            byte_range: self
                .source
                .word_byte_range(byte_offset)
                .map(|(start, end)| start..end),
        })
    }

    pub fn hover_with_imports(
        &self,
        path: &Path,
        byte_offset: usize,
        read: &dyn Fn(&Path) -> Option<resolve::ParsedFile>,
    ) -> Option<HoverContent> {
        Self::source_hover(path, self.source, self.ast, byte_offset, read)
            .or_else(|| self.hover(byte_offset))
    }

    pub(crate) fn source_hover(
        path: &Path,
        source: &str,
        ast: &FileAst,
        byte_offset: usize,
        read: &dyn Fn(&Path) -> Option<resolve::ParsedFile>,
    ) -> Option<HoverContent> {
        source.get(..byte_offset)?;
        if let Some(hover) = keyword_hover(source, byte_offset) {
            return Some(hover);
        }
        let target = resolve::resolve_import_at(ast, source, byte_offset)?;
        let markdown = resolve::hover_for_import_target(path, target, read)?;
        Some(HoverContent {
            markdown,
            byte_range: source
                .word_byte_range(byte_offset)
                .map(|(start, end)| start..end),
        })
    }

    pub fn scope(&self) -> Vec<ScopeSymbol> {
        let mut symbols = Vec::new();
        for (id, bind) in &self.ast.defs {
            symbols.push(self.binding(*id, bind));
        }
        for (name, declaration) in &self.ast.tags {
            symbols.push(ScopeSymbol {
                name: name.to_string(),
                signature: declaration
                    .surface_text()
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .to_owned(),
                kind: SymbolKind::Type,
                ty: self.typed.tag_types.get(&typecheck::TagId(*name)).cloned(),
                span: self.ast.span_table.get(declaration.name_span),
                documentation: declaration
                    .doc_comment
                    .as_ref()
                    .map(|doc| doc.value.clone()),
            });
        }
        self.imports(&mut symbols);
        symbols.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then(left.span.start().cmp(&right.span.start()))
        });
        symbols
    }

    fn binding(&self, id: DefId, bind: &ast::Bind) -> ScopeSymbol {
        let span = self.ast.span_table.get(bind.name_span);
        let typed = self.typed.defs.get(&id).or_else(|| {
            self.typed.defs.values().find(|candidate| {
                self.typed.span_table.get(candidate.name_span) == span
                    && candidate.receiver_type.is_some() == bind.receiver_type.is_some()
            })
        });
        let mut signature = bind.signature_surface();
        if bind.return_tag.is_none()
            && bind.return_type_name.is_none()
            && bind.named_results.is_empty()
            && bind.anonymous_result_alternatives.is_empty()
            && let Some(result @ Ty::Named { .. }) = typed.map(|bind| &bind.return_type)
        {
            signature.push(' ');
            signature.push_str(&result.format_for_hover());
        }
        ScopeSymbol {
            name: bind.name.to_string(),
            signature,
            kind: if bind.params.is_some() {
                SymbolKind::Function
            } else {
                SymbolKind::Value
            },
            ty: typed.map(|bind| bind.return_type.clone()),
            span,
            documentation: bind.doc_comment.as_ref().map(|doc| doc.value.clone()),
        }
    }

    pub fn scope_completions(&self) -> Vec<CompletionCandidate> {
        self.scope()
            .into_iter()
            .map(|symbol| CompletionCandidate {
                label: symbol.name,
                kind: match symbol.kind {
                    SymbolKind::Function => CompletionKind::Function,
                    SymbolKind::Type => CompletionKind::Tag,
                    SymbolKind::Value | SymbolKind::Module | SymbolKind::Imported => {
                        CompletionKind::Variable
                    }
                },
                detail: Some(symbol.signature),
                documentation: symbol.documentation,
                insert_text: None,
                is_snippet: false,
            })
            .collect()
    }

    pub fn completions(&self, byte_offset: usize) -> Vec<CompletionCandidate> {
        self.ast.completions_with_scope(
            self.source,
            byte_offset,
            Some(&self.package.tag_types),
            &self.scope_completions(),
        )
    }

    fn imports(&self, symbols: &mut Vec<ScopeSymbol>) {
        for module in self.ast.uses.iter().flat_map(|import| &import.0) {
            if matches!(
                module.source,
                ImportSource::LocalBundle(_) | ImportSource::CurrentModule { .. }
            ) {
                for (name, span) in module.bound_name_spans() {
                    let name = name.to_string();
                    if symbols.iter().any(|symbol| symbol.name == name) {
                        continue;
                    }
                    symbols.push(ScopeSymbol {
                        signature: format!("use {name}"),
                        name,
                        kind: SymbolKind::Imported,
                        ty: None,
                        span: self.ast.span_table.get(span),
                        documentation: None,
                    });
                }
            } else {
                let name = module
                    .alias
                    .map(|alias| alias.to_string())
                    .unwrap_or_else(|| module.effective_name());
                if symbols.iter().any(|symbol| symbol.name == name) {
                    continue;
                }
                symbols.push(ScopeSymbol {
                    signature: format!("use {name}"),
                    name,
                    kind: SymbolKind::Module,
                    ty: None,
                    span: self.ast.span_table.get(module.span_id()),
                    documentation: None,
                });
            }
        }
    }
}

fn keyword_hover(source: &str, byte_offset: usize) -> Option<HoverContent> {
    source.get(..byte_offset)?;
    if source.word_at_byte_offset(byte_offset)?.as_str() == "self" {
        return None;
    }
    let hover = crate::hover::HoverSnippet::keyword(source, byte_offset)?;
    Some(HoverContent {
        markdown: hover.markdown,
        byte_range: hover.byte_range,
    })
}
