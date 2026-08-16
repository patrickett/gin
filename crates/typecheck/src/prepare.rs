//! Prepare a parsed [`FileAst`] for typecheck and codegen.
//!
//! Runs compiler passes in a fixed order after parse (and after import resolution
//! when the driver has applied it. Both `ginc` and the IDE database must call
//! [`prepare_parse_ast`] before transform.

use flask::CompileTarget;

use crate::intrinsic_fold::inject_compiler_intrinsics;
use crate::prepare_target::{
    apply_entry_target_merge, materialize_default_binds, materialize_target_dependent_constants,
    materialize_type_static_access, materialize_when_declare_subjects_from_package,
    validate_when_declare_exhaustiveness,
};
use ast::FileAst;
use diagnostic::Diagnostic;

/// Parse-only preparation before semantic lowering.
///
/// This phase may materialize syntax-driven defaults and apply the entry target,
/// but it must not classify or evaluate expressions.
pub fn prepare_parse_ast(ast: &mut FileAst, entry: &CompileTarget) -> Vec<Diagnostic> {
    let mut diags = materialize_default_binds(ast);
    materialize_type_static_access(ast);
    diags.extend(apply_entry_target_merge(ast, entry));
    materialize_target_dependent_constants(ast, entry);
    diags
}

/// Prepare every file in a package, then materialize cross-file `when` declare subjects.
pub fn prepare_package_asts(asts: &mut [FileAst], entry: &CompileTarget) -> Vec<Vec<Diagnostic>> {
    let mut diags: Vec<Vec<Diagnostic>> = asts
        .iter_mut()
        .map(|ast| {
            let mut diags = prepare_parse_ast(ast, entry);
            // Type-level `when` subjects may depend on tags or const binds from sibling files.
            // Re-run this validation after package merge below so exhaustive cross-file cases
            // don't keep stale single-file MissingElse/UnreachableElse diagnostics.
            diags.retain(|d| {
                d.code.slug() != "type-missing-else-arm"
                    && d.code.slug() != "type-unreachable-else-arm"
            });
            diags
        })
        .collect();

    let mut merged = FileAst::default();
    for ast in asts.iter() {
        merged.merge_from(ast.clone());
    }

    for (i, ast) in asts.iter_mut().enumerate() {
        materialize_when_declare_subjects_from_package(ast, &merged);
        diags[i].extend(validate_when_declare_exhaustiveness(ast, &merged.tags));
        inject_compiler_intrinsics(ast);
    }

    diags
}
#[cfg(test)]
#[path = "../tests/prepare_tests.rs"]
mod tests;
