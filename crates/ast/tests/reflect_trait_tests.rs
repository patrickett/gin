//! `Reflectable` synthesis and compile-time trait helpers.

use ast::span::SpanId;
use ast::{ConstValue, ParameterKind, TypeExpr};
use internment::Intern;
use typecheck::analysis::{TyCopyExt, eval_compile_time_expr_with_env, pattern_matches_public};
use typecheck::ty::Ty;
use typecheck::{reflect_ty_to_const_value, trait_field_for_ty};

mod support;
use support::{
    empty_typed, marker_trait_registry, transform_source, transform_with_marker_package,
};

fn intern(s: &str) -> Intern<String> {
    Intern::new(s.to_owned())
}

fn tag_name(cv: &ConstValue) -> Option<&str> {
    match cv {
        ConstValue::Tag { name, .. } => Some(name.as_str()),
        _ => None,
    }
}

#[test]
fn reflect_primitive_int_width() {
    let ty = Ty::Int {
        width: 64,
        signed: false,
        value: None,
        min: None,
        max: None,
    };
    let cv = reflect_ty_to_const_value(&ty);
    assert_eq!(tag_name(&cv), Some("Primitive"));
    let ConstValue::Tag { args, .. } = &cv else {
        panic!("expected tag");
    };
    assert!(matches!(args.first(), Some(ConstValue::Int(64))));
}

#[test]
fn reflect_opaque_is_dynamic_size() {
    use ast::TypeExpr;

    let ty = Ty::Opaque(Intern::new("T".to_string()));
    let cv = reflect_ty_to_const_value(&ty);
    assert_eq!(tag_name(&cv), Some("Opaque"));
    let opaque_pat = TypeExpr::Generic {
        name: Intern::new("Opaque".to_string()),
        params: vec![(Intern::new("_".to_string()), ast::ParameterKind::Generic)],
        param_spans: vec![(Intern::new("_".to_string()), ast::span::SpanId::INVALID)],
        span: ast::span::SpanId::INVALID,
    };
    assert!(pattern_matches_public(&opaque_pat, &cv));
    let registry = marker_trait_registry();
    assert!(registry.trait_in_scope("Sized"));
    assert!(registry.blanket_for("Sized").is_some());
    let blanket = registry.blanket_for("Sized").expect("Sized blanket");
    let (_, field_expr) = blanket
        .fields
        .iter()
        .find(|(n, _)| n.as_str() == "size")
        .expect("size field");
    let field_cv = eval_compile_time_expr_with_env(
        &field_expr.value,
        &std::collections::HashMap::from([(blanket.type_var, Some(cv.clone()))]),
        registry.eval_ast.as_ref(),
    );
    assert!(field_cv.is_some(), "blanket field eval failed");
    let typed = empty_typed();
    let size = trait_field_for_ty("Sized", "size", &ty, &typed, &registry).expect("Sized.size");
    let ConstValue::Tag { name, .. } = &size else {
        panic!("expected Size tag");
    };
    assert_eq!(name.as_str(), "Dynamic");
}

#[test]
fn reflect_record_fields_in_shape() {
    let ty = Ty::Record {
        name: Intern::new("Pair".to_string()),
        fields: vec![(
            Intern::new("a".to_string()),
            Box::new(Ty::Int {
                width: 8,
                signed: false,
                value: None,
                min: None,
                max: None,
            }),
        )],
    };
    let registry = marker_trait_registry();
    let typed = empty_typed();
    let size = trait_field_for_ty("Sized", "size", &ty, &typed, &registry).expect("Sized.size");
    let ConstValue::Tag { name, args, .. } = &size else {
        panic!("expected Size");
    };
    assert_eq!(name.as_str(), "Const");
    assert!(matches!(args.first(), Some(ConstValue::Int(1))));
}

