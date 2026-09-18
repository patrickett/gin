use ast::Expr;
use parser::query::SourceParseExt;

#[test]
fn parses_assembly_as_an_expression_with_named_operands() {
    let parsed = "f(value Int) Int: #asm('spec', value: value)".parse_source_full();
    assert!(
        parsed.symptoms.is_empty(),
        "parse errors: {:?}",
        parsed.symptoms
    );
    let bind = parsed
        .ast
        .defs
        .values()
        .find(|bind| bind.name.as_str() == "f")
        .unwrap();
    let ast::BindValue::Expr(body) = &bind.value else {
        panic!("expected expression body");
    };
    let Expr::Asm {
        specification,
        operands,
    } = &body.value
    else {
        panic!("expected asm expression");
    };
    assert!(
        matches!(specification.value, Expr::Lit(ast::Literal::String(ref value)) if value == "spec")
    );
    assert_eq!(operands.len(), 1);
    assert_eq!(operands[0].name.as_str(), "value");
}

#[test]
fn parses_reserved_target_expression() {
    let parsed = "f() Int: #target.arch".parse_source_full();
    assert!(
        parsed.symptoms.is_empty(),
        "parse errors: {:?}",
        parsed.symptoms
    );
    let bind = parsed
        .ast
        .defs
        .values()
        .find(|bind| bind.name.as_str() == "f")
        .unwrap();
    let ast::BindValue::Expr(body) = &bind.value else {
        panic!("expected expression body");
    };
    assert!(matches!(body.value, Expr::RecordGet { ref field, .. } if field.as_str() == "arch"));
}

#[test]
fn parses_multiline_asm_with_trailing_separator() {
    let parsed = "write(fd Int, buf @Byte, len Int) Int:\n    return #asm(\n        protocol,\n        fd: fd,\n        buf: buf,\n        len: len,\n    )".parse_source_full();
    assert!(
        parsed
            .symptoms
            .iter()
            .all(|symptom| symptom.code.slug() == "parse-unnecessary-comma"),
        "parse errors: {:?}",
        parsed.symptoms
    );
}
