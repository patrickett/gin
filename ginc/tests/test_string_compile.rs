use crossbeam_channel::unbounded;
use ginc::backend::compile::compile;
use ginc::database::{File, input_database::InputDatabase};
use ginc::diagnostic::Symptom;
use ginc::frontend::parser::parse;
use std::path::PathBuf;

#[test]
fn test_parse_string_literal() {
    let src = "hello_text: 'hello'\n";

    let (tx, _rx) = unbounded();
    let db = InputDatabase::new(tx);
    let file = File::new(&db, PathBuf::from("test.gin"), src.to_string());

    // Just parse, don't compile
    let _ast = parse(&db, file);
}

#[test]
fn test_compile_number_literal() {
    let src = "hello_text: 42\n";

    let (tx, _rx) = unbounded();
    let db = InputDatabase::new(tx);
    let file = File::new(&db, PathBuf::from("test.gin"), src.to_string());

    // This should compile successfully
    let _compiled = compile(&db, file);
}

#[test]
fn test_compile_empty_function() {
    let src = "hello_text: 42\n";

    let (tx, _rx) = unbounded();
    let db = InputDatabase::new(tx);
    let file = File::new(&db, PathBuf::from("test.gin"), src.to_string());

    // This should compile successfully
    let _compiled = compile(&db, file);

    // Collect diagnostics only, don't print
    let diagnostics = compile::accumulated::<Symptom>(&db, file);
    println!("Diagnostics count: {}", diagnostics.len());
}

#[test]
fn test_compile_string_literal() {
    let src = "hello_text: 'hello'\n";

    let (tx, _rx) = unbounded();
    let db = InputDatabase::new(tx);
    let file = File::new(&db, PathBuf::from("test.gin"), src.to_string());

    let compiled = compile(&db, file);

    let diagnostics = compile::accumulated::<Symptom>(&db, file);
    assert_eq!(diagnostics.len(), 0, "expected no diagnostics");

    let mlir = String::from_utf8_lossy(compiled.bytecode(&db));
    assert!(
        mlir.contains("llvm.mlir.global"),
        "should contain global: {mlir}"
    );
    assert!(
        mlir.contains("llvm.mlir.addressof"),
        "should contain addressof: {mlir}"
    );
    assert!(
        mlir.contains("llvm.insertvalue"),
        "should contain insertvalue: {mlir}"
    );
    assert!(
        mlir.contains("llvm.struct"),
        "should contain struct type: {mlir}"
    );
}

#[test]
fn test_compile_string_literal_with_print() {
    let src = "hello_text: 'hello'\n";

    let (tx, _rx) = unbounded();
    let db = InputDatabase::new(tx);
    let file = File::new(&db, PathBuf::from("test.gin"), src.to_string());

    // This should compile successfully
    let _compiled = compile(&db, file);

    // Collect and print diagnostics (this is where segfault might occur)
    let diagnostics = compile::accumulated::<Symptom>(&db, file);

    // Print each diagnostic
    for diagnostic in &diagnostics {
        diagnostic.print(src, "test.gin");
    }
}

#[test]
fn test_compile_unterminated_string() {
    let src = "hello_text: 'hello\n";

    let (tx, _rx) = unbounded();
    let db = InputDatabase::new(tx);
    let file = File::new(&db, PathBuf::from("test.gin"), src.to_string());

    // This should parse (error is accumulated as diagnostic)
    let _compiled = compile(&db, file);

    // Collect and print diagnostics
    let diagnostics = compile::accumulated::<Symptom>(&db, file);

    // Print each diagnostic - this is where segfault might occur
    for diagnostic in &diagnostics {
        diagnostic.print(src, "test.gin");
    }
}

#[test]
fn test_compile_lone_quote() {
    let src = "y: '\n";

    let (tx, _rx) = unbounded();
    let db = InputDatabase::new(tx);
    let file = File::new(&db, PathBuf::from("test.gin"), src.to_string());

    // This should parse (error is accumulated as diagnostic)
    let _compiled = compile(&db, file);

    // Collect and print diagnostics
    let diagnostics = compile::accumulated::<Symptom>(&db, file);

    // Print each diagnostic
    for diagnostic in &diagnostics {
        diagnostic.print(src, "test.gin");
    }
}
