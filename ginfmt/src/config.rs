/// Configuration options for the Gin formatter.
#[derive(Debug, Clone)]
pub struct Config {
    /// Enable alignment of `is` declarations.
    pub align_declarations: bool,
    /// Enable alignment of `:` bind statements.
    pub align_binds: bool,
    /// Enable alignment of `---` comment separators.
    pub align_comments: bool,
    /// Maximum line width before wrapping (default: 80).
    pub max_line_width: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            align_declarations: true,
            align_binds: true,
            align_comments: true,
            max_line_width: 80,
        }
    }
}
