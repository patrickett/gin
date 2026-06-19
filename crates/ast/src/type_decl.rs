//! Utility for checking whether a name follows capitalized-type-name style
//! (used by the typechecker to decide whether to treat an unknown symbol as
//! a missing type vs. a missing value).

/// Extension trait for checking capitalized type names on `str`.
pub trait TypeNameExt {
    /// Returns `true` when `name` starts with an ASCII uppercase letter,
    /// matching Gin's convention that type names are capitalized.
    fn is_capitalized_type_name(&self) -> bool;
}

impl TypeNameExt for str {
    fn is_capitalized_type_name(&self) -> bool {
        self.chars().next().is_some_and(|c| c.is_ascii_uppercase())
    }
}
