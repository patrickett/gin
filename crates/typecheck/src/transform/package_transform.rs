use crate::{FileId, typed::TypedFileAst};
use crate::compile_time_trait::CompileTimeTraitRegistry;
use ast::FileAst;
use std::sync::Arc;

use super::{PackageTransformOptions, TransformCtx, transform_package};

pub struct PackageTransformArtifacts {
    pub typed_asts: Vec<TypedFileAst>,
    pub compile_time_eval_ast: Arc<FileAst>,
    pub trait_registry: Option<CompileTimeTraitRegistry>,
}

pub fn transform_package_with_shared_context(
    mut file_asts: Vec<FileAst>,
    options: PackageTransformOptions,
) -> PackageTransformArtifacts {
    let mut compile_time_eval_ast = FileAst::default();
    for ast in &file_asts {
        compile_time_eval_ast.merge_from(ast.clone());
    }

    let compile_time_eval_ast = Arc::new(compile_time_eval_ast);
    let trait_registry = if options.ide_package {
        None
    } else {
        Some(CompileTimeTraitRegistry::from_parse_ast(
            &compile_time_eval_ast,
            Arc::clone(&compile_time_eval_ast),
        ))
    };

    let package_ctx = TransformCtx::with_package_compile_time_arc(Arc::clone(&compile_time_eval_ast));

    let file_asts_with_ids: Vec<(FileAst, FileId)> = file_asts
        .drain(..)
        .enumerate()
        .map(|(i, ast)| (ast, FileId(i as u32)))
        .collect();

    let typed_asts = transform_package(&file_asts_with_ids, &package_ctx, options);

    PackageTransformArtifacts {
        typed_asts,
        compile_time_eval_ast,
        trait_registry,
    }
}
