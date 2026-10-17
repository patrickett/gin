/// Configuration options for the Gin formatter.
#[derive(Debug, Clone)]
pub struct Config {
    /// Maximum line width before wrapping (default: 80).
    pub max_line_width: usize,
    pub max_type_alignment_padding: usize,
    // TODO: Add `comment_fill_column` for relative comment wrapping (default: 70).
    //
    // Design: Comment content should be wrapped relative to the comment's start column,
    // not relative to the global line width. This means indented comments wrap narrower
    // than top-level comments, but never exceed the global max_line_width.
    //
    // Example with max_line_width=100, comment_fill_column=70:
    //     -- Top level comments can be this wide.
    //     some_method:
    //         -- Nested comments are
    //         -- also this wide, but
    //         -- shifted right.
    //         do_something Nothing
    //     return
    //
    // The effective wrap column for a comment is:
    //   min(comment_start_col + comment_fill_column, max_line_width)
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_line_width: 80,
            max_type_alignment_padding: 16,
        }
    }
}
