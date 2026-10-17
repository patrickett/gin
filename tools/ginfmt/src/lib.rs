pub mod ast_formatter;
pub mod config;
mod expression_layout;
pub mod help;

pub use ast_formatter::AstFormatter;
pub use config::Config;
use diagnostic::Category;
use parser::query::SourceParseExt;

/// Format Gin source code using the Gin AST parser.
///
/// Parses the source into an `ast::FileAst`, walks it to build formatted
/// output.
pub fn format(source: &str) -> String {
    format_with_config(source, Config::default())
}

/// Format Gin source code using the provided configuration.
pub fn format_with_config(source: &str, config: Config) -> String {
    let output = source.parse_source_full();
    if output
        .symptoms
        .iter()
        .any(|symptom| symptom.category == Category::Flaw)
        || output.comments.iter().any(|comment| !comment.top_level)
    {
        return source.to_string();
    }
    let mut formatter =
        AstFormatter::new(source, &config, &output.ast.span_table).with_comments(&output.comments);
    let formatted = formatter.format_file(&output.ast);
    let layout_ast = formatted.parse_source_full();
    if !valid_output(&output, &layout_ast) {
        return source.to_string();
    }
    let bodies = help::format_value_bodies(&formatted, &layout_ast);
    let layout_ast = bodies.parse_source_full();
    if !valid_output(&output, &layout_ast) {
        return formatted;
    }
    let layout = expression_layout::format(&bodies, &layout_ast.ast, &config);
    let reparsed = layout.parse_source_full();
    if !valid_output(&output, &reparsed) {
        return bodies;
    }
    layout
}

fn valid_output(
    original: &parser::query::ParseOutput,
    formatted: &parser::query::ParseOutput,
) -> bool {
    !formatted
        .symptoms
        .iter()
        .any(|symptom| symptom.category == Category::Flaw)
        && original
            .comments
            .iter()
            .map(|comment| &comment.text)
            .eq(formatted.comments.iter().map(|comment| &comment.text))
}
