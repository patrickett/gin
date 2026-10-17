mod support;

use ast::ConstValue;
use internment::Intern;
use support::{body, check};

#[test]
fn renamed_fixed_array_former_retains_named_application_and_array_repr() {
    let typed = check(
        "Byte is in 0 to 255\n#fixed_array(element, count)\nLibraryBuffer(element, count BigInt) has\nmake(value Byte) LibraryBuffer(Byte, 4): return [value; 4]\n",
    );
    let result = &typed.exprs.ty[body(&typed, "LibraryBuffer.make").as_usize()];
    let former = &typed.tags[&typecheck::TagId(Intern::from_ref("LibraryBuffer"))].resolved_ty;
    assert_eq!(result.type_id(), former.type_id());
    let declaration = result
        .named_instance_stripping_reference_wrappers()
        .expect("named fixed-array instance")
        .declaration;
    let acceptance = typed
        .type_registry
        .declaration(declaration)
        .and_then(|semantics| semantics.sequence_literal.as_ref())
        .expect("declaration-owned sequence literal acceptance");
    assert_eq!(acceptance.element_parameter.as_str(), "element");
    assert!(matches!(
        acceptance.materialization,
        typecheck::SequenceLiteralMaterialization::FixedArray { length_parameter }
            if length_parameter.as_str() == "count"
    ));
    assert!(
        matches!(
            typed.exprs.facts.const_facts.literal_materialization[body(&typed, "LibraryBuffer.make").as_usize()].as_deref(),
            Some(typecheck::typed::LiteralMaterializationPlan::FixedArray {
                declaration: Some(materializer),
                ..
            }) if *materializer == declaration
        ),
        "{:?}",
        typed.exprs.facts.const_facts.literal_materialization
            [body(&typed, "LibraryBuffer.make").as_usize()]
    );
    assert!(matches!(
        typed.exprs.facts.const_facts.sequence_literal[body(&typed, "LibraryBuffer.make").as_usize()].as_deref(),
        Some(typecheck::typed::TypedSequenceLiteral::Repeat { count, .. })
            if count.as_const_int() == Some(4)
    ));
    assert_eq!(
        typecheck::representation::Repr::derive(result, Some(&typed.type_registry)).ok(),
        Some(typecheck::representation::Repr::Array {
            element: Box::new(typecheck::representation::Repr::Bits { width: 8 }),
            length: 4,
        })
    );
}

#[test]
fn uncontextualized_fixed_array_construction_infers_structural_array() {
    let typed = check(
        "Byte is in 0 to 255\n#fixed_array(element, count)\nLibraryBuffer(element, count BigInt) has\nmake(value Byte): return [value; 4]\n",
    );
    let result = &typed.exprs.ty[body(&typed, "LibraryBuffer.make").as_usize()];
    assert!(matches!(result, typecheck::ty::Ty::Array { .. }));
}

#[test]
fn inferred_repeat_materializes_concrete_seed_and_array_type() {
    let typed = check("inferred: [0; 4]\n");
    let expression = body(&typed, "inferred");
    let ty = &typed.exprs.ty[expression.as_usize()];
    assert!(matches!(ty, typecheck::ty::Ty::Array { elem, size }
        if **elem == typecheck::ty::Ty::i64() && size.as_const_int() == Some(4)));
    let seed = match &typed.exprs.kind[expression.as_usize()] {
        typecheck::TypedExprKind::FixedArrayAlloc { init, size }
            if size.as_const_int() == Some(4) =>
        {
            *init
        }
        _ => panic!("expected a compact inferred repeat"),
    };
    assert_eq!(typed.exprs.ty[seed.as_usize()], typecheck::ty::Ty::i64());
    assert_eq!(
        typecheck::representation::Repr::derive(ty, Some(&typed.type_registry)).ok(),
        Some(typecheck::representation::Repr::Array {
            element: Box::new(typecheck::representation::Repr::Bits { width: 64 }),
            length: 4,
        })
    );
}

#[test]
fn unconstrained_bracket_literal_never_infers_nominal_list() {
    let typed = check("values := [1, 2, 3]\n");
    let expression = body(&typed, "values");
    let ty = &typed.exprs.ty[expression.as_usize()];
    assert!(!matches!(ty, typecheck::ty::Ty::Opaque(name) if name.as_str() == "List"));
    assert!(matches!(ty, typecheck::ty::Ty::Array { size, .. } if size.as_const_int() == Some(3)));
}

#[test]
fn range_iteration_uses_an_explicit_range_plan() {
    let typed = check("main:\n    for value in 0 to 2\n        iteration: 0\n");
    let plan = typed.exprs.kind.iter().find_map(|kind| match kind {
        typecheck::TypedExprKind::Loop(typecheck::TypedLoop {
            kind: typecheck::TypedLoopKind::ForIn { traversal, .. },
            ..
        }) => Some(traversal),
        _ => None,
    });

    assert!(matches!(
        plan,
        Some(typecheck::SequenceTraversal::IntegerRange { .. })
    ));
}

