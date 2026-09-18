use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{FileId, TagId};

fn transform_source(source: &str) -> typecheck::TypedFileAst {
    transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    )
}

#[test]
fn nested_generic_constructor_uses_declared_return_arguments() {
    let source = "Word is in 0 to 255\n\
        Packet(element Type, count BigInt) has values Array(element, count)\n\
        Envelope(payload Type) has value payload\n\
        make() Envelope(Packet(Word, 2)):\n\
            return Envelope(value: Packet(values: [1, 2]))\n\
        #fixed_array(element, count)\n\
        Array(element Type, count BigInt) has\n";
    let parsed = TokenCursor::parse_source(source);
    assert!(
        parsed.parse_warnings.is_empty(),
        "{:?}",
        parsed.parse_warnings
    );

    let typed = transform_source(source);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let bind = &typed.defs[&typecheck::DefId(Intern::from_ref("make"))];
    let definition = typed
        .type_registry
        .resolved_definition_for_type(&bind.return_type);
    let typecheck::ty::Ty::Record { fields, .. } = definition else {
        panic!("expected envelope: {definition:?}");
    };
    let packet = typed
        .type_registry
        .resolved_definition_for_type(&fields[0].1);
    let typecheck::ty::Ty::Record { fields, .. } = packet else {
        panic!("expected packet: {packet:?}");
    };
    let (element, length) =
        typecheck::representation::Repr::array_parts(&fields[0].1, Some(&typed.type_registry))
            .expect("specialized array field");
    assert_eq!(length, 2);
    assert_eq!(element.type_name().unwrap().as_str(), "Word");
}

#[test]
fn omitted_generic_type_default_resolves_to_the_declared_default() {
    let typed = transform_source(
        "GlobalAllocator has marker Int\n\
         ArenaAllocator has marker Int\n\
         Vec(element Type, allocator: GlobalAllocator) has value element\n\
         Holder has value Vec(Int)\n\
         Explicit has value Vec(Int, allocator: ArenaAllocator)\n\
         Reordered has value Vec(allocator: ArenaAllocator, element: Int)\n",
    );

    assert!(
        typed.all_flaws().is_empty(),
        "defaulted generic application should typecheck: {:?}",
        typed.all_flaws()
    );

    let allocator_for = |owner_name: &str| {
        let owner = typed
            .tags
            .get(&TagId(Intern::new(owner_name.to_string())))
            .expect("owner declaration");
        let typecheck::ty::Ty::Record { fields, .. } = typed
            .type_registry
            .resolved_definition_for_type(&owner.resolved_ty)
        else {
            panic!("owner should resolve to a record: {:?}", owner.resolved_ty);
        };
        let (_, value_ty) = fields
            .iter()
            .find(|(name, _)| name.as_str() == "value")
            .expect("owner.value field");
        let typecheck::ty::Ty::Named { instance, .. } = value_ty.as_ref() else {
            panic!("owner.value should be a named Vec: {value_ty:?}");
        };

        assert_eq!(instance.arguments.len(), 2);
        assert_eq!(instance.arguments[1].0.as_str(), "allocator");
        let ast::TyArg::Type(allocator) = &instance.arguments[1].1 else {
            panic!("Vec allocator argument should be a type");
        };
        let typecheck::ty::Ty::Named { name, .. } = allocator.as_ref() else {
            panic!("Vec allocator should be named: {allocator:?}");
        };
        name.as_str().to_owned()
    };

    assert_eq!(allocator_for("Holder"), "GlobalAllocator");
    assert_eq!(allocator_for("Explicit"), "ArenaAllocator");
    assert_eq!(allocator_for("Reordered"), "ArenaAllocator");
}
