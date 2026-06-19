//! Regression: defs after tuple `when` arms must parse (sized.gin tail).

use internment::Intern;
use parser::query::SourceParseExt;

#[test]
fn sized_gin_includes_tail_helpers() {
    // Parser-only test: type definitions not needed — just the def signatures
    // with list-when and tuple-when bodies followed by more top-level defs.
    let src = "\
Size is Const(BigInt) or Dynamic

compute_size(x) Size := when x is
    Primitive(w, _)     then Const(w / 8)

sum_named(fields) Size := when fields is
    []                  then Const(0)
    [f, ...rest]        then add(compute_size(f.ty), sum_named(rest))

add(a Size, b Size) Size := when (a, b) is
    (Const(x), Const(y)) then Const(x + y)
                         else Dynamic

mul_size(size Size, n) Size := when size is
    Const(x) then Const(x * n)
             else Dynamic

union_size(variants) Size := add(union_disc(variants), union_max_payload(variants))

union_disc(variants) Size := when variants is
    []           then Const(0)
    [_]          then Const(1)
                 else Const(2)

union_max_payload(variants) Size := when variants is
    []              then Const(0)
    [v, ...rest]    then max_size(sum_named(v.fields), union_max_payload(rest))

max_size(a Size, b Size) Size := when (a, b) is
    (Const(x), Const(y)) then Const(when x > y then x else y)
                         else Dynamic
";
    let file = src.parse_source_full().ast;
    let names: Vec<_> = file.defs.keys().map(|k| k.as_str()).collect();
    for name in [
        "union_size",
        "mul_size",
        "union_disc",
        "union_max_payload",
        "max_size",
    ] {
        assert!(
            file.defs.contains_key(&Intern::new(name.to_string())),
            "missing def `{name}`; parsed defs: {names:?}"
        );
    }
}

#[test]
fn tuple_when_with_dedented_else_allows_following_top_level_bind() {
    let src = r#"
Size is Const(BigInt) or Dynamic

add(a Size, b Size) Size := when (a, b) is
    (Const(x), Const(y)) then Const(x + y)
                         else Dynamic

mul_size(size Size, n BigInt) Size := Const(0)
"#;
    let file = src.parse_source_full().ast;
    assert!(
        file.defs.contains_key(&Intern::new("mul_size".to_string())),
        "defs after tuple-when-else: {:?}",
        file.defs.keys().map(|k| k.as_str()).collect::<Vec<_>>()
    );
}

#[test]
fn when_tuple_subject_parses_as_when_not_tuple_of_const() {
    let src = "f() Size := when (a, b) is\n    (Const(x), Const(y)) then Const(0)\n";
    let file = src.parse_source_full().ast;
    let f = file.defs.get(&Intern::new("f".to_string())).unwrap();
    match &f.value {
        ast::BindValue::Expr(e) => match &e.value {
            ast::Expr::When(w) => {
                assert!(w.subject.is_some());
                assert!(!w.arms.is_empty());
            }
            other => panic!("expected When, got {other:?}"),
        },
        _ => panic!("not expr"),
    }
}
