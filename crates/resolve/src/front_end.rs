use std::collections::HashMap;
use std::path::PathBuf;

use ast::ty::PackageInstanceKey;
use diagnostic::Diagnostic;
use flask::CompileTarget;
use typecheck::transform::{
    PackageTransformArtifacts, PackageTransformOptions,
    transform_package_with_shared_context_and_package,
};

use crate::{ImportDependencyGraph, ParsedFile, ParsedModuleCache, resolve_imports_with_graph};

pub struct PackageFrontEndOptions {
    pub compile_target: CompileTarget,
    pub transform: PackageTransformOptions,
    pub resolve_imports: bool,
    pub package_fallback: PackageInstanceKey,
    pub transform_paths: Option<Vec<PathBuf>>,
}

pub struct PackageFrontEndOutput {
    pub files: Vec<ParsedFile>,
    pub prepare_diagnostics: Vec<Vec<Diagnostic>>,
    pub artifacts: PackageTransformArtifacts,
    pub cache: ParsedModuleCache,
    pub import_graph: Option<ImportDependencyGraph>,
}

pub fn run_package_front_end(
    files: Vec<ParsedFile>,
    dependencies: &HashMap<String, PathBuf>,
    cache: ParsedModuleCache,
    options: PackageFrontEndOptions,
) -> PackageFrontEndOutput {
    let (mut files, cache, import_graph) = if options.resolve_imports {
        let resolved = resolve_imports_with_graph(files, dependencies, cache);
        let mut files_by_path: HashMap<PathBuf, ParsedFile> = resolved
            .files
            .iter()
            .cloned()
            .map(|file| (file.path.clone(), file))
            .collect();
        let file_paths = files_by_path.keys().cloned().collect::<Vec<_>>();
        let ordered_paths = resolved.import_graph.declaration_file_order(&file_paths);
        let mut ordered_files = Vec::with_capacity(ordered_paths.len());
        for path in ordered_paths {
            if let Some(file) = files_by_path.remove(&path) {
                ordered_files.push(file);
            }
        }
        (ordered_files, resolved.cache, Some(resolved.import_graph))
    } else {
        (files, cache, None)
    };

    let prepare_diagnostics = files
        .iter_mut()
        .map(|file| typecheck::prepare_parse_ast(&mut file.output.ast, &options.compile_target))
        .collect();
    let package = files
        .iter()
        .find_map(|file| file.output.ast.semantic_origin.as_ref())
        .map(|origin| origin.package.clone())
        .unwrap_or(options.package_fallback);
    let transform_asts = files
        .iter()
        .filter(|file| {
            options
                .transform_paths
                .as_ref()
                .is_none_or(|paths| paths.contains(&file.path))
        })
        .map(|file| file.output.ast.clone())
        .collect();
    let artifacts = transform_package_with_shared_context_and_package(
        transform_asts,
        options
            .transform
            .with_compile_target(options.compile_target.clone()),
        package,
    );

    PackageFrontEndOutput {
        files,
        prepare_diagnostics,
        artifacts,
        cache,
        import_graph,
    }
}
