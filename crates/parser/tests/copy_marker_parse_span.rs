use ast::{BindValue, DeclareValue, Expr, HasMember, WhenArm};
use parser::query::SourceParseExt;

const COPY_GIN: &str = r#"#auto
Copy has can_copy Bool: is_copy(Self)

is_copy(x Type) Bool := when x is
    Primitive(_, _)     then True
    Ptr(_)              then False
    Ref(_, False)       then True
    Ref(_, True)        then False
    Opaque(_)           then False
    Record(_, fields)   then all_named_copy(fields)
    Tuple(elems)        then all_types_copy(elems)
    Union(_, variants)  then all_variants_copy(variants)
    Array(elem, _)      then is_copy(elem)

all_named_copy(fields List(NamedTy)) Bool := when fields is
    []                  then True
    [f, ...rest]        then is_copy(f.ty) and all_named_copy(rest)

all_types_copy(elems List(Type)) Bool := when elems is
    []                  then True
    [t, ...rest]        then is_copy(t) and all_types_copy(rest)

all_variants_copy(variants List(VariantShape)) Bool := when variants is
    []                  then True
    [v, ...rest]        then all_named_copy(v.fields) and all_variants_copy(rest)
"#;

#[test]
fn copy_trait_declares_can_copy_field() {
    let source = COPY_GIN;
    let output = source.parse_source_full();
    assert!(
        output.symptoms.is_empty(),
        "copy.gin parse symptoms: {:?}",
        output.symptoms,
    );
    let file = output.ast;
    assert!(
        file.parse_warnings.is_empty(),
        "copy.gin should parse without warnings: {:?}",
        file.parse_warnings
    );
    let copy_decl = file
        .tags
        .get(&internment::Intern::new("Copy".to_string()))
        .unwrap_or_else(|| {
            panic!(
                "Copy tag missing; tags={:?}, defs={:?}",
                file.tags.keys().collect::<Vec<_>>(),
                file.defs.keys().collect::<Vec<_>>(),
            )
        });

    let DeclareValue::Has(members) = &copy_decl.value else {
        panic!(
            "Copy should be `has can_copy Bool`, got {:?}",
            copy_decl.value
        );
    };
    assert!(
        members.iter().any(
            |member| matches!(member, HasMember::Property(property) if property.name.as_str() == "can_copy")
        ),
        "Copy trait should declare can_copy property"
    );
}

#[test]
fn js_style_list_rest_pattern_parses_as_list_cons() {
    let source = r#"
all_named_copy(fields List(NamedTy)) Bool := when fields is
    []                  then True
    [f, ...rest]        then is_copy(f.ty) and all_named_copy(rest)
"#;
    let file = source.parse_source_full().ast;
    assert!(
        file.parse_warnings.is_empty(),
        "unexpected parse warnings: {:?}",
        file.parse_warnings
    );

    let check = file
        .defs
        .get(&internment::Intern::new("all_named_copy".to_string()))
        .expect("all_named_copy bind");
    let BindValue::Expr(expr) = &check.value else {
        panic!(
            "check should parse as expression bind, got {:?}",
            check.value
        );
    };
    let Expr::When(when_expr) = &expr.value else {
        panic!(
            "check body should parse as when expression, got {:?}",
            expr.value
        );
    };

    let Some(WhenArm::Is { pattern, .. }) = when_expr.arms.get(1) else {
        panic!(
            "second arm should be an is-pattern arm: {:?}",
            when_expr.arms
        );
    };
    let ast::Pattern::ListCons { head, tail } = &pattern.value else {
        panic!("expected list cons pattern, got {:?}", pattern.value);
    };

    assert!(matches!(head.value, ast::Pattern::Nominal(name, _) if name.as_str() == "f"));
    assert!(matches!(tail.value, ast::Pattern::Nominal(name, _) if name.as_str() == "rest"));
}
