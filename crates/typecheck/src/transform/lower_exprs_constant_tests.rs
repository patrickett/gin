use super::*;

fn constructor_constant(arguments: &str) -> Option<ConstValue> {
    let source = format!(
        "Bit is in 0 to 1\nChoice is Triple(first Bit, second Bit, third Bit) or Empty\nmake(value Bit) Choice: return Triple({arguments})\n"
    );
    constant_for_source(&source)
}

fn constant_for_source(source: &str) -> Option<ConstValue> {
    let parsed = parser::parse_from_str(source);
    assert!(
        parsed.parse_warnings.is_empty(),
        "{:?}",
        parsed.parse_warnings
    );
    let typed = super::super::transform(&parsed, crate::FileId(0), &TransformCtx::new());
    let constants = super::super::constant_queries::ConstantQueries::new(std::sync::Arc::clone(
        &typed.eval_ast,
    ));
    const_for_def_id(
        DefId::named(Intern::from_ref("make")),
        &typed,
        &constants,
        None,
    )
}

#[test]
fn unknown_constructor_arguments_cannot_produce_a_partial_constant() {
    for arguments in ["value, 0, 1", "0, value, 1", "0, 1, value"] {
        assert_eq!(constructor_constant(arguments), None, "{arguments}");
    }
}

#[test]
fn known_constructor_arguments_preserve_their_order() {
    let Some(ConstValue::Tag { name, args, .. }) = constructor_constant("1, 0, 1") else {
        panic!("complete constructor constant")
    };
    assert_eq!(name, Intern::from_ref("Triple"));
    assert_eq!(
        args.as_ref(),
        &[
            ConstValue::Int(1.into()),
            ConstValue::Int(0.into()),
            ConstValue::Int(1.into())
        ]
    );
}

#[test]
fn zero_argument_constructor_is_still_constant() {
    let source = "Choice is Empty or Other\nmake() Choice: return Empty\n";
    let Some(ConstValue::Tag { name, args, .. }) = constant_for_source(source) else {
        panic!("zero-argument constructor constant")
    };
    assert_eq!(name, Intern::from_ref("Empty"));
    assert!(args.is_empty());
}
