use ast::{BindValue, Expr, FileAst};
use diagnostic::Category;
use parser::query::{ParseOutput, SourceParseExt};

pub(crate) fn is_standalone_expression(source: &str, parsed: &ParseOutput) -> bool {
    if parsed.ast.defs.values().any(|definition| {
        !matches!(definition.value, BindValue::Unassigned)
            || definition.return_tag.is_some()
            || definition.return_type_name.is_some()
            || !definition.named_results.is_empty()
    }) {
        return false;
    }
    let syntax_flaws = parsed
        .symptoms
        .iter()
        .any(|diagnostic| diagnostic.category == Category::Flaw);
    if !syntax_flaws && !is_unassigned_function(&parsed.ast) {
        return false;
    }

    let candidate = format!("__gin_interpreter_probe: {source}\n");
    let expression = candidate.parse_source_full();
    if expression.is_incomplete()
        || expression
            .symptoms
            .iter()
            .any(|diagnostic| diagnostic.category == Category::Flaw)
        || !has_single_definition(&expression.ast)
    {
        return false;
    }
    let definition = expression.ast.defs.values().next().unwrap();
    definition.name.as_str() == "__gin_interpreter_probe"
        && definition.params.is_none()
        && matches!(&definition.value, BindValue::Expr(value) if !matches!(value.value, Expr::Bind(_)))
}

fn is_unassigned_function(ast: &FileAst) -> bool {
    if !has_single_definition(ast) {
        return false;
    }
    let definition = ast.defs.values().next().unwrap();
    definition.params.is_some() && matches!(definition.value, BindValue::Unassigned)
}

fn has_single_definition(ast: &FileAst) -> bool {
    ast.defs.len() == 1
        && ast.uses.is_empty()
        && ast.tags.is_empty()
        && ast.method_binds.is_empty()
        && ast.exprs.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InterpreterSession, RuntimeStorage, Submission};

    fn accept(session: &mut InterpreterSession, source: &str) {
        let result = session.submit(source);
        assert!(
            matches!(result, Submission::DeclarationAccepted { .. }),
            "{result:?}"
        );
    }

    fn assert_integer(session: &mut InterpreterSession, source: &str, expected: i64) {
        let result = session.submit(source);
        assert!(
            matches!(&result, Submission::Evaluated { value, .. }
            if value.storage == RuntimeStorage::Int64(expected)),
            "{source}: {result:?}"
        );
    }

    #[test]
    fn bare_calls_support_literals_variables_named_arguments_and_nesting() {
        let mut session = InterpreterSession::default();
        accept(&mut session, test_fixtures::preludes::WORD_PRELUDE);
        accept(&mut session, "echo(value Word) Word: return value\n");
        accept(&mut session, "value Word: 42\n");
        accept(&mut session, "zero Word: 42\n");
        accept(&mut session, "plain: 40\n");
        let declarations = session.declarations().to_owned();
        for source in [
            "echo(42)",
            "echo(value)",
            "echo(value: 42)",
            "echo(echo(value))",
            "zero()",
            "plain() + 2",
        ] {
            assert_integer(&mut session, source, 42);
            assert_eq!(session.declarations(), declarations);
        }
    }

    #[test]
    fn bare_compiler_queries_match_parenthesized_queries() {
        let mut session = InterpreterSession::default();
        for source in [
            "#size(@())",
            "#alignment(@())",
            "#index_bits(@())",
            "#target.arch",
            "#target.vendor",
            "#target.os",
            "#target.triple",
        ] {
            let result = session.submit(source);
            let Submission::Evaluated { value, .. } = result else {
                panic!("{source}: {result:?}");
            };
            let expected = session.submit(&format!("({source})"));
            assert!(
                matches!(expected, Submission::Evaluated { value: other, .. } if value == other)
            );
        }
        assert!(session.declarations().is_empty());
    }

    #[test]
    fn declaration_attributes_and_bodies_keep_the_declaration_path() {
        let mut session = InterpreterSession::default();
        let source = concat!(
            "#bits(8)\nByte is in 0 to 255\n",
            "Byte has\n    finish(self):\n        return\n",
        );
        accept(&mut session, source);
        accept(&mut session, "echo(value Byte) Byte: return value\n");
        assert_integer(&mut session, "echo(42)", 42);
        assert!(session.declarations().contains("#bits(8)"));
        assert!(
            session
                .declarations()
                .contains("echo(value Byte) Byte: return value")
        );
    }

    #[test]
    fn incomplete_calls_queries_and_declarations_still_request_input() {
        let mut session = InterpreterSession::default();
        for source in [
            "echo(",
            "echo(\n    42",
            "echo(value Word) Word:",
            "#size(",
            "#target.",
        ] {
            let result = session.submit(source);
            assert!(
                matches!(result, Submission::Incomplete),
                "{source}: {result:?}"
            );
        }
        assert!(session.declarations().is_empty());
        assert!(session.pending().is_none());
    }

    #[test]
    fn malformed_declarations_keep_the_original_parser_diagnostics() {
        let mut session = InterpreterSession::default();
        for source in [
            "broken(1) Word: 2",
            "broken(value Word) Word = 2",
            "broken(value Word, value Word) Word: return value",
        ] {
            let parsed = source.parse_source_full();
            let flaws = parsed
                .symptoms
                .iter()
                .filter(|diagnostic| diagnostic.category == Category::Flaw);
            assert!(flaws.clone().next().is_some(), "{source}");
            let result = session.submit(source);
            let Submission::Rejected { diagnostics } = result else {
                panic!("{source}: {result:?}");
            };
            for flaw in flaws {
                assert!(
                    diagnostics.iter().any(|diagnostic| {
                        diagnostic.source() == source && diagnostic.diagnostic() == flaw
                    }),
                    "{source}: {diagnostics:?}"
                );
            }
        }
        assert!(session.declarations().is_empty());
    }

    #[test]
    fn bare_call_diagnostics_and_retry_use_original_submission_coordinates() {
        let mut session = InterpreterSession::default();
        accept(&mut session, test_fixtures::preludes::WORD_PRELUDE);
        accept(
            &mut session,
            "-- café 🦀\necho(value Word) Word: return value\n",
        );
        let source = "echo(missing)";
        let result = session.submit(source);
        let Submission::Rejected { diagnostics } = result else {
            panic!("{result:?}");
        };
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.diagnostic().code.slug() == "type-unknown-symbol")
            .unwrap_or_else(|| panic!("{diagnostics:?}"));
        assert_eq!(diagnostic.source(), source);
        assert_eq!(diagnostic.display_path(), "<interpreter>");
        assert_eq!(&source[diagnostic.diagnostic().span.to_range()], "missing");
        assert_eq!(session.pending(), Some(source));
        accept(&mut session, "missing Word: 42\n");
        assert_integer(&mut session, "#retry", 42);
        assert!(session.pending().is_none());
    }

    #[test]
    fn expression_probe_rejects_declarations_and_extra_top_level_items() {
        for source in [
            "answer := 42",
            "echo(value Word) Word: return value",
            "echo(value Word) Word",
            "echo(1)\nanswer := 2",
            "#size(@())\nanswer := 2",
            "#bits(8)\nByte is in 0 to 255",
        ] {
            let parsed = source.parse_source_full();
            assert!(!is_standalone_expression(source, &parsed), "{source}");
        }
    }
}
