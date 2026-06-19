//! `:=` binds with multiline `when subject is` type-pattern arms must parse as one expr.

use parser::query::SourceParseExt;
use typecheck::ComptimeClass;
use typecheck::comptime_classify::{BindComptimeExt, FileAstComptimeExt};

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
    assert!(bind.is_constant, "fn def with `:=` should be constant");
    let mut file = file;
    file.apply_comptime_classification();
    let bind = file
        .defs
        .get(&internment::Intern::new("is_copy".to_string()))
        .unwrap();
    assert!(
        bind.is_compile_time,
        "Type-param when body is comptime-classified"
    );
    assert_eq!(bind.classify(&file), ComptimeClass::ComptimeFn);
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
Copy has (can_copy Bool)

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
fn parse_copy_blanket_impl() {
    let src = "x has Copy(can_copy: is_copy(x))\n";
    let file = src.parse_source_full().ast;
    assert_eq!(file.blanket_impls.len(), 1);
    assert_eq!(file.blanket_impls[0].trait_name.as_str(), "Copy");
}

#[test]
fn copy_gin_prefix_has_blanket() {
    let src = "\
use '../reflect/'.(Type, NamedTy, VariantShape, Reflectable)
use '../primitive/'.Bool

Copy has (can_copy Bool)

x has Copy(can_copy: True)
";
    let file = src.parse_source_full().ast;
    assert_eq!(file.blanket_impls.len(), 1);
}

#[test]
fn full_copy_gin_has_blanket_impl() {
    // Parser-only: just needs a blanket impl (`x.Trait(...)`) to parse.
    let src = "\
Bool is True or False

Copy has (can_copy Bool)
x.Copy(can_copy: is_copy(x))

is_copy(x) Bool := True
";
    let file = src.parse_source_full().ast;
    assert!(
        !file.blanket_impls.is_empty(),
        "blankets={:?}",
        file.blanket_impls
            .iter()
            .map(|b| b.trait_name.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn full_sized_gin_parses_compile_time_helpers() {
    // Parser-only: no type defs needed — just a blanket impl + defs with when bodies.
    let src = "\
Size is Const(BigInt) or Dynamic
x.Sized(size: compute_size(x))

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
    assert_eq!(file.blanket_impls.len(), 1);
    assert_eq!(file.blanket_impls[0].type_var.as_str(), "x");
}

#[test]
fn full_copy_gin_is_copy_body() {
    // Parser-only: no type defs needed — just the `when` body + list-when helpers.
    let src = "\
Copy has (can_copy Bool)
x.Copy(can_copy: is_copy(x))

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
