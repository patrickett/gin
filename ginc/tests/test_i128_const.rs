use ginc::codegen::generate_mlir;
use ginc::parse::parse_from_str;
use ginc::typeck::TyEnv;

#[test]
fn test_i128_constant_not_truncated() {
    // Value: 170141183460469231731687303715884105727 (2^127 - 1)
    let source = "main: 170141183460469231731687303715884105727\n";
    let ast = parse_from_str(source);
    let ty_env = TyEnv::from_file_ast(&ast);

    let (mlir_text, symptoms) = generate_mlir(&ast, source, "test.gin", &ty_env);
    let mlir_text = mlir_text.expect("codegen should succeed");

    // If there are codegen symptoms, print them for debugging
    if !symptoms.is_empty() {
        for s in &symptoms {
            eprintln!("codegen symptom: {s:?}");
        }
    }

    // The MLIR output should contain the full value, not a truncated i64
    assert!(
        mlir_text.contains("170141183460469231731687303715884105727"),
        "i128 constant should not be truncated. MLIR output:\n{mlir_text}"
    );
}

#[test]
fn test_negative_i128_constant() {
    // Value: -20000000000000000000 (-2e19, below i64::MIN of -9223372036854775808)
    // Note: -2^127 can't be used because the lexer parses the positive literal first,
    // and 2^127 overflows i128 (max is 2^127 - 1).
    let source = "main: -20000000000000000000\n";
    let ast = parse_from_str(source);
    let ty_env = TyEnv::from_file_ast(&ast);

    let (mlir_text, symptoms) = generate_mlir(&ast, source, "test.gin", &ty_env);
    let mlir_text = mlir_text.expect("codegen should succeed");

    if !symptoms.is_empty() {
        for s in &symptoms {
            eprintln!("codegen symptom: {s:?}");
        }
    }

    // Unary minus is lowered as `arith.subi(0, val)`, so the positive literal
    // must appear at full precision in the constant, not truncated.
    assert!(
        mlir_text.contains("20000000000000000000 : i128"),
        "Negative i128 constant should not be truncated. MLIR output:\n{mlir_text}"
    );
    assert!(
        mlir_text.contains("arith.subi"),
        "Negation should be lowered via subi. MLIR output:\n{mlir_text}"
    );
}

#[test]
fn test_i64_constant_fast_path() {
    // A value that fits in i64 should still work correctly
    let source = "main: 42\n";
    let ast = parse_from_str(source);
    let ty_env = TyEnv::from_file_ast(&ast);

    let (mlir_text, symptoms) = generate_mlir(&ast, source, "test.gin", &ty_env);
    let mlir_text = mlir_text.expect("codegen should succeed");

    if !symptoms.is_empty() {
        for s in &symptoms {
            eprintln!("codegen symptom: {s:?}");
        }
    }

    assert!(
        mlir_text.contains("42"),
        "i64 constant should be present. MLIR output:\n{mlir_text}"
    );
}
