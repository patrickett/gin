use parser::query::SourceParseExt;

mod common;

use common::assert_roundtrip;

#[test]
fn union_discriminants_survive_formatting_and_reparsing() {
    for source in [
        "Code is Foo:=3 or Bar:=-2 or Baz:=0\n",
        "Code is Foo := 3\n     or Bar := -2\n     or Baz := 0\n",
        "Code is Only := 7\n",
        "#bits(16)\nCode is Foo := 300 or Bar := 2\n",
    ] {
        let formatted = ginfmt::format(source);
        let parsed = formatted.parse_source_full();
        assert!(
            parsed.symptoms.is_empty(),
            "{formatted}: {:?}",
            parsed.symptoms
        );
        let ast::DeclareValue::Union { variants } = &parsed.ast.tags.values().next().unwrap().value
        else {
            panic!()
        };
        assert!(
            variants
                .iter()
                .all(|variant| variant.discriminant().is_some()),
            "{formatted}"
        );
        assert_roundtrip(&formatted);
    }
}
