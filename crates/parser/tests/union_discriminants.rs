use ast::DeclareValue;
use parser::query::SourceParseExt;

#[test]
fn explicit_discriminants_parse_in_single_and_multiline_unions() {
    for source in [
        "Code is Foo := 3 or Bar := 2 or Baz := 1\n",
        "Code is Foo := 3\n     or Bar := -2\n     or Baz := 0\n",
        "Code is\n    Foo := 3\n    or Bar := -2\n",
        "Code is Only := 7\n",
        "Code is Only := 0xffff_ffff_ffff_ffff_ffff_ffff_ffff_ffff\n",
    ] {
        let parsed = source.parse_source_full();
        assert!(
            parsed.symptoms.is_empty(),
            "{source}: {:?}",
            parsed.symptoms
        );
        let declaration = parsed.ast.tags.values().next().unwrap();
        let DeclareValue::Union { variants } = &declaration.value else {
            panic!("expected union")
        };
        assert!(
            variants
                .iter()
                .all(|variant| variant.discriminant().is_some())
        );
    }
}

#[test]
fn discriminants_require_integer_literals() {
    for value in ["1.5", "'ready'", "other", "1 + 2", "", "-other"] {
        let source = format!("Code is Foo := {value} or Bar := 2\n");
        let parsed = source.parse_source_full();
        assert!(!parsed.symptoms.is_empty(), "accepted {source}");
    }
}

#[test]
fn variant_documentation_preserves_discriminants() {
    let source = "Code is Foo := 3 --- first\n     or Bar := 2 --- second\n";
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let DeclareValue::Union { variants } = &parsed.ast.tags.values().next().unwrap().value else {
        panic!()
    };
    assert_eq!(variants[0].discriminant().unwrap().value, 3.into());
    assert_eq!(variants[1].discriminant().unwrap().value, 2.into());
}
