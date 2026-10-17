//! Prepare a parsed [`FileAst`] for typecheck and codegen.
//!
//! Runs compiler passes in a fixed order after parse (and after import resolution
//! when the driver has applied it. Both `ginc` and the IDE database must call
//! [`prepare_parse_ast`] before transform.

use flask::CompileTarget;

use crate::intrinsic_fold::{inject_compiler_intrinsic_expressions, inject_compiler_intrinsics};
use crate::prepare_target::{
    materialize_target_dependent_constants, materialize_type_static_access,
    materialize_when_declare_subjects_from_package, validate_when_declare_exhaustiveness,
};
use ast::FileAst;
use diagnostic::Diagnostic;

pub fn prepare_parse_ast(ast: &mut FileAst, entry: &CompileTarget) -> Vec<Diagnostic> {
    materialize_type_static_access(ast);
    materialize_target_dependent_constants(ast, entry);
    inject_compiler_intrinsic_expressions(ast);
    Vec::new()
}

/// Prepare every file in a package, then materialize cross-file `when` declare subjects.
pub fn prepare_package_asts(asts: &mut [FileAst], entry: &CompileTarget) -> Vec<Vec<Diagnostic>> {
    let mut diags: Vec<Vec<Diagnostic>> = asts
        .iter_mut()
        .map(|ast| prepare_parse_ast(ast, entry))
        .collect();

    let mut merged = FileAst::default();
    for ast in asts.iter() {
        merged.merge_from(ast.clone());
    }

    for (i, ast) in asts.iter_mut().enumerate() {
        materialize_when_declare_subjects_from_package(ast, &merged);
        diags[i].extend(validate_when_declare_exhaustiveness(ast));
        inject_compiler_intrinsics(ast);
    }

    diags
}
