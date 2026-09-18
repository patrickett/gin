use parser::query::SourceParseExt;
use typecheck::FileId;
use typecheck::transform::transform_file;

#[test]
fn named_value_and_nested_constructor_alternatives() {
    let source = "Word is in 0 to 255\nOutcome is Word or Failure(Empty or Full)\nchoose(value Word) Outcome: value\nempty Outcome: Failure(Empty)\nfull Outcome: Failure(Full)\n";
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let typed = transform_file(parsed.ast, FileId(0));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn excluding_constructor_exposes_the_remaining_named_value() {
    let source = "Word is in 0 to 255\nOutcome is Word or Failure(Empty or Full)\nrecover(value Outcome) Word: when value is Failure then 0 else value\n";
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let typed = transform_file(parsed.ast, FileId(0));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(
        typed
            .exprs
            .kind
            .iter()
            .any(|kind| matches!(kind, typecheck::TypedExprKind::UnionPayload { .. }))
    );
}

#[test]
fn success_cannot_be_used_without_excluding_failure() {
    for body in [
        "value",
        "value as Word",
        "when value is Failure then value else 0",
    ] {
        let source = format!(
            "Word is in 0 to 255\nOutcome is Word or Failure(Empty or Full)\nbad(value Outcome) Word: {body}\n"
        );
        let parsed = source.parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let typed = transform_file(parsed.ast, FileId(0));
        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, flaw)| flaw.code.slug() == "type-incompatible-union-value"),
            "{:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn narrowed_success_remains_linear() {
    let source = "Word is in 0 to 255\nOutcome is Word or Failure(Empty or Full)\npair(first Word, second Word) Word:\n    discard is in 0 to 255: first\nreturn second\nbad(value Outcome) Word: when value is Failure then 0 else pair(value, value)\n";
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let typed = transform_file(parsed.ast, FileId(0));
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug().contains("move")),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn integer_literal_selects_only_an_unambiguous_member() {
    for (alternatives, valid) in [
        ("Word or Failure(Empty or Full)", true),
        ("Word or Other or Failure(Empty or Full)", false),
    ] {
        let source = format!(
            "Word is in 0 to 255\nOther is in 0 to 255\nOutcome is {alternatives}\nvalue Outcome: 42\n"
        );
        let parsed = source.parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let typed = transform_file(parsed.ast, FileId(0));
        assert_eq!(
            typed.all_flaws().is_empty(),
            valid,
            "{:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn shared_constructor_labels_use_the_subjects_payload_type() {
    let source = "Word is in 0 to 255\nFirst is Word or Failure(Empty or Full)\nSecond is Word or Failure(Small or Large)\ninspect(value First) Word: when value is\n    Failure(reason) then when reason is Empty then 1 else 2\n    Word(number) then number\n";
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let typed = transform_file(parsed.ast, FileId(0));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let reason = typed.exprs.kind.iter().position(|kind| matches!(kind, typecheck::TypedExprKind::FnCall { target, args: None, .. } if target.0.as_str() == "reason")).unwrap();
    let ast::Ty::Union { variants, .. } = &typed.exprs.ty[reason] else {
        panic!("expected the payload union")
    };
    assert_eq!(
        variants
            .iter()
            .map(|variant| variant.name.as_str())
            .collect::<Vec<_>>(),
        ["Empty", "Full"]
    );
}
