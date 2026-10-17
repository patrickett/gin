use ast::Expr;
use parser::query::SourceParseExt;

#[test]
fn rejects_removed_assembly_expressions() {
    for source in [
        "f(value Int) Int: return #asm('spec', value: value)",
        "f(): return #asm('spec')",
    ] {
        let parsed = source.parse_source_full();
        assert!(
            parsed
                .symptoms
                .iter()
                .any(|diagnostic| { diagnostic.code.slug() == "parse-expected-query-name" }),
            "{:?}",
            parsed.symptoms
        );
    }
}

#[test]
fn parses_reserved_target_expression() {
    let parsed = "f Int: #target.arch".parse_source_full();
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
fn rejects_removed_machine_and_register_syntax() {
    for source in [
        "#machine(region)\nwrite(value Word) Word: return value\n",
        "#machine\nwrite: 0\n",
        "write: #register(9)\n",
    ] {
        let parsed = source.parse_source_full();
        assert!(
            !parsed.symptoms.is_empty(),
            "accepted removed syntax: {source}"
        );
    }
}