#[test]
fn synthesize_reflectable_on_tag() {
    let source = "\
Point has (x Int, y Int)
";
    let typed = transform_source(source);
    let tag = typed
        .tags
        .get(&typecheck::TagId(Intern::new("Point".to_string())))
        .expect("Point");
    let reflectable = tag
        .provided_traits
        .iter()
        .find(|pt| pt.trait_name.as_str() == "Reflectable")
        .expect("Reflectable synthesized");
    assert!(
        reflectable
            .fields
            .iter()
            .any(|(n, _)| n.as_str() == "shape")
    );
}

#[test]
fn user_reflectable_impl_is_rejected() {
    let source = "\
Bad has (x Int)
Bad.Reflectable(shape: Opaque('hidden'))
";
    let typed = transform_source(source);
    assert!(typed.declaration_flaws.iter().any(|(_, flaw)| {
        flaw.code.slug() == "type-reserved-trait-impl"
            && flaw.arg("trait_name") == Some("Reflectable")
    }));
}

#[test]
fn marker_eval_ast_has_compile_time_defs() {
    let eval = support::marker_trait_registry();
    let names: Vec<_> = eval.eval_ast.defs.keys().map(|k| k.as_str()).collect();
    assert!(
        eval.eval_ast
            .defs
            .contains_key(&Intern::new("is_copy".to_string())),
        "defs: {names:?}"
    );
    let copy_source = "\
Type is Primitive(width BigInt, signed Bool)
     or Record(name String, fields List(NamedTy))
     or Union(name String, variants List(VariantShape))
     or Ptr(inner Type)
     or Opaque(name String)

NamedTy has (name String, ty Type)
VariantShape has (name String, fields List(NamedTy))
Bool is True or False
BigInt is in 0...18446744073709551615
List(x) has (pointer Pointer(x), length BigInt)
String has (bytes List(BigInt))

Copy has (can_copy Bool)
x.Copy(can_copy: is_copy(x))

is_copy(x Type) Bool := when x is
    Primitive(_, _)     then True
    Ptr(_)              then False
    Opaque(_)           then False
    Record(_, fields)   then all_named_copy(fields)
    Union(_, variants)  then all_variants_copy(variants)

all_named_copy(fields List(NamedTy)) Bool := when fields is
    []                  then True
    [f, ...rest]        then is_copy(f.ty) and all_named_copy(rest)

all_variants_copy(variants List(VariantShape)) Bool := when variants is
    []                  then True
    [v, ...rest]        then all_named_copy(v.fields) and all_variants_copy(rest)
";
    let copy_file = parser::cursor::TokenCursor::parse_source(copy_source);
    assert_eq!(
        copy_file.blanket_impls.len(),
        1,
        "copy.gin blankets: {:?}",
        copy_file.blanket_impls[0].trait_name.as_str()
    );
    assert!(
        !eval.blanket_impls.is_empty(),
        "eval blankets: {:?}",
        eval.blanket_impls
            .iter()
            .map(|b| b.trait_name.as_str())
            .collect::<Vec<_>>()
    );
    let sized_source = "\
Type is Primitive(width BigInt, signed Bool)
     or Record(name String, fields List(NamedTy))
     or Union(name String, variants List(VariantShape))
     or Ptr(inner Type)
     or Opaque(name String)

NamedTy has (name String, ty Type)
VariantShape has (name String, fields List(NamedTy))
Bool is True or False
BigInt is in 0...18446744073709551615
List(x) has (pointer Pointer(x), length BigInt)
String has (bytes List(BigInt))

Size is Const(BigInt) or Dynamic
Sized has (size Size)
x.Sized(size: compute_size(x))

compute_size(x Type) Size := when x is
    Primitive(w, _)     then Const(w / 8)
    Ptr(_)              then Const(8)
    Opaque(_)           then Dynamic
    Record(_, fields)   then sum_named(fields)
    Union(_, variants)  then union_size(variants)

sum_named(fields List(NamedTy)) Size := when fields is
    []                  then Const(0)
    [f, ...rest]        then add(compute_size(f.ty), sum_named(rest))

add(a Size, b Size) Size := when (a, b) is
    (Const(x), Const(y)) then Const(x + y)
                         else Dynamic

union_size(variants List(VariantShape)) Size := add(union_disc(variants), union_max_payload(variants))

union_disc(variants List(VariantShape)) Size := when variants is
    []           then Const(0)
    [_]          then Const(1)
                 else Const(2)

union_max_payload(variants List(VariantShape)) Size := when variants is
    []              then Const(0)
    [v, ...rest]    then max_size(sum_named(v.fields), union_max_payload(rest))

max_size(a Size, b Size) Size := when (a, b) is
    (Const(x), Const(y)) then Const(when x > y then x else y)
                         else Dynamic
";
    let sized_only = parser::cursor::TokenCursor::parse_source(sized_source);
    let sized_names: Vec<_> = sized_only.defs.keys().map(|k| k.as_str()).collect();
    assert!(
        sized_only
            .defs
            .contains_key(&Intern::new("compute_size".to_string())),
        "sized.gin defs: {sized_names:?}"
    );
}

