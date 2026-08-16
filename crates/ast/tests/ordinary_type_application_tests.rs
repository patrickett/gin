//! Ordinary expressions in generic type applications.

mod support;

use support::transform_source;
use typecheck::TypedExprKind;
use typecheck::analysis::unify_type_args;
use typecheck::normal_expr::Normalize;
use typecheck::representation::Repr;
use typecheck::ty::Ty;
use typecheck::typed::TagId;
use typecheck::{CompileTimeTraitRegistry, trait_field_for_ty};

#[test]
fn binary_type_argument_resolves_as_array_size() {
    let source = "#fixed_array(x, n)\nBuffer(x Type, n Int) has size Int\nHolder has values Buffer(Int, n + 1)\n";
    let typed = transform_source(source);
    assert!(
        typed.all_flaws().is_empty(),
        "ordinary type argument should resolve without flaws: {:?}",
        typed.all_flaws()
    );

    let holder = typed
        .tags
        .get(&TagId(internment::Intern::from_ref("Holder")))
        .expect("Holder");
    let Ty::Record { fields, .. } = typed
        .type_registry
        .resolved_definition_for_type(&holder.resolved_ty)
    else {
        panic!(
            "Holder should resolve to a record: {:?}",
            holder.resolved_ty
        );
    };
    let values = fields
        .iter()
        .find(|(name, _)| name.as_str() == "values")
        .expect("values field");
    let Ty::Named { instance, name, .. } = values.1.as_ref() else {
        panic!("expected named Buffer type, got {:?}", values.1);
    };

    let expected_buffer_decl = typed
        .tag_types
        .get(&TagId(internment::Intern::from_ref("Buffer")))
        .expect("Buffer");

    assert_eq!(name.as_str(), "Buffer");
    assert_eq!(
        instance.declaration,
        expected_buffer_decl.type_id().unwrap()
    );
    assert_eq!(instance.arguments.len(), 2);
    assert_eq!(instance.arguments[0].0.as_str(), "x");
    assert_eq!(instance.arguments[1].0.as_str(), "n");

    let element = match &instance.arguments[0].1 {
        ast::TyArg::Type(ty) => ty.as_ref().clone(),
        other => panic!("expected type element argument, got {other:?}"),
    };
    let expected_element = typed
        .tag_types
        .get(&TagId(internment::Intern::from_ref("Int")))
        .cloned()
        .unwrap_or_else(|| Ty::Opaque(internment::Intern::from_ref("Int")));
    assert_eq!(element, expected_element);

    let length = match &instance.arguments[1].1 {
        ast::TyArg::Const(expr) => expr,
        other => panic!("expected const length argument, got {other:?}"),
    };

    let former = typed
        .type_registry
        .declaration_for_instance(instance)
        .expect("declaration metadata");
    assert!(former.fixed_array.is_some());
    let former = former
        .fixed_array
        .as_ref()
        .expect("Buffer fixed-array contract");
    assert_eq!(former.element_parameter.as_str(), "x");
    assert_eq!(former.length_parameter.as_str(), "n");

    assert!(length.normalize_to_value().is_none());
    assert_eq!(
        length.normalize(),
        ast::NormalExpr::Add(
            Box::new(ast::NormalExpr::Var(internment::Intern::from_ref("n"))),
            Box::new(1.into()),
        )
    );
    assert!(!matches!(
        Repr::derive(values.1.as_ref(), Some(&typed.type_registry)),
        Ok(Repr::Array { .. })
    ));
}