#[test]
fn fixed_array_iteration_preserves_structural_element_and_length() {
    let typed = check("main:\n    for value in [1, 2, 3]\n        iteration: 0\n");
    let plan = typed.exprs.kind.iter().find_map(|kind| match kind {
        typecheck::TypedExprKind::Loop(typecheck::TypedLoop {
            kind: typecheck::TypedLoopKind::ForIn { traversal, .. },
            ..
        }) => Some(traversal),
        _ => None,
    });

    assert!(matches!(
        plan,
        Some(typecheck::SequenceTraversal::Fixed { iterations })
            if iterations.len() == 3 && iterations.iter().enumerate().all(|(index, statements)|
                typed.exprs.ty_of(statements[0]).and_then(|ty| ty.anonymous_integer_validity()).is_some_and(|validity| validity.domain().storage_hull().is_some_and(|hull| hull.min() == ((index + 1) as u64).into() && hull.max() == ((index + 1) as u64).into())))
    ));
}

#[test]
fn empty_sequence_without_context_has_an_empty_shape() {
    let typed = check("empty := []\n");
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(
        matches!(&typed.exprs.ty[body(&typed, "empty").as_usize()], ast::Ty::Sequence(fields) if fields.is_empty())
    );
}

#[test]
fn mixed_sequence_elements_preserve_individual_types() {
    let typed = check("Flag is True or False\nmixed := [1, True]\n");
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(
        matches!(&typed.exprs.ty[body(&typed, "mixed").as_usize()], ast::Ty::Sequence(fields) if fields.len() == 2 && fields[0].ty != fields[1].ty)
    );
}

#[test]
fn symbolic_repeat_without_context_is_rejected() {
    let typed = check("n := 4\nvalues := [0; n]\n");
    let expression = body(&typed, "values");
    assert!(
        typed.exprs.facts.semantic_facts.flaws[expression.as_usize()]
            .iter()
            .any(|flaw| flaw.code.slug() == "type-sequence-repeat-requires-context")
    );
}

#[test]
fn bracket_array_forms_lower_to_structural_arrays() {
    let typed = check(
        "Byte is in 0 to 255\n\
         explicit [Byte; 4] := [1, 2, 3, 4]\n\
         repeated [Byte; 4] := [0; 4]\n",
    );

    for name in ["explicit", "repeated"] {
        let expression = body(&typed, name);
        assert!(matches!(
            typed.exprs.facts.const_facts.literal_materialization[expression.as_usize()].as_deref(),
            Some(typecheck::typed::LiteralMaterializationPlan::FixedArray {
                declaration: None,
                ..
            })
        ));
        assert_eq!(
            typecheck::representation::Repr::derive(
                &typed.exprs.ty[expression.as_usize()],
                Some(&typed.type_registry),
            )
            .ok(),
            Some(typecheck::representation::Repr::Array {
                element: Box::new(typecheck::representation::Repr::Bits { width: 8 }),
                length: 4,
            }),
            "{name}",
        );
    }

    assert!(matches!(
        &typed.exprs.kind[body(&typed, "explicit").as_usize()],
        typecheck::TypedExprKind::ArrayLit(items) if items.len() == 4
    ));
    assert!(matches!(
        &typed.exprs.facts.const_facts.sequence_literal[body(&typed, "explicit").as_usize()].as_deref(),
        Some(typecheck::typed::TypedSequenceLiteral::Elements(items)) if items.len() == 4
    ));
    assert!(matches!(
        &typed.exprs.kind[body(&typed, "repeated").as_usize()],
        typecheck::TypedExprKind::FixedArrayAlloc { .. }
    ));
    assert!(matches!(
        &typed.exprs.facts.const_facts.sequence_literal[body(&typed, "repeated").as_usize()].as_deref(),
        Some(typecheck::typed::TypedSequenceLiteral::Repeat { count, .. })
            if count.as_const_int() == Some(4)
    ));
}

#[test]
fn typed_aggregate_constants_preserve_fixed_array_category() {
    let typed = check(
        "Byte is in 0 to 255\n\
         bytes [Byte; 2] := [1, 2]\n",
    );

    assert!(matches!(
        &typed.exprs.facts.const_facts.const_value[body(&typed, "bytes").as_usize()],
        Some(ConstValue::FixedArray(items)) if items.len() == 2
    ));
    let object = typed.exprs.facts.const_facts.const_object[body(&typed, "bytes").as_usize()]
        .and_then(|id| typed.constant_objects.get(id))
        .expect("fixed-array constant object");
    assert!(matches!(
        &object.value,
        ConstValue::FixedArray(items) if items.len() == 2
    ));
}

