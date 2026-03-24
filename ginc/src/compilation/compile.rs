//! Compile query - compiles parsed AST to MLIR bytecode.

use crate::{
    codegen::generate_mlir,
    database::{CompiledModule, File, input_database::Db},
    diagnostic::{Symptom, SymptomSource},
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

    analyzer.into_result()
}

/// Compile a single file to MLIR bytecode.
#[salsa::tracked]
pub fn compile<'db>(db: &'db dyn Db, file: File) -> CompiledModule<'db> {
    let ast = parse(db, file);

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
                span: SimpleSpan::new((), 0..0), // TODO: fix
            };

            symptom.accumulate(db);
            CompiledModule::new(db, Vec::new())
        }
    }
}

/// Compile an entry point and all its dependencies.
#[salsa::tracked]
pub fn compile_entry<'db>(db: &'db dyn Db, entry: File) -> CompiledModule<'db> {
    let imported_files = resolve_imports(db, entry);

    let tasks: Vec<_> = imported_files
        .into_iter()
        .map(|f| (db.clone_for_par(), f))
        .collect();

    rayon::scope(|s| {
        for (db_clone, imported_file) in tasks {
            s.spawn(move |_| {
                compile(&*db_clone, imported_file);
            });
        }
    });

    compile(db, entry)
}