#[test]
fn symbolic_length_expressions_support_nested_arithmetic() {
    let typed = transform_source(
        "#fixed_array(x, n)\nBuffer(x Type, n Int) has size Int\nHolder has values Buffer(Int, 2 * n + 1)\n",
    );
    assert!(
        typed.all_flaws().is_empty(),
        "ordinary type argument should resolve without flaws: {:?}",
        typed.all_flaws()
    );

    let holder = typed
        .tags
        .get(&TagId(internment::Intern::from_ref("Holder")))
        .expect("Holder");
    let Ty::Record { fields, .. } = typed
        .type_registry
        .resolved_definition_for_type(&holder.resolved_ty)
    else {
        panic!(
            "Holder should resolve to a record: {:?}",
            holder.resolved_ty
        );
    };
    let values = fields
        .iter()
        .find(|(name, _)| name.as_str() == "values")
        .expect("values field");
    let Ty::Named { instance, .. } = values.1.as_ref() else {
        panic!("expected named Buffer type, got {:?}", values.1);
    };
    let expected_buffer_decl = typed
        .tag_types
        .get(&TagId(internment::Intern::from_ref("Buffer")))
        .expect("Buffer");
    assert_eq!(
        instance.declaration,
        expected_buffer_decl.type_id().unwrap()
    );
    assert_eq!(instance.arguments[0].0.as_str(), "x");
    assert_eq!(instance.arguments[1].0.as_str(), "n");
    let length = match &instance.arguments[1].1 {
        ast::TyArg::Const(expr) => expr,
        other => panic!("expected const length argument, got {other:?}"),
    };
    assert!(length.normalize_to_value().is_none());
    assert_eq!(
        length.normalize(),
        ast::NormalExpr::Add(
            Box::new(ast::NormalExpr::Mul(
                Box::new(2.into()),
                Box::new(ast::NormalExpr::Var(internment::Intern::from_ref("n"))),
            )),
            Box::new(1.into()),
        )
    );
    assert!(
        typed
            .type_registry
            .declaration_for_instance(instance)
            .is_some_and(|declaration| declaration.fixed_array.is_some())
    );
}

#[test]
fn symbolic_length_expressions_support_multiplicative_growth() {
    let typed = transform_source(
        "#fixed_array(x, n)\nBuffer(x Type, n Int) has size Int\nHolder has values Buffer(Int, 2 * n)\n",
    );
    assert!(
        typed.all_flaws().is_empty(),
        "ordinary type argument should resolve without flaws: {:?}",
        typed.all_flaws()
    );

    let holder = typed
        .tags
        .get(&TagId(internment::Intern::from_ref("Holder")))
        .expect("Holder");
    let Ty::Record { fields, .. } = typed
        .type_registry
        .resolved_definition_for_type(&holder.resolved_ty)
    else {
        panic!(
            "Holder should resolve to a record: {:?}",
            holder.resolved_ty
        );
    };
    let values = fields
        .iter()
        .find(|(name, _)| name.as_str() == "values")
        .expect("values field");
    let Ty::Named { instance, .. } = values.1.as_ref() else {
        panic!("expected named Buffer type, got {:?}", values.1);
    };
    let expected_buffer_decl = typed
        .tag_types
        .get(&TagId(internment::Intern::from_ref("Buffer")))
        .expect("Buffer");
    assert_eq!(
        instance.declaration,
        expected_buffer_decl.type_id().unwrap()
    );
    assert_eq!(instance.arguments[0].0.as_str(), "x");
    assert_eq!(instance.arguments[1].0.as_str(), "n");
    let length = match &instance.arguments[1].1 {
        ast::TyArg::Const(expr) => expr,
        other => panic!("expected const length argument, got {other:?}"),
    };
    assert!(length.normalize_to_value().is_none());
    assert_eq!(
        length.normalize(),
        ast::NormalExpr::Mul(
            Box::new(2.into()),
            Box::new(ast::NormalExpr::Var(internment::Intern::from_ref("n"))),
        )
    );
    assert!(
        typed
            .type_registry
            .declaration_for_instance(instance)
            .is_some_and(|declaration| declaration.fixed_array.is_some())
    );
}