#[test]
fn renamed_owned_sequence_uses_declared_builder_calls() {
    let typed = check(
        "Byte is in 0 to 255\n#sequence_builder(x, empty, push)\nBag(x) has\n    empty() Bag(x)\n    push(self Bag(x) own, item x own) Bag(x)\nmake() Bag(Byte): return [1, 2]\nrepeat() Bag(Byte): return [1; 3]\n",
    );
    let expression = body(&typed, "make");
    assert!(matches!(
        &typed.exprs.facts.const_facts.literal_materialization[expression.as_usize()].as_deref(),
        Some(typecheck::typed::LiteralMaterializationPlan::OwnedSequence { .. })
    ));
    assert!(matches!(
        &typed.exprs.kind[expression.as_usize()],
        typecheck::TypedExprKind::FnCall { args: Some(args), .. } if args.len() == 2
    ));
    assert!(typed.exprs.facts.const_facts.const_value[expression.as_usize()].is_none());
    let repeat = body(&typed, "repeat");
    assert!(matches!(
        &typed.exprs.kind[repeat.as_usize()],
        typecheck::TypedExprKind::FnCall { args: Some(args), .. } if args.len() == 2
    ));
    assert!(
        typed.exprs.facts.semantic_facts.flaws[repeat.as_usize()]
            .iter()
            .all(|flaw| {
                flaw.code.slug() != "type-sequence-builder-repeat-requires-constant-count"
            })
    );
}

#[test]
fn owned_repeat_zero_and_one_have_expected_builder_shapes() {
    let typed = check(
        "Byte is in 0 to 255\n#sequence_builder(x, empty, push)\nBag(x) has\n    empty() Bag(x)\n    push(self Bag(x) own, item x own) Bag(x)\nzero() Bag(Byte): return [1; 0]\none() Bag(Byte): return [1; 1]\n",
    );
    let zero = body(&typed, "zero");
    let one = body(&typed, "one");
    assert!(
        matches!(typed.exprs.kind[zero.as_usize()], typecheck::TypedExprKind::FnCall { args: Some(ref args), .. } if args.is_empty())
    );
    assert!(
        matches!(typed.exprs.kind[one.as_usize()], typecheck::TypedExprKind::FnCall { args: Some(ref args), .. } if args.len() == 2)
    );
}

#[test]
fn owned_builder_accepts_named_constant_repeat_count() {
    let typed = check(
        "Byte is in 0 to 255\n#sequence_builder(x, empty, push)\nBag(x) has\n    empty() Bag(x)\n    push(self Bag(x) own, item x own) Bag(x)\nn := 3\nvalues() Bag(Byte): return [1; n]\n",
    );
    let expression = body(&typed, "values");
    assert!(matches!(
        &typed.exprs.kind[expression.as_usize()],
        typecheck::TypedExprKind::FnCall { args: Some(args), .. } if args.len() == 2
    ));
    assert!(
        typed.exprs.facts.semantic_facts.flaws[expression.as_usize()]
            .iter()
            .all(|flaw| flaw.code.slug() != "type-sequence-builder-repeat-requires-constant-count")
    );
}

#[test]
fn owned_builder_rejects_runtime_repeat_count() {
    let typed = check(
        "Byte is in 0 to 255\n#sequence_builder(x, empty, push)\nBag(x) has empty() Bag(x)\nvalues(n Byte) Bag(Byte): return [1; n]\n",
    );
    let expression = body(&typed, "values");
    assert!(
        typed.exprs.facts.semantic_facts.flaws[expression.as_usize()]
            .iter()
            .any(|flaw| {
                flaw.code.slug() == "type-sequence-builder-repeat-requires-constant-count"
            })
    );
}

#[test]
fn owned_repeat_requires_rematerializable_seed_when_duplicated() {
    let typed = check(
        "Byte is in 0 to 255\n#sequence_builder(x, empty, push)\nBag(x) has empty() Bag(x)\nmake() Byte\nvalues() Bag(Byte): return [make(); 2]\n",
    );
    let expression = body(&typed, "values");
    assert!(
        typed.exprs.facts.semantic_facts.flaws[expression.as_usize()]
            .iter()
            .any(|flaw| { flaw.code.slug() == "type-sequence-repeat-seed-not-copyable" })
    );
}

#[test]
fn owned_repeat_once_moves_a_runtime_seed_without_rematerializing_it() {
    let typed = check(
        "Byte is in 0 to 255\n#sequence_builder(x, empty, push)\nBag(x) has\n    empty() Bag(x)\n    push(self Bag(x) own, item x own) Bag(x)\nmake() Byte\nvalues() Bag(Byte): return [make(); 1]\n",
    );
    let expression = body(&typed, "values");
    assert!(
        typed
            .all_flaws()
            .iter()
            .all(|(_, flaw)| { flaw.code.slug() != "type-sequence-repeat-seed-not-copyable" }),
        "{:?}",
        typed.all_flaws()
    );
    let typecheck::TypedExprKind::FnCall {
        args: Some(args), ..
    } = &typed.exprs.kind[expression.as_usize()]
    else {
        panic!("one-element builder call")
    };
    assert_eq!(args.len(), 2);
    assert!(
        matches!(&typed.exprs.kind[args[1].as_usize()], typecheck::TypedExprKind::FnCall { target, .. } if target.0.as_str() == "make")
    );
}
