use crate::{Binding, Item, SourceFile, Span, TypeDeclaration};

/// The written declaration form, without inferred function or semantic kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclarationKind {
    Binding,
    Type,
    /// A local positional binder, never a top-level outline entry.
    PatternBinder,
}

/// A borrowed declaration occurrence and its raw source locations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Declaration<'a> {
    pub name: &'a str,
    pub kind: DeclarationKind,
    pub span: Span,
    pub name_span: Span,
}

impl<'a> Declaration<'a> {
    pub(crate) fn from_binding(file: &SourceFile, span: Span, binding: &'a Binding) -> Self {
        Self {
            name: &binding.name.value,
            kind: DeclarationKind::Binding,
            span,
            name_span: file.span_table.get(binding.name.span_id),
        }
    }

    pub(crate) fn from_type(
        file: &SourceFile,
        span: Span,
        declaration: &'a TypeDeclaration,
    ) -> Self {
        Self {
            name: &declaration.name.value,
            kind: DeclarationKind::Type,
            span,
            name_span: file.span_table.get(declaration.name.span_id),
        }
    }
}

impl SourceFile {
    /// Top-level declarations in source order, including duplicates and private sections.
    ///
    /// Expressions, section markers, and rejected items are omitted. Local bindings
    /// and type members are not traversed; type extensions remain separate occurrences.
    pub fn declaration_outline(&self) -> impl Iterator<Item = Declaration<'_>> {
        self.items.iter().filter_map(|item| {
            let span = self.span_table.get(item.span_id);
            match &item.value {
                Item::Binding(binding) => Some(Declaration::from_binding(self, span, binding)),
                Item::Type(declaration) => Some(Declaration::from_type(self, span, declaration)),
                Item::Import(_)
                | Item::Expression(_)
                | Item::Private
                | Item::Entry
                | Item::Load
                | Item::Store
                | Item::Wrap
                | Item::Unrestricted
                | Item::Instruction { .. }
                | Item::Error => None,
            }
        })
    }
}
