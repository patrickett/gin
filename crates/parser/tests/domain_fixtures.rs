use ast::{
    DeclarationKind, DomainBase, DomainKeyword, Expr, Item, RefinementPredicate, SourceFile,
    SyntaxNodeKind, TypeBody, TypeDomain,
};

const INT: &str = include_str!("../../test-fixtures/fixtures/int_prelude.gin");
const BYTE: &str = include_str!("../../test-fixtures/fixtures/byte_prelude.gin");
const HEAP: &str = include_str!("../../test-fixtures/fixtures/box_storage_heap_backend.gin");

fn domain(file: &SourceFile) -> &TypeDomain {
    let Item::Type(declaration) = &file.items[0].value else {
        panic!("fixture begins with a type");
    };
    let TypeBody::Domain(domain) = &declaration.body.value else {
        panic!("fixture retains a domain");
    };
    &domain.value
}

fn check_range<'a>(file: &'a SourceFile, lower_text: &str, upper_text: &str) -> &'a TypeDomain {
    let domain = domain(file);
    let source = file.text.as_ref();
    assert_eq!(
        file.span_table
            .get(domain.in_keyword.unwrap())
            .extract(source),
        "in"
    );
    let DomainBase::Range {
        lower,
        to_keyword,
        upper,
    } = &domain.base.value
    else {
        panic!("written range");
    };
    assert_eq!(
        file.span_table.get(lower.span_id).extract(source),
        lower_text
    );
    assert_eq!(file.span_table.get(*to_keyword).extract(source), "to");
    assert_eq!(
        file.span_table.get(upper.span_id).extract(source),
        upper_text
    );
    domain
}

fn check_queries(file: &SourceFile, name: &str) {
    let source = file.text.as_ref();
    for (word, keyword) in [
        ("in", DomainKeyword::In),
        ("to", DomainKeyword::To),
        ("and", DomainKeyword::And),
    ] {
        for (offset, _) in source.match_indices(&format!(" {word} ")) {
            let hit = file.node_at_offset(offset + 1).unwrap();
            assert!(
                matches!(hit.node.kind, SyntaxNodeKind::DomainKeyword(found) if found == keyword)
            );
            assert_eq!(hit.node.span.extract(source), word);
            assert_eq!(hit.declarations[0].name, name);
        }
    }
    let entry = file.declaration_outline().next().unwrap();
    assert_eq!(entry.kind, DeclarationKind::Type);
    assert_eq!(entry.name, name);
    assert_eq!(entry.name_span.extract(source), name);
    assert_eq!(entry.span.extract(source), source.lines().next().unwrap());
}

#[test]
fn original_signed_int_declaration_keeps_sign_separate_from_digits() {
    // The rest of int_prelude uses the unsupported dotted method declaration form.
    let source = INT.split_inclusive('\n').next().unwrap();
    assert_eq!(
        source,
        "Int is in -9223372036854775808 to 9223372036854775807\n"
    );
    let file = parser::parse_strict(source).unwrap();
    assert_eq!(file.text.as_ref(), source);
    assert_eq!(file.items.len(), 1);
    let domain = check_range(&file, "-9223372036854775808", "9223372036854775807");
    assert!(domain.refinements.is_empty());
    let DomainBase::Range { lower, .. } = &domain.base.value else {
        unreachable!()
    };
    let Expr::Unary { operator, operand } = &lower.value else {
        panic!("sign must not be folded into a literal");
    };
    assert_eq!(file.span_table.get(operator.span_id).extract(source), "-");
    assert_eq!(
        file.span_table.get(operand.span_id).extract(source),
        "9223372036854775808"
    );
    assert!(matches!(operand.value, Expr::Literal(_)));
    let hit = file.node_at_offset(source.find('-').unwrap()).unwrap();
    assert_eq!(hit.node.span.extract(source), "-");
    assert_eq!(hit.declarations[0].name, "Int");
    check_queries(&file, "Int");
}

#[test]
fn complete_byte_prelude_keeps_domain_and_extension_outline() {
    let file = parser::parse_strict(BYTE).expect("complete small real fixture");
    assert_eq!(file.text.as_ref(), BYTE);
    assert_eq!(file.items.len(), 2);
    assert!(check_range(&file, "0", "255").refinements.is_empty());
    check_queries(&file, "Byte");
    let outline: Vec<_> = file
        .declaration_outline()
        .map(|entry| (entry.name, entry.kind))
        .collect();
    assert_eq!(
        outline,
        [
            ("Byte", DeclarationKind::Type),
            ("Byte", DeclarationKind::Type)
        ]
    );
}

#[test]
fn original_mapped_address_retains_written_alignment_predicate() {
    // The complete heap fixture also contains unsupported ownership and control flow.
    let source = HEAP
        .split_inclusive('\n')
        .find(|line| line.starts_with("MappedAddress is "))
        .unwrap();
    assert_eq!(
        source,
        "MappedAddress is in 0 to 18446744073709551615 and self % 4096 = 0\n"
    );
    let file = parser::parse_strict(source).unwrap();
    assert_eq!(file.text.as_ref(), source);
    assert_eq!(file.items.len(), 1);
    let domain = check_range(&file, "0", "18446744073709551615");
    assert_eq!(domain.refinements.len(), 1);
    let clause = &domain.refinements[0].value;
    assert_eq!(
        file.span_table.get(clause.and_keyword).extract(source),
        "and"
    );
    let RefinementPredicate::Expression(expression) = &clause.predicate.value else {
        panic!("explicit self predicate");
    };
    assert_eq!(
        file.span_table.get(expression.span_id).extract(source),
        "self % 4096 = 0"
    );
    let Expr::Binary {
        left,
        operator,
        right,
    } = &expression.value
    else {
        panic!("written equality");
    };
    assert_eq!(file.span_table.get(operator.span_id).extract(source), "=");
    assert_eq!(file.span_table.get(right.span_id).extract(source), "0");
    let Expr::Binary {
        left,
        operator,
        right,
    } = &left.value
    else {
        panic!("written remainder");
    };
    assert_eq!(file.span_table.get(left.span_id).extract(source), "self");
    assert_eq!(file.span_table.get(operator.span_id).extract(source), "%");
    assert_eq!(file.span_table.get(right.span_id).extract(source), "4096");
    check_queries(&file, "MappedAddress");
    let hit = file.node_at_offset(source.find("self").unwrap()).unwrap();
    assert!(matches!(hit.node.kind, SyntaxNodeKind::Name("self")));
    assert_eq!(hit.declarations[0].name, "MappedAddress");
}