#[test]
fn is_copy_bind_parses_when_body() {
    let src = "\
Type is Primitive(width BigInt, signed Bool)
     or Record(name String, fields List(NamedTy))
     or Union(name String, variants List(VariantShape))
     or Ptr(inner Type)
     or Ref(inner Type, mutable Bool)
     or Opaque(name String)

Bool is True or False
BigInt is in 0...18446744073709551615

Copy has (can_copy Bool)
x.Copy(can_copy: is_copy(x))

is_copy(x Type) Bool := when x is
    Primitive(_, _)     then True
    Ptr(_)              then False
    Ref(_, False)       then True
    Ref(_, True)        then False
    Opaque(_)           then False
    Record(_, fields)   then all_named_copy(fields)
    Union(_, variants)  then all_variants_copy(variants)

all_named_copy(fields List(NamedTy)) Bool := when fields is
    []                  then True
    [f, ...rest]        then is_copy(f.ty) and all_named_copy(rest)

all_variants_copy(variants List(VariantShape)) Bool := when variants is
    []                  then True
    [v, ...rest]        then all_named_copy(v.fields) and all_variants_copy(rest)
";
    let file = parser::cursor::TokenCursor::parse_source(src);
    let bind = file
        .defs
        .get(&Intern::new("is_copy".to_string()))
        .expect("is_copy");
    match &bind.value {
        ast::BindValue::Expr(e) => match &e.value {
            ast::Expr::When(w) => assert!(w.arms.len() >= 7, "arms: {}", w.arms.len()),
            other => panic!("got {other:?}"),
        },
        _ => panic!("not expr"),
    }
}

