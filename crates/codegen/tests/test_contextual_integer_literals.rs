#[allow(dead_code)]
mod support;

#[test]
fn materialized_constants_use_the_selected_bits_width() {
    for (width, max) in [
        (8, "255"),
        (16, "65535"),
        (32, "4294967295"),
        (127, "170141183460469231731687303715884105727"),
    ] {
        let source = format!("Selected is in 0...{max}\nvalue Selected: 42\n");
        let (mlir, symptoms) = support::codegen_to_mlir_text(&source, "literal.gin", false);

        assert!(
            symptoms.is_empty(),
            "unexpected i{width} symptoms: {symptoms:?}"
        );
        assert!(
            mlir.contains(&format!("42 : i{width}")),
            "expected i{width} constant, got:\n{mlir}"
        );
    }
}

#[test]
fn unresolved_integer_literal_becomes_implicit_anonymous_type() {
    let (verified, symptoms) = support::codegen_verifies("value: 42\n", "literal.gin");

    assert!(
        verified,
        "contextual integer literal should now lower to implicit anonymous type"
    );
    assert!(
        !symptoms
            .iter()
            .any(|diagnostic| diagnostic.code.slug() == "codegen-internal"),
        "unexpected internal codegen diagnostic: {symptoms:?}"
    );
}
