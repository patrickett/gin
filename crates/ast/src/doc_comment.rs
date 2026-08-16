use derive_more::{Deref, From};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deref, From)]
pub struct DocComment {
    #[deref]
    pub value: String,
}

impl DocComment {
    /// Create a doc comment from a string value.
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
        }
    }

    /// Combine two doc comments, joining with a blank line.
    /// Returns `Some(a)` when only `a` is present,
    /// `Some(b)` when only `b` is present,
    /// `Some(a + "\n\n" + b)` when both are present,
    /// and `None` when both are absent or empty.
    pub fn combine(a: Option<Self>, b: Option<Self>) -> Option<Self> {
        match (a, b) {
            (Some(a), None) => {
                if a.is_empty() {
                    None
                } else {
                    Some(a)
                }
            }
            (None, Some(b)) => {
                if b.is_empty() {
                    None
                } else {
                    Some(b)
                }
            }
            (Some(a), Some(b)) => {
                let value = if a.value.is_empty() {
                    b.value
                } else if b.value.is_empty() {
                    a.value
                } else {
                    format!("{}\n\n{}", a.value, b.value)
                };
                let doc = Self { value };
                if doc.is_empty() { None } else { Some(doc) }
            }
            (None, None) => None,
        }
    }
}
