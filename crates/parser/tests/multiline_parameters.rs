use ast::{HasMember, ParameterKind};
use parser::query::SourceParseExt;

#[test]
fn newline_separates_untyped_parameters_from_following_parameters() {
    for receiver in ["self", "self ref", "self mut"] {
        let source = format!(
            "Pool has\n    reclaim(\n        {receiver}\n        element\n        allocation Block(element)\n    )\n"
        );
        let parsed = source.parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:#?}", parsed.symptoms);
        let declaration = parsed.ast.tags.values().next().unwrap();
        let ast::DeclareValue::Has(members) = &declaration.value else {
            panic!("expected members");
        };
        let HasMember::Function(function) = &members[0] else {
            panic!("expected a function");
        };
        let params = &function.params;
        assert_eq!(
            params.keys().map(|name| name.as_str()).collect::<Vec<_>>(),
            ["self", "element", "allocation"]
        );
        assert!(matches!(params[0].kind, ParameterKind::Generic));
        assert!(matches!(params[1].kind, ParameterKind::Generic));
        assert!(matches!(params[2].kind, ParameterKind::Tagged(_)));
    }
}
