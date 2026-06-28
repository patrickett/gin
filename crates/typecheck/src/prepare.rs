//! Prepare a parsed [`FileAst`] for typecheck and codegen.
//!
//! Runs compiler passes in a fixed order after parse (and after import resolution
//! when the driver has applied it). Both `ginc` and the IDE database must call
//! [`prepare_file_ast`] before transform.

use flask::CompileTarget;

use crate::analysis::{
    check_const_bind_after_declare, check_construction_refinements, fold_compile_time_binds,
    validate_compile_time_binds,
};
use crate::asm_intrinsics::inject_asm_exprs;
use crate::intrinsic_fold::inject_compiler_intrinsics;
use crate::prepare_target::{
    apply_entry_target_merge, materialize_default_binds, materialize_type_static_access,
    materialize_when_declare_subjects_from_package, validate_when_declare_exhaustiveness,
};
use ast::FileAst;
use diagnostic::Diagnostic;

/// Default materialization, entry `target` merge, compile-time validation/folding, asm injection.
///
/// Order:
/// 1. Materialize `has Default` on unassigned binds
/// 2. Classify comptime vs runtime; validate and fold comptime binds
/// 3. Merge entry flask triple into `target` **after** fold (fold would otherwise
///    re-evaluate `target` from `Target.default` and erase the triple override)
/// 4. Validate / simplify type-level `when` declares; inject asm from folded specs
pub fn prepare_file_ast(ast: &mut FileAst, entry: &CompileTarget) -> Vec<Diagnostic> {
    let mut diags = materialize_default_binds(ast);
    materialize_type_static_access(ast);
    use crate::comptime_classify::FileAstComptimeExt as _;
    ast.apply_comptime_classification();
    diags.extend(check_const_bind_after_declare(ast));
    diags.extend(validate_compile_time_binds(ast));
    fold_compile_time_binds(ast);
    diags.extend(check_construction_refinements(ast));
    diags.extend(apply_entry_target_merge(ast, entry));
    diags.extend(validate_when_declare_exhaustiveness(ast, &ast.tags));
    inject_compiler_intrinsics(ast);
    inject_asm_exprs(ast);
    diags
}

/// Prepare every file in a package, then materialize cross-file `when` declare subjects.
pub fn prepare_package_asts(asts: &mut [FileAst], entry: &CompileTarget) -> Vec<Vec<Diagnostic>> {
    let mut diags: Vec<Vec<Diagnostic>> = asts
        .iter_mut()
        .map(|ast| {
            let mut diags = prepare_file_ast(ast, entry);
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
