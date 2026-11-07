use crate::{Name, Spanned};

/// One written `use` statement. Entries retain source order and duplicates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    pub entries: Vec<Spanned<ImportEntry>>,
}

/// A source, its optional selected exports, and its written namespace alias.
///
/// No effective name, filesystem path, or resolved target is computed here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportEntry {
    pub source: Spanned<ImportSource>,
    pub members: Vec<Spanned<ImportMember>>,
    pub alias: Option<Name>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportSource {
    /// A quoted path. Delimiters and escapes remain in the source span, not decoded.
    Path,
    /// Written dotted components, including `self` when present.
    Module(Vec<Name>),
    /// An uppercase export written without a module source.
    Current(Name),
}

/// One selected export, with every dotted component and alias located separately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportMember {
    pub path: Vec<Name>,
    pub alias: Option<Name>,
}
