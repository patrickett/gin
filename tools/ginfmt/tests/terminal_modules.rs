use diagnostic::Category;
use ginfmt::{AstFormatter, Config, format};
use parser::query::SourceParseExt;

mod common;

use common::assert_roundtrip;

#[test]
fn terminal_modules_apply_formatting_instead_of_falling_back_to_source() {
    for original in [
        include_str!("../../../modules/terminal/terminal.gin"),
        include_str!("../../../modules/terminal/output/output.gin"),
        include_str!("../../../modules/terminal/io/io.gin"),
        include_str!("../../../modules/terminal/event/event.gin"),
        include_str!("../../../modules/terminal_demo/main.gin"),
    ] {
        let source = original
            .replace(" :: ", "::")
            .replace("main SystemResult: when", "main SystemResult: return when");
        let source = source.as_str();
        let parsed = source.parse_source_full();
        let config = Config::default();
        let mut formatter = AstFormatter::new(source, &config, &parsed.ast.span_table)
            .with_comments(&parsed.comments);
        let formatted = formatter.format_file(&parsed.ast);
        let reparsed = formatted.parse_source_full();
        let flaws: Vec<_> = reparsed
            .symptoms
            .iter()
            .filter(|symptom| symptom.category == Category::Flaw)
            .collect();
        assert!(flaws.is_empty(), "{flaws:?}\n{formatted}");
        let formatted = format(source);
        assert!(!formatted.contains("column::"), "{formatted}");
        if source.contains("main SystemResult") {
            assert!(
                formatted.contains("main SystemResult: return when"),
                "{formatted}"
            );
        } else if source.contains("close(self) CloseResult") {
            assert!(
                formatted.contains("close(self) CloseResult: return when"),
                "{formatted}"
            );
        }
        assert_roundtrip(&formatted);
    }
}
