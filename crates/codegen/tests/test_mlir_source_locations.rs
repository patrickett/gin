//! Integration tests: MLIR ops should carry `FileLineCol` locations back to the `.gin` source.

mod support;

/// Zero-arg callee used so the emitted IR contains a single obvious `func.call`.
const TWO_FN_PROGRAM: &str = "\
noop() Unit:\n\
return\n\
\n\
main:\n\
    noop()\n\
return\n\
";

#[test]
fn func_call_carries_gin_filename_and_call_line_in_loc() {
    let filename = "mlir_source_loc_tracking.gin";
    let (mlir_text, symptoms) = support::codegen_to_mlir_text(TWO_FN_PROGRAM, filename, true);
    assert!(
        symptoms.is_empty(),
        "expected no codegen symptoms: {symptoms:?}"
    );
    assert!(
        mlir_text.contains("callee = @noop"),
        "expected a call to noop; got:\n{mlir_text}"
    );
    assert!(
        mlir_text.contains(&format!(r#"loc("{filename}""#)),
        "expected MLIR loc() to name the Gin file {filename:?}; got:\n{mlir_text}"
    );
    assert!(
        mlir_text.contains(&format!(r#"{filename}":5:"#)),
        "noop() call is on source line 5; expected loc(...:5:...) with that file; got:\n{mlir_text}"
    );
}
