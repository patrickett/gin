use ariadne::Color;

/// Error severity levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    /// This is a compiler error and prevents compilation.
    Flaw,
    /// This gives a guided suggestion for an improvement. Does NOT prevent
    /// compilation.
    Help,
    /// This provides more detail but does not give guidance. Does NOT prevent
    /// compilation.
    Info,
}

impl Category {
    /// Get the ariadne color for this severity.
    pub fn color(&self) -> Color {
        use Category::*;
        use Color::*;
        match self {
            Flaw => Red,
            Help => Yellow,
            Info => Blue,
        }
    }

    /// Get the display name for this severity.
    pub fn as_str(&self) -> &str {
        use Category::*;
        match self {
            Flaw => "flaw",
            Help => "hint",
            Info => "info",
        }
    }

    /// Get the display name for this severity.
    pub fn as_char(&self) -> char {
        use Category::*;
        match self {
            Flaw => 'F',
            Help => 'H',
            Info => 'I',
        }
    }
}
