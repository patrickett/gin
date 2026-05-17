use internment::Intern;

use crate::expr::Expr;
use crate::expr::Typed;
use crate::path::ModPath;
use crate::span::Spanned;

/// A capitalized variant constructor call, e.g. `Some(5)` or `Maybe.Some(3)`.
///
/// Distinct from [`FnCall`] (which uses lowercase identifiers) — this constructs
/// a tagged union value with the given variant name and arguments.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TagCall {
    /// Simple variant name (e.g., "Some") - used for variant lookup
    pub name: Intern<String>,
    /// Optional qualified path (e.g., ModPath { root: "Maybe", segments: ["Some"] })
    pub qual_path: Option<Spanned<ModPath>>,
    pub args: Vec<Typed<Expr>>,
}
