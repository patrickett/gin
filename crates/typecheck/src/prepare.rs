//! Prepare a parsed [`FileAst`] for typecheck and codegen.
//!
//! Runs compiler passes in a fixed order after parse (and after import resolution
//! when the driver has applied it. Both `ginc` and the IDE database must call
//! [`prepare_parse_ast`] before transform.

use flask::CompileTarget;

use crate::intrinsic_fold::inject_compiler_intrinsics;
use crate::prepare_target::{
    apply_entry_target_merge, materialize_default_binds, materialize_type_static_access,
    materialize_when_declare_subjects_from_package, validate_when_declare_exhaustiveness,
};
use ast::{DeclareValue, FileAst, HasMember};
use diagnostic::Diagnostic;

/// Parse-only preparation before semantic lowering.
///
/// This phase may materialize syntax-driven defaults and apply the entry target,
/// but it must not classify or evaluate expressions.
pub fn prepare_parse_ast(ast: &mut FileAst, entry: &CompileTarget) -> Vec<Diagnostic> {
    let mut diags = validate_auto_traits(ast);
    diags.extend(materialize_default_binds(ast));
    materialize_type_static_access(ast);
    diags.extend(apply_entry_target_merge(ast, entry));
    diags
}

fn validate_auto_traits(ast: &FileAst) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for decl in ast.tags.values() {
        if !decl.attributes.auto {
            continue;
        }

        match &decl.value {
            DeclareValue::Has(members) => {
                for member in members {
                    match member {
                        HasMember::Property(property) if property.body.is_none() => {
                            diags.push(auto_missing_default(
                                &decl.name,
                                property.name.as_str(),
                                property.name_span,
                                ast,
                            ));
                        }
                        HasMember::Function(function) if function.body.is_none() => {
                            diags.push(auto_missing_default(
                                &decl.name,
                                function.name.as_str(),
                                function.name_span,
                                ast,
                            ));
                        }
                        _ => {}
                    }
                }
            }
            _ => {
                diags.push(
                    Diagnostic::new(
                        "type-auto-attribute-target",
                        format!(
                            "#auto can only be used on trait declarations like `{}` has ...",
                            decl.name.as_str()
                        ),
                    )
                    .with_arg("name", decl.name.as_str().to_string())
                    .at_span_id(decl.name_span, &ast.span_table),
                );
            }
        }
    }
    diags
}

fn auto_missing_default(
    trait_name: &internment::Intern<String>,
    member_name: &str,
    span: ast::SpanId,
    ast: &FileAst,
) -> Diagnostic {
    Diagnostic::new(
        "type-auto-trait-member-default",
        format!(
            "auto trait `{}` requires a default for member `{member_name}`",
            trait_name.as_str()
        ),
    )
    .with_arg("trait_name", trait_name.as_str().to_string())
    .with_arg("member_name", member_name.to_string())
    .at_span_id(span, &ast.span_table)
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
mod tests {
    use super::*;
    use crate::prepare_parse_ast;
    use parser::cursor::TokenCursor;

    #[test]
    fn auto_trait_requires_member_defaults() {
        let mut ast = TokenCursor::parse_source("#auto\nCopy has can_copy Bool\n");
        let diags = prepare_parse_ast(&mut ast, &CompileTarget::Library);

        assert!(
            diags
                .iter()
                .any(|d| d.code.slug() == "type-auto-trait-member-default"),
            "diags: {diags:?}"
        );
    }

    #[test]
    fn auto_attribute_rejects_non_trait_declare() {
        let mut ast = TokenCursor::parse_source("#auto\nMaybe is Some(Int) or None\n");
        let diags = prepare_parse_ast(&mut ast, &CompileTarget::Library);

        assert!(
            diags
                .iter()
                .any(|d| d.code.slug() == "type-auto-attribute-target"),
            "diags: {diags:?}"
        );
    }

    #[test]
    fn parse_preparation_does_not_classify_or_fold_expressions() {
        let mut ast = TokenCursor::parse_source("value: 1 + 2\n");
        let diags = prepare_parse_ast(&mut ast, &CompileTarget::Library);

        assert!(diags.is_empty(), "diags: {diags:?}");
        let bind = ast.defs.get(&internment::Intern::from_ref("value"));
        assert!(matches!(
            bind.map(|bind| &bind.value),
            Some(ast::BindValue::Expr(expr)) if expr.const_value.is_none()
        ));
    }
}