#[test]
fn unannotated_array_spelling_remains_nominal_record() {
    let typed =
        transform_source("Array(x Type, n Int) has size Int\nHolder has values Array(Int, n)\n");
    assert!(
        typed.all_flaws().is_empty(),
        "ordinary type argument should resolve without flaws: {:?}",
        typed.all_flaws()
    );

    let holder = typed
        .tags
        .get(&TagId(internment::Intern::from_ref("Holder")))
        .expect("Holder");
    let Ty::Record { fields, .. } = typed
        .type_registry
        .resolved_definition_for_type(&holder.resolved_ty)
    else {
        panic!(
            "Holder should resolve to a record: {:?}",
            holder.resolved_ty
        );
    };
    let values = fields
        .iter()
        .find(|(name, _)| name.as_str() == "values")
        .expect("values field");
    let values_ty = values.1.as_ref();

    let Ty::Named { instance, name, .. } = values_ty else {
        panic!("expected named Array type, got {:?}", values_ty);
    };
    let declaration = typed
        .type_registry
        .declaration_for_instance(instance)
        .expect("declaration");

    assert_eq!(name.as_str(), "Array");
    assert!(declaration.fixed_array.is_none());
    assert!(!matches!(
        Repr::derive(values_ty, Some(&typed.type_registry)),
        Ok(Repr::Array { .. })
    ));
    assert!(!typecheck::representation::has_symbolic_fixed_array_length(
        values_ty,
        Some(&typed.type_registry)
    ));
}

#[test]
fn equivalent_symbolic_former_lengths_compare_by_normalized_expression() {
    let typed = transform_source(
        "Array(x Type, n Int) has size Int\nHolderOne is Array(Int, n + 0)\nHolderTwo is Array(Int, 0 + n)\n",
    );
    assert!(
        typed.all_flaws().is_empty(),
        "ordinary type argument should resolve without flaws: {:?}",
        typed.all_flaws()
    );

    let one = typed
        .tags
        .get(&TagId(internment::Intern::from_ref("HolderOne")))
        .expect("HolderOne");
    let two = typed
        .tags
        .get(&TagId(internment::Intern::from_ref("HolderTwo")))
        .expect("HolderTwo");

    let one = one
        .resolved_ty
        .named_instance()
        .and_then(|instance| {
            instance
                .arguments
                .iter()
                .find_map(|(_, argument)| match argument {
                    ast::TyArg::Const(ast::NormalExpr::Inferred(_)) => None,
                    ast::TyArg::Const(expr) => Some(expr.clone()),
                    ast::TyArg::Type(_) => None,
                })
        })
        .or_else(|| match &one.resolved_ty {
            Ty::Array { size, .. } => Some(size.clone()),
            _ => None,
        })
        .unwrap();
    let two = two
        .resolved_ty
        .named_instance()
        .and_then(|instance| {
            instance
                .arguments
                .iter()
                .find_map(|(_, argument)| match argument {
                    ast::TyArg::Const(ast::NormalExpr::Inferred(_)) => None,
                    ast::TyArg::Const(expr) => Some(expr.clone()),
                    ast::TyArg::Type(_) => None,
                })
        })
        .or_else(|| match &two.resolved_ty {
            Ty::Array { size, .. } => Some(size.clone()),
            _ => None,
        })
        .unwrap();

    assert_eq!(one.normalize(), two.normalize());
}

#[test]
fn symbolic_array_type_applications_preserve_alias_identity() {
    let typed = transform_source(
        "Array(x Type, n Int) has size Int\nHolder is Array(Int, 2 + 1)\nAlias is Holder\n",
    );
    assert!(
        typed.all_flaws().is_empty(),
        "ordinary type alias should resolve without flaws: {:?}",
        typed.all_flaws()
    );

    let holder = typed
        .tag_types
        .get(&TagId(internment::Intern::from_ref("Holder")))
        .expect("Holder");
    let alias = typed
        .tag_types
        .get(&TagId(internment::Intern::from_ref("Alias")))
        .expect("Alias");

    assert_eq!(holder.type_id(), alias.type_id());
    assert_eq!(holder, alias);
}

#[test]
fn same_range_declarations_keep_distinct_identity_and_equal_representation() {
    let typed = transform_source(
        "First is in 0...4294967295\nSecond is in 0...4294967295\nAlias is First\n",
    );
    let first = typed
        .tag_types
        .get(&TagId(internment::Intern::from_ref("First")))
        .expect("First");
    let second = typed
        .tag_types
        .get(&TagId(internment::Intern::from_ref("Second")))
        .expect("Second");
    let alias = typed
        .tag_types
        .get(&TagId(internment::Intern::from_ref("Alias")))
        .expect("Alias");

    assert_ne!(first, second);
    assert_eq!(first, alias);
    assert_ne!(first.type_id(), second.type_id());
    assert_eq!(
        Repr::derive(first, Some(&typed.type_registry)),
        Ok(Repr::Bits { width: 32 })
    );
    assert_eq!(
        Repr::derive(first, Some(&typed.type_registry)),
        Repr::derive(second, Some(&typed.type_registry))
    );
    assert!(
        unify_type_args(
            &[ast::TyArg::Type(Box::new(first.clone()))],
            &[ast::TyArg::Type(Box::new(first.clone()))],
        )
        .is_ok()
    );
    assert!(
        unify_type_args(
            &[ast::TyArg::Type(Box::new(first.clone()))],
            &[ast::TyArg::Type(Box::new(second.clone()))],
        )
        .is_err()
    );
}

