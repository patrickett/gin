//! Compile query - compiles parsed AST to MLIR bytecode.

use crate::{
    backend::codegen::generate_mlir,
    database::{CompiledModule, File, input_database::Db},
    diagnostic::{Symptom, SymptomSource},
    frontend::parser::{parse, resolve_imports},
};
use chumsky::span::{SimpleSpan, Span};
use salsa::Accumulator;

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
///
/// This recursively compiles all imported files before compiling
/// the entry point.
#[salsa::tracked]
pub fn compile_entry<'db>(db: &'db dyn Db, entry: File) -> CompiledModule<'db> {
    let imported_files = resolve_imports(db, entry);

    // TODO: parallel/async compile instead of sequential
    for imported_file in imported_files {
        compile(db, imported_file);
    }

    compile(db, entry)
}