#[test]
fn bool_is_copy_via_blanket() {
    let registry = marker_trait_registry();
    let bind = registry
        .eval_ast
        .defs
        .get(&Intern::new("is_copy".to_string()))
        .expect("is_copy bind");
    assert!(
        bind.is_constant,
        "is_copy should be a constant fn def (`:=`)"
    );
    assert!(
        bind.is_compile_time,
        "is_copy should be comptime-classified after prepare/classify"
    );
    match &bind.value {
        ast::BindValue::Expr(e) => match &e.value {
            ast::Expr::When(w) => {
                assert!(!w.arms.is_empty(), "is_copy when has no arms: {:?}", w.arms)
            }
            other => panic!("is_copy body not when: {other:?}"),
        },
        other => panic!("is_copy not expr: {other:?}"),
    }
    let ty = Ty::Union {
        name: Intern::from_ref("Bool"),
        variants: vec![
            (Intern::from_ref("True"), vec![]),
            (Intern::from_ref("False"), vec![]),
        ],
        literal_values: None,
    };
    let typed = empty_typed();
    let shape = reflect_ty_to_const_value(&ty);
    let param = bind
        .params
        .as_ref()
        .and_then(|p| p.keys().next().copied())
        .expect("is_copy param");
    let copy_source = "\
Type is Primitive(width BigInt, signed Bool)
     or Record(name String, fields List(NamedTy))
     or Union(name String, variants List(VariantShape))
     or Ptr(inner Type)
     or Opaque(name String)

NamedTy has (name String, ty Type)
VariantShape has (name String, fields List(NamedTy))
Bool is True or False
BigInt is in 0...18446744073709551615
List(x) has (pointer Pointer(x), length BigInt)
String has (bytes List(BigInt))

Copy has (can_copy Bool)
x.Copy(can_copy: is_copy(x))

is_copy(x Type) Bool := when x is
    Primitive(_, _)     then True
    Ptr(_)              then False
    Opaque(_)           then False
    Record(_, fields)   then all_named_copy(fields)
    Union(_, variants)  then all_variants_copy(variants)

all_named_copy(fields List(NamedTy)) Bool := when fields is
    []                  then True
    [f, ...rest]        then is_copy(f.ty) and all_named_copy(rest)

all_variants_copy(variants List(VariantShape)) Bool := when variants is
    []                  then True
    [v, ...rest]        then all_named_copy(v.fields) and all_variants_copy(rest)
";
    let copy_only = parser::cursor::TokenCursor::parse_source(copy_source);
    // Reflect Bool and verify the reflected shape matches one of the is_copy patterns.
    use ast::WhenArm;
    if let ast::BindValue::Expr(e) = &bind.value
        && let ast::Expr::When(w) = &e.value
    {
        let env = std::collections::HashMap::from([(param, Some(shape.clone()))]);
        let sub = w.subject.as_ref().expect("subject");
        let sub_cv =
            eval_compile_time_expr_with_env(&sub.value, &env, &copy_only).expect("is_copy subject");
        let matched = w.arms.iter().any(|arm| {
            matches!(arm, WhenArm::Is { pattern, .. } if pattern_matches_public(&pattern.value, &sub_cv))
        });
        assert!(matched, "no is_copy arm matched {sub_cv:?}");
    }
    assert!(
        !registry.blanket_impls.is_empty(),
        "blankets: {:?}",
        registry
            .blanket_impls
            .iter()
            .map(|b| b.trait_name.as_str())
            .collect::<Vec<_>>()
    );
    let copy_blanket = registry
        .blanket_impls
        .iter()
        .find(|b| b.trait_name.as_str() == "Copy")
        .expect("Copy blanket");
    let (_, field_expr) = copy_blanket
        .fields
        .iter()
        .find(|(n, _)| n.as_str() == "can_copy")
        .expect("can_copy field");
    let cv_field = eval_compile_time_expr_with_env(
        &field_expr.value,
        &std::collections::HashMap::from([(copy_blanket.type_var, Some(shape.clone()))]),
        registry.eval_ast.as_ref(),
    );
    assert!(cv_field.is_some(), "blanket field eval failed");
    let cv = trait_field_for_ty("Copy", "can_copy", &ty, &typed, &registry).expect("Copy.can_copy");
    let ConstValue::Tag { name, .. } = &cv else {
        panic!("expected Bool tag");
    };
    assert_eq!(name.as_str(), "True");
}

#[test]
fn copy_override_contradicts_structure() {
    let source = "\
Int is in 1...400

AllInts has (x Int, y Int)
AllInts.Copy(can_copy: False)
";
    let typed = transform_with_marker_package(source);
    let registry = marker_trait_registry();
    let ty = typed
        .tag_types
        .get(&typecheck::TagId(Intern::new("AllInts".to_string())))
        .expect("AllInts");
    let cv = trait_field_for_ty("Copy", "can_copy", ty, &typed, &registry).expect("Copy.can_copy");
    let ConstValue::Tag { name, .. } = &cv else {
        panic!("expected Bool tag");
    };
    assert_eq!(name.as_str(), "False");
    assert!(!ty.is_copyable(&registry, &typed));
}

