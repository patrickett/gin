//! Analysis pipeline — single entry point for type checking and flow analysis.

use crate::analysis::flow_analyzer::FlowAnalyzer;
use crate::{FileAst, HasSpanId};
use diagnostic::Diagnostic;
use diagnostic::DiagnosticLike;
use diagnostic::type_::TypeSymptom;

/// Analyze a file's AST for type errors and flow issues.
///
/// Resolves types from `all_asts` as needed, then runs type-checking
/// (unknown references, return-type matching, etc.) and flow analysis.
///
/// As a side effect, populates the AST nodes with resolved type info
/// (`bind.return_type`, `tag.resolved_type`, etc.) via
/// [`populate_ast_types`].
///
/// Returns all collected symptoms (type errors, flow warnings, etc.).
pub fn analyze_file(ast: &mut FileAst, all_asts: &[FileAst]) -> Vec<Diagnostic> {
    let mut symptoms = Vec::new();

    // Resolve types from all available ASTs.
    let mut resolved: Vec<crate::Analysis> = Vec::new();
    for a in all_asts {
        let analysis = crate::resolve_types(a, all_asts);
        resolved.push(analysis);
    }
    let analysis = &resolved[0];

    // Populate AST nodes with resolved type info
    crate::populate_ast_types(ast, analysis);

    crate::analysis::check::check_unknowns(
        ast,
        &analysis.tag_types,
        &analysis.explicit_tag_names,
        &analysis.fn_return_types,
        &analysis.variant_map,
        &mut symptoms,
    );

    let mut analyzer = FlowAnalyzer::new(
        &analysis.tag_types,
        &analysis.fn_return_types,
        &analysis.variant_map,
    );
    analyzer.analyze_file(ast);
    let result = analyzer.into_result();

    for check in &result.bounds_checks {
        symptoms.push(
            TypeSymptom::IndexOutOfBounds {
                index: check.index,
                size: check.size,
            }
            .into_diagnostic(check.span_id()),
        );
    }

    symptoms
}
