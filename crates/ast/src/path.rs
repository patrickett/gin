use internment::Intern;

/// A multi‑segment identifier.
///
/// `root` and every element of `segments` are **owned** strings.
/// This removes the need for a `'src` lifetime and allows the type to be
/// stored in caches or sent across threads safely.
///
/// The span for a `ModPath` is stored in its `Spanned<ModPath>` wrapper.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModPath {
    /// The root of a path can be a name mapped in `flask.jsonc` or
    /// a child folder in the current directory.
    ///
    /// NOTE: If there is a name conflict it will error.
    pub root: Intern<String>,
    pub segments: Vec<Intern<String>>,
}

impl ModPath {
    /// Construct a new `ModPath` from an owned root and segment list.
    pub fn new(root: Intern<String>, segments: Vec<Intern<String>>) -> Self {
        Self { root, segments }
    }
}
