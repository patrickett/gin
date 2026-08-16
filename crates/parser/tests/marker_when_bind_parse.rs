//! `:=` binds with multiline `when subject is` type-pattern arms must parse as one expr.

use parser::query::SourceParseExt;

#[test]
fn is_copy_multiline_when_parses_as_single_expr() {
    let src = "\
is_copy(x Type) Bool := when x is
    Primitive(_, _)     then True
    Opaque(_)           then False
";
    let file = src.parse_source_full().ast;
    let bind = file
        .defs
        .get(&internment::Intern::new("is_copy".to_string()))
        .expect("is_copy def");
    assert!(bind.is_constant(), "fn def with `:=` should be constant");
    match &bind.value {
        ast::BindValue::Expr(e) => match &e.value {
            ast::Expr::When(w) => {
                assert!(
                    w.arms.len() >= 2,
                    "expected when arms, got {} arms, expr {:?}",
                    w.arms.len(),
                    e.value
                );
            }
            other => panic!("expected When body, got {other:?}"),
        },
        other => panic!("expected Expr bind, got {other:?}"),
    }
}

#[test]
fn is_copy_all_arms() {
    let src = "\
is_copy(x Type) Bool := when x is
    Primitive(w, _)     then True
    Ptr(_)              then False
    Ref(_, False)       then True
    Ref(_, True)        then False
    Opaque(_)           then False
    Record(_, fields)   then all_named_copy(fields)
    Tuple(elems)        then all_types_copy(elems)
    Union(_, variants)  then all_variants_copy(variants)
    Array(elem, _)      then is_copy(elem)
";
    let file = src.parse_source_full().ast;
    let bind = file
        .defs
        .get(&internment::Intern::new("is_copy".to_string()))
        .unwrap();
    match &bind.value {
        ast::BindValue::Expr(e) => match &e.value {
            ast::Expr::When(w) => assert!(w.arms.len() >= 7, "arms={}", w.arms.len()),
            other => panic!("{other:?}"),
        },
        _ => panic!("not expr"),
    }
}

#[test]
fn copy_gin_with_trait_decl() {
    let src = "\
Copy has can_copy Bool

is_copy(x Type) Bool := when x is
    Primitive(w, _)     then True
    Opaque(_)           then False
";
    let file = src.parse_source_full().ast;
    let bind = file
        .defs
        .get(&internment::Intern::new("is_copy".to_string()))
        .unwrap();
    match &bind.value {
        ast::BindValue::Expr(e) => match &e.value {
            ast::Expr::When(w) => assert!(w.arms.len() >= 2),
            other => panic!("{other:?}"),
        },
        _ => panic!("not expr"),
    }
}

#[test]
fn full_sized_gin_parses_compile_time_helpers() {
    let src = "\
Size is Const(BigInt) or Dynamic
Sized has size Size: compute_size(Self)

compute_size(x) Size := when x is
    Primitive(w, _)     then Const(w / 8)

sum_named(fields) Size := when fields is
    []                  then Const(0)
    [f, ...rest]        then add(compute_size(f.ty), sum_named(rest))

sum_types(elems) Size := when elems is
    []                  then Const(0)
    [t, ...rest]        then add(compute_size(t), sum_types(rest))

add(a Size, b Size) Size := when (a, b) is
    (Const(x), Const(y)) then Const(x + y)
                         else Dynamic
";
    let file = src.parse_source_full().ast;
    for name in ["compute_size", "sum_named", "sum_types", "add"] {
        assert!(
            file.defs
                .contains_key(&internment::Intern::new(name.to_string())),
            "missing def {name}, defs={:?}",
            file.defs.keys().map(|k| k.as_str()).collect::<Vec<_>>()
        );
    }
}

#[test]
fn full_copy_gin_is_copy_body() {
    // Parser-only: no type defs needed — just the `when` body + list-when helpers.
    let src = "\
Copy has can_copy Bool: is_copy(Self)

is_copy(x) Bool := when x is
    Primitive(_, _)     then True
    Ptr(_)              then False
    Ref(_, False)       then True
    Ref(_, True)        then False
    Opaque(_)           then False
    Record(_, fields)   then all_named_copy(fields)

all_named_copy(fields) Bool := when fields is
    []                  then True
    [f, ...rest]        then is_copy(f.ty) and all_named_copy(rest)
";
    let file = src.parse_source_full().ast;
    let bind = file
        .defs
        .get(&internment::Intern::new("is_copy".to_string()))
        .expect("is_copy");
    match &bind.value {
        ast::BindValue::Expr(e) => match &e.value {
            ast::Expr::When(w) => assert!(w.arms.len() >= 5, "arms={}", w.arms.len()),
            other => panic!("got {other:?}"),
        },
        _ => panic!("not expr"),
    }
    assert!(
        file.defs
            .contains_key(&internment::Intern::new("all_named_copy".to_string()))
    );
}
