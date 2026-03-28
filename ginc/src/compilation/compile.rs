//! Compile query - compiles parsed AST to MLIR bytecode.

use crate::{
    codegen::generate_mlir,
    database::{CompiledModule, File, input_database::Db},
    diagnostic::{Symptom, SymptomSource, type_ as type_symptom},
    parse::{parse, resolve_imports},
    typeck::{FlowAnalysis, TyEnv, flow_analyzer::FlowAnalyzer},
};

use salsa::Accumulator;

/// Get the shared type environment for a file's package.
///
/// This function collects all files in the package (entry + imports),
/// parses them, and creates a shared type environment that includes
/// all types from all files. This allows types defined in one file
/// to be used in another file within the same package.
///
/// Note: This is not a Salsa tracked function because TyEnv doesn't
/// implement Hash (due to containing HashMaps). Instead, this function
/// is called directly when needed.
pub fn shared_ty_env(db: &dyn Db, file: File) -> TyEnv {
    let imported_files = resolve_imports(db, file);

    // Collect all files (this file + imports)
    let mut all_files: Vec<File> = vec![file];
    all_files.extend(imported_files);

    // Parse all files and collect AST references
    let all_asts: Vec<crate::ast::FileAst> = all_files.iter().map(|f| parse(db, *f)).collect();

    // Create shared type environment from all files
    TyEnv::from_multiple_file_asts(&all_asts)
}

/// Flow analysis result for a file - tracks type narrowing through control flow.
#[salsa::tracked]
pub fn flow_analysis<'db>(db: &'db dyn Db, file: File) -> FlowAnalysis {
    let ast = parse(db, file);
    let ty_env = shared_ty_env(db, file);

    let mut analyzer = FlowAnalyzer::new(&ty_env);
    analyzer.analyze_file(&ast);

    let result = analyzer.into_result();

    for check in &result.bounds_checks {
        type_symptom::index_out_of_bounds(check.span, check.index, check.size).accumulate(db);
    }

    result
}

/// Compile a single file to MLIR bytecode.
///
/// This function performs flow analysis and code generation. Type checking
/// is done separately via `type_check_entry` to enable sharing type information
/// across all files in a package.
///
/// All diagnostics are emitted via `.accumulate(db)` and can be retrieved
/// via `compile::accumulated::<Symptom>(&db, file)`.
#[salsa::tracked]
pub fn compile<'db>(db: &'db dyn Db, file: File) -> CompiledModule<'db> {
    let ast = parse(db, file);

    // Note: Type checking is done in type_check_entry with shared TyEnv

    // Flow analysis - emits bounds check symptoms and returns result for hover
    let _flow_result = flow_analysis(db, file);

    // Code generation - emits codegen errors
    let source = file.contents(db);
    let filename = file.path(db).to_string_lossy().into_owned();
    let ty_env = shared_ty_env(db, file);
    let (mlir_text, codegen_symptoms) = generate_mlir(&ast, source, &filename, &ty_env);

    // Accumulate all codegen symptoms
    for e in codegen_symptoms {
        Symptom {
            source: SymptomSource::CodeGen(e.clone()),
            category: crate::diagnostic::Category::Flaw,
            span: e.span(),
        }
        .accumulate(db);
    }

    match mlir_text {
        Some(text) => {
            let bytecode = text.into_bytes();
            CompiledModule::new(db, bytecode)
        }
        None => CompiledModule::new(db, Vec::new()),
    }
}

/// Type check an entry point and all its dependencies with a shared type environment.
///
/// This function collects all imported files, creates a shared type environment
/// that includes all types from all files, and then type checks each file using
/// that shared environment. This allows types defined in one file to be used
/// in another file within the same package.
#[salsa::tracked]
pub fn type_check_entry<'db>(db: &'db dyn Db, entry: File) {
    let imported_files = resolve_imports(db, entry);

    // Collect all files (entry + imports)
    let mut all_files: Vec<File> = vec![entry];
    all_files.extend(imported_files);

    // Create shared type environment
    let ty_env = shared_ty_env(db, entry);

    // Type check each file using the shared type environment
    for file in &all_files {
        let ast = parse(db, *file);
        // Type check - emits unknown type/function/variable symptoms
        ty_env.check_unknowns(&ast, db);
    }
}

/// Compile an entry point and all its dependencies.
///
/// This is the main entry point for compilation. It:
/// 1. Type checks all files with a shared type environment
/// 2. Compiles all imported files (Salsa memoizes per-file results)
/// 3. Compiles the entry point and returns its result
#[salsa::tracked]
pub fn compile_entry<'db>(db: &'db dyn Db, entry: File) -> CompiledModule<'db> {
    // Type check all files with shared type environment
    type_check_entry(db, entry);

    let imported_files = resolve_imports(db, entry);

    // Compile all imported files (Salsa memoizes per-file results)
    for imported_file in imported_files {
        compile(db, imported_file);
    }

    // Compile the entry point and return its result
    compile(db, entry)
}
