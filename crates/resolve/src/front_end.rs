use std::collections::HashMap;
use std::path::PathBuf;

use ast::ty::PackageInstanceKey;
use diagnostic::Diagnostic;
use flask::CompileTarget;
use interface::PublicInterface;
use typecheck::TypedFileAst;
use typecheck::transform::{
    PackageTransformOptions, transform_package_with_shared_context_and_package,
};

use crate::{ImportDependencyGraph, ParsedFile, ParsedModuleCache, resolve_imports_with_home};

pub struct PackageFrontEndOptions {
    pub gin_home: Option<flask::GinHome>,
    pub compile_target: CompileTarget,
    pub transform: PackageTransformOptions,
    pub resolve_imports: bool,
    pub package_fallback: PackageInstanceKey,
    pub transform_paths: Option<Vec<PathBuf>>,
}

pub struct FrontEndFile {
    pub parsed: ParsedFile,
    pub typed: Option<TypedFileAst>,
    pub import_diagnostics: Vec<Diagnostic>,
    pub prepare_diagnostics: Vec<Diagnostic>,
}

pub struct PackageFrontEndOutput {
    pub files: Vec<FrontEndFile>,
    pub public_interface: PublicInterface,
    pub cache: ParsedModuleCache,
    pub import_graph: Option<ImportDependencyGraph>,
}

pub fn run_package_front_end(
    files: Vec<ParsedFile>,
    dependencies: &HashMap<String, PathBuf>,
    cache: ParsedModuleCache,
    options: PackageFrontEndOptions,
) -> PackageFrontEndOutput {
    let (files, cache, import_graph) = if options.resolve_imports {
        let resolved =
            resolve_imports_with_home(files, dependencies, cache, options.gin_home.as_ref());
        let mut files_by_path: HashMap<PathBuf, ParsedFile> = resolved
            .files
            .into_iter()
            .map(|file| (file.path.clone(), file))
            .collect();
        let file_paths = files_by_path.keys().cloned().collect::<Vec<_>>();
        let ordered_paths = resolved.import_graph.declaration_file_order(&file_paths);
        let ordered_files = ordered_paths
            .into_iter()
            .filter_map(|path| files_by_path.remove(&path))
            .collect();
        (ordered_files, resolved.cache, Some(resolved.import_graph))
    } else {
        let mut files = files;
        files.sort_by(|a, b| a.path.cmp(&b.path));
        (files, cache, None)
    };

    let mut files: Vec<_> = files
        .into_iter()
        .map(|mut parsed| {
            let import_diagnostics = if import_graph.is_some() {
                let parse_count = cache.files[&parsed.path].output.symptoms.len();
                parsed.output.symptoms.split_off(parse_count)
            } else {
                Vec::new()
            };
            let prepare_diagnostics =
                typecheck::prepare_parse_ast(&mut parsed.output.ast, &options.compile_target);
            FrontEndFile {
                parsed,
                typed: None,
                import_diagnostics,
                prepare_diagnostics,
            }
        })
        .collect();
    let package = files
        .iter()
        .find_map(|file| file.parsed.output.ast.semantic_origin.as_ref())
        .map(|origin| origin.package.clone())
        .unwrap_or(options.package_fallback);
    let selected: Vec<_> = files
        .iter_mut()
        .filter(|file| {
            options
                .transform_paths
                .as_ref()
                .is_none_or(|paths| paths.contains(&file.parsed.path))
        })
        .collect();
    let transform_asts = selected
        .iter()
        .map(|file| file.parsed.output.ast.clone())
        .collect();
    let artifacts = transform_package_with_shared_context_and_package(
        transform_asts,
        options
            .transform
            .with_compile_target(options.compile_target.clone()),
        package,
    );

    assert_eq!(selected.len(), artifacts.typed_asts.len());
    for (file, typed) in selected.into_iter().zip(artifacts.typed_asts) {
        file.typed = Some(typed);
    }

    PackageFrontEndOutput {
        files,
        public_interface: artifacts.public_interface,
        cache,
        import_graph,
    }
}