#[test]
fn parse_blanket_impl() {
    let source = "\
x.Sized(size: Const(0))
";
    let file = parser::cursor::TokenCursor::parse_source(source);
    assert_eq!(file.blanket_impls.len(), 1);
    assert_eq!(file.blanket_impls[0].trait_name.as_str(), "Sized");
    assert_eq!(file.blanket_impls[0].type_var.as_str(), "x");
}

#[test]
fn list_pattern_matches_empty_and_cons() {
    use ast::TypeExpr;

    let empty_pat = TypeExpr::ListEmpty;
    assert!(pattern_matches_public(
        &empty_pat,
        &ConstValue::List(vec![].into())
    ));

    let cons_pat = TypeExpr::ListCons {
        head: Box::new(ast::Spanned {
            value: TypeExpr::Nominal(Intern::new("Const".to_string()), ast::span::SpanId::INVALID),
            span_id: ast::span::SpanId::INVALID,
        }),
        tail: Box::new(ast::Spanned {
            value: TypeExpr::ListEmpty,
            span_id: ast::span::SpanId::INVALID,
        }),
    };
    let cv = ConstValue::List(
        vec![ConstValue::Tag {
            name: Intern::new("Const".to_string()),
            qual_path: None,
            args: vec![ConstValue::Int(8)].into(),
        }]
        .into(),
    );
    assert!(pattern_matches_public(&cons_pat, &cv));
}

#[test]
fn gin_sized_nested_record_matches_rust_layout() {
    let ty = Ty::Record {
        name: Intern::new("Nested".to_string()),
        fields: vec![(
            Intern::new("inner".to_string()),
            Box::new(Ty::Record {
                name: Intern::new("Inner".to_string()),
                fields: vec![(
                    Intern::new("x".to_string()),
                    Box::new(Ty::Int {
                        width: 64,
                        signed: true,
                        value: None,
                        min: None,
                        max: None,
                    }),
                )],
            }),
        )],
    };
    let registry = marker_trait_registry();
    let typed = empty_typed();
    let size = trait_field_for_ty("Sized", "size", &ty, &typed, &registry).expect("Sized.size");
    let ConstValue::Tag { name, args, .. } = &size else {
        panic!("expected Const Size");
    };
    assert_eq!(name.as_str(), "Const");
    assert!(
        matches!(args.first(), Some(ConstValue::Int(8))),
        "got {size:?}"
    );
}

// ── Pattern-match lowering (for compile-time `when subject is`) ────────

#[test]
fn bool_reflects_as_union() {
    let ty = Ty::Union {
        name: Intern::from_ref("Bool"),
        variants: vec![
            (Intern::from_ref("True"), vec![]),
            (Intern::from_ref("False"), vec![]),
        ],
        literal_values: None,
    };
    let cv = reflect_ty_to_const_value(&ty);
    let pat = TypeExpr::Generic {
        name: intern("Union"),
        params: vec![
            (intern("_"), ParameterKind::Generic),
            (intern("_"), ParameterKind::Generic),
        ],
        param_spans: vec![
            (intern("_"), SpanId::INVALID),
            (intern("_"), SpanId::INVALID),
        ],
        span: SpanId::INVALID,
    };
    assert!(pattern_matches_public(&pat, &cv));
}

#[test]
fn record_pattern_matches_record_reflect_shape() {
    let ty = Ty::Record {
        name: intern("R"),
        fields: vec![(
            intern("a"),
            Box::new(Ty::Int {
                width: 8,
                signed: false,
                value: None,
                min: None,
                max: None,
            }),
        )],
    };
    let cv = reflect_ty_to_const_value(&ty);
    let pat = TypeExpr::Generic {
        name: intern("Record"),
        params: vec![
            (intern("_"), ParameterKind::Generic),
            (intern("fields"), ParameterKind::Generic),
        ],
        param_spans: vec![
            (intern("_"), SpanId::INVALID),
            (intern("fields"), SpanId::INVALID),
        ],
        span: SpanId::INVALID,
    };
    assert!(
        pattern_matches_public(&pat, &cv),
        "Record(_, fields) should match reflect Record shape: {cv:?}"
    );
}
