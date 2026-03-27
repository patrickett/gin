//! Compile query - compiles parsed AST to MLIR bytecode.

use crate::{
    codegen::generate_mlir,
    database::{CompiledModule, File, input_database::Db},
    diagnostic::{Symptom, SymptomSource, type_ as type_symptom},
    parse::{parse, resolve_imports},
    typeck::{TyEnv, flow_analyzer::FlowAnalyzer, FlowAnalysis},
};
use chumsky::span::{SimpleSpan, Span};
use salsa::Accumulator;

/// Flow analysis result for a file - tracks type narrowing through control flow.
#[salsa::tracked]
pub fn flow_analysis<'db>(db: &'db dyn Db, file: File) -> FlowAnalysis {
    let ast = parse(db, file);
    let ty_env = TyEnv::from_file_ast(&ast);

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
/// This is the main compilation pipeline. It:
/// 1. Parses the file (emits parse symptoms)
/// 2. Type checks (emits unknown type/function/variable symptoms)
/// 3. Flow analyzes (via `flow_analysis`, emits bounds check symptoms)
/// 4. Generates MLIR bytecode (emits codegen errors)
///
/// All diagnostics are emitted via `.accumulate(db)` and can be retrieved
/// via `compile::accumulated::<Symptom>(&db, file)`.
#[salsa::tracked]
pub fn compile<'db>(db: &'db dyn Db, file: File) -> CompiledModule<'db> {
    let ast = parse(db, file);
    let ty_env = TyEnv::from_file_ast(&ast);

    // Type checking - emits unknown type/function/variable symptoms
    ty_env.check_unknowns(&ast, db);

    // Flow analysis - emits bounds check symptoms and returns result for hover
    let _flow_result = flow_analysis(db, file);

    // Code generation - emits codegen errors
    let mlir_text = generate_mlir(&ast);

    match mlir_text {
        Ok(text) => {
            let bytecode = text.into_bytes();
            CompiledModule::new(db, bytecode)
        }
        Err(e) => {
            let symptom = Symptom {
                source: SymptomSource::CodeGen(e),
                category: crate::diagnostic::Category::Flaw,
                span: SimpleSpan::new((), 0..0),
            };

            symptom.accumulate(db);
            CompiledModule::new(db, Vec::new())
        }
    }
}

/// Compile an entry point and all its dependencies.
///
/// This is the main entry point for compilation. It compiles the entry file
/// and all imported files. Salsa memoizes results per file.
#[salsa::tracked]
pub fn compile_entry<'db>(db: &'db dyn Db, entry: File) -> CompiledModule<'db> {
    let imported_files = resolve_imports(db, entry);

    // Compile all imported files (Salsa memoizes per-file results)
    for imported_file in imported_files {
        compile(db, imported_file);
    }

    // Compile the entry point and return its result
    compile(db, entry)
}
