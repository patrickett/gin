//! ID types for the typed AST — opaque file, definition, tag, variant, and expression identifiers.

use internment::Intern;

/// Opaque file identifier assigned during compilation coordination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileId(pub u32);

/// A definition (bind) identifier — the fully-qualified name.
/// Interned string, e.g. Intern("main") or Intern("Range.new").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DefId(pub Intern<String>);

/// A tag (type) identifier — the interned tag name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TagId(pub Intern<String>);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VariantId {
    pub union: TagId,
    pub name: Intern<String>,
}

/// Index into the expression arena (soa_derive TypedExprVec).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExprId(pub u32);

impl ExprId {
    pub fn as_usize(self) -> usize {
        self.0 as usize
    }
}