#[test]
fn trait_lookup_does_not_cross_equal_integer_representations() {
    let source = "\
Int is in 0...4294967295
Flavor has code Int

First is in 0...4294967295
First has Flavor
    Flavor.code: 1

Second is in 0...4294967295
Second has Flavor
    Flavor.code: 2
";
    let ast = parser::parse_from_str(source);
    let registry = CompileTimeTraitRegistry::from_file_ast(&ast);
    let typed = typecheck::transform::transform_file(ast, typecheck::FileId(0));
    let first = typed
        .tag_types
        .get(&TagId(internment::Intern::from_ref("First")))
        .expect("First");
    let second = typed
        .tag_types
        .get(&TagId(internment::Intern::from_ref("Second")))
        .expect("Second");

    assert_eq!(
        trait_field_for_ty("Flavor", "code", first, &typed, &registry),
        Some(ast::ConstValue::Int(1.into()))
    );
    assert_eq!(
        trait_field_for_ty("Flavor", "code", second, &typed, &registry),
        Some(ast::ConstValue::Int(2.into()))
    );
}

#[test]
fn intrinsic_names_and_signatures_are_checked_during_typechecking() {
    let unknown = transform_source(
        "Int is in 0...4294967295\n#intrinsic(NoSuchIntrinsic)\nbad(a Int, b Int) Int extern\n",
    );
    assert!(unknown.all_flaws().iter().any(|(_, flaw)| {
        flaw.code.slug() == "type-unknown-intrinsic" && flaw.arg("name") == Some("NoSuchIntrinsic")
    }));

    let invalid = transform_source("#intrinsic(BitsAdd)\nbad(a String, b String) String extern\n");
    assert!(
        invalid
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-intrinsic-non-bits")
    );
}

#[test]
fn intrinsic_constant_failures_are_expression_diagnostics() {
    let division = transform_source(
        "Wide is in 0...18446744073709551615\n#intrinsic(BitsUnsignedDivide)\ndivide(a Wide, b Wide) Wide extern\nmain := divide(1, 0)\n",
    );
    assert!(
        division.all_flaws().iter().any(|(_, flaw)| {
            flaw.code.slug() == "type-intrinsic-division-by-zero"
                && flaw.arg("intrinsic") == Some("BitsUnsignedDivide")
        }),
        "{:?}",
        division.all_flaws()
    );

    let extension = transform_source(
        "Byte is in 0...255\n#intrinsic(BitsExtendUnsigned)\nextend(a Byte) Byte extern\n",
    );
    assert!(extension.all_flaws().iter().any(|(_, flaw)| {
        flaw.code.slug() == "type-intrinsic-invalid-extension"
            && flaw.arg("source_width") == Some("8")
            && flaw.arg("result_width") == Some("8")
    }));
}

#[test]
fn intrinsic_known_integer_value_is_stored_on_the_expression() {
    let typed = transform_source(
        "Wide is in 0...18446744073709551615\n#intrinsic(BitsAdd)\nadd(a Wide, b Wide) Wide extern\nmain := add(1, 2)\n",
    );
    let (index, _) = typed
        .exprs
        .kind
        .iter()
        .enumerate()
        .find(|(_, kind)| matches!(kind, TypedExprKind::IntrinsicCall { .. }))
        .expect("intrinsic call");

    assert_eq!(
        typed.exprs.const_value[index],
        Some(ast::ConstValue::Int(3.into()))
    );
    assert!(
        typed
            .type_registry
            .integer_validity_for_type(&typed.exprs.ty[index])
            .is_some()
    );
}
