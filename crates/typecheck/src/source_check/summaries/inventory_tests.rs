use super::*;

#[test]
fn capture_uses_first_scalar_alias_but_never_anonymous_identity() {
    let source = parser::parse_strict("").unwrap();
    let mut checker = Checker::new(&source, crate::Limits::default());
    let scalar = checker
        .fresh_value(Value::integer(I256::ZERO, I256::ONE).ty)
        .unwrap();
    checker.proof.names.extend([
        ("first", scalar),
        ("alias", scalar),
        ("anonymous", Value::unit()),
    ]);
    checker.guard_integer(scalar, I256::ONE).unwrap();
    let inputs = checker.summary_inputs().unwrap();
    let summary = checker.summarize(&inputs, Some(scalar)).unwrap();
    assert_eq!(summary.roots, [0, 1, 2]);
    assert_eq!(summary.outputs[0].input, Some(0));
    assert_eq!(summary.integer_guards, [(0, I256::ONE)]);
    assert_eq!(summary.dispositions, [Disposition::Scalar; 3]);

    let anonymous = checker.summarize(&inputs, Some(Value::unit())).unwrap();
    assert_eq!(anonymous.outputs[0].input, None);
    let actual = checker.fresh_value(scalar.ty).unwrap();
    let returned = checker
        .apply_outcomes(&[summary], &[actual, actual, Value::unit()], &[])
        .unwrap();
    assert_eq!(returned.identity, actual.identity);
    assert_eq!((returned.lower, returned.upper), (I256::ONE, I256::ONE));
}

#[test]
fn sparse_capture_is_breadth_first_and_returning_root_closes_over_children() {
    let source = parser::parse_strict("").unwrap();
    let mut checker = Checker::new(&source, crate::Limits::default());
    let generic = SourceType::Generic { index: 0 };
    let inner_ty = checker.array_type(generic, 100).unwrap();
    let outer_ty = checker.array_type(inner_ty, 100).unwrap();
    let outer = checker.input_value(outer_ty).unwrap();
    let inner = checker.array_child(outer, 7).unwrap();
    checker.array_child(inner, 42).unwrap();
    let scalar = checker
        .fresh_value(Value::integer(I256::ZERO, I256::ONE).ty)
        .unwrap();
    checker
        .proof
        .names
        .extend([("outer", outer), ("scalar", scalar)]);
    let inputs = checker.summary_inputs().unwrap();
    let summary = checker.summarize(&inputs, Some(outer)).unwrap();
    assert_eq!(summary.roots, [0, 1]);
    assert_eq!(summary.inputs.len(), 4);
    assert_eq!(summary.inputs[0].children, [2]);
    assert_eq!(summary.inputs[0].slots, [7]);
    assert_eq!(summary.inputs[2].children, [3]);
    assert_eq!(summary.inputs[2].slots, [42]);
    assert_eq!(
        summary.dispositions,
        [
            Disposition::Returned,
            Disposition::Scalar,
            Disposition::Returned,
            Disposition::Returned,
        ]
    );
    assert_eq!(summary.outputs.len(), 1);
    assert_eq!(summary.outputs[0].input, Some(0));
    assert!(summary.outputs[0].children.is_empty());

    // Rebind from another original root. Paths outside the captured graph
    // belong to that root too; forwarding must not replace its inventory.
    let actual = checker.input_value(outer_ty).unwrap();
    let actual_inner = checker.array_child(actual, 7).unwrap();
    let actual_child = checker.array_child(actual_inner, 42).unwrap();
    let extra = checker.array_child(actual, 99).unwrap();
    let returned = checker
        .apply_outcomes(&[summary], &[actual, scalar], &[generic])
        .unwrap();
    assert_eq!(returned.owner, actual.owner);
    assert_eq!(returned.identity, actual.identity);
    assert_eq!(checker.proof.owners[actual.owner.unwrap()].slots, [7, 99]);
    assert_eq!(
        checker.array_child(returned, 99).unwrap().identity,
        extra.identity
    );
    assert_eq!(
        checker.array_child(actual_inner, 42).unwrap().identity,
        actual_child.identity
    );
}

#[test]
fn fresh_sparse_outputs_attach_positions_without_reusing_captured_values() {
    let source = parser::parse_strict("").unwrap();
    let mut checker = Checker::new(&source, crate::Limits::default());
    let scalar_ty = Value::integer(I256::ZERO, I256::ONE).ty;
    let inner_ty = checker.array_type(scalar_ty, 100).unwrap();
    let outer_ty = checker.array_type(inner_ty, 100).unwrap();
    let outer = checker.fresh_value(outer_ty).unwrap();
    let inner = checker.array_child(outer, 7).unwrap();
    let scalar = checker.array_child(inner, 42).unwrap();
    let inputs = checker.summary_inputs().unwrap();
    let summary = checker.summarize(&inputs, Some(outer)).unwrap();
    assert!(summary.inputs.is_empty());
    assert_eq!(summary.outputs.len(), 3);
    assert_eq!(summary.outputs[0].children, [1]);
    assert_eq!(summary.outputs[0].slots, [7]);
    assert_eq!(summary.outputs[1].children, [2]);
    assert_eq!(summary.outputs[1].slots, [42]);
    assert!(summary.outputs.iter().all(|output| output.input.is_none()));

    let returned = checker.apply_summary(&summary, &[], &[]).unwrap();
    assert_ne!(returned.identity, outer.identity);
    let owner = &checker.proof.owners[returned.owner.unwrap()];
    assert_eq!(owner.slots, [7]);
    assert!(!owner.materialized);
    assert!(owner.live);
    let returned_inner = checker.array_child(returned, 7).unwrap();
    assert_ne!(returned_inner.identity, inner.identity);
    assert_eq!(
        checker.proof.owners[returned_inner.owner.unwrap()].slots,
        [42]
    );
    let returned_scalar = checker.array_child(returned_inner, 42).unwrap();
    assert_ne!(returned_scalar.identity, scalar.identity);
    assert_eq!(returned_scalar.integer_domain, scalar.integer_domain);
}

#[test]
fn opaque_forwarding_preserves_fresh_empty_and_unmaterialized_records() {
    let source = parser::parse_strict(concat!(
        "Empty has\n",
        "Pair has left Empty, right Empty\n",
        "make() Pair: return Pair(left: Empty(), right: Empty())\n",
        "lazy(address @Pair) Pair: return #unchecked(address as Pair)\n",
        "identity(element Type, value element) element: return value\n",
    ))
    .unwrap();
    let library = check_module("library", &source, &[], crate::Limits::default()).unwrap();
    let source = parser::parse_strict(concat!(
        "use library Pair, make, lazy, identity\n",
        "made() Pair: return identity(Pair, make())\n",
        "rebuilt(address @Pair) Pair: return identity(Pair, lazy(address))\n",
    ))
    .unwrap();
    let client = check_module("client", &source, &[&library], crate::Limits::default()).unwrap();
    for (module, made, rebuilt) in [(&library, "make", "lazy"), (&client, "made", "rebuilt")] {
        let outputs = &module.summaries(made)[0].outputs;
        assert_eq!(outputs.len(), 3);
        assert_eq!(outputs[0].children, [1, 2]);
        assert!(outputs.iter().all(|output| output.slots.is_empty()));
        assert!(outputs.iter().all(|output| output.materialized));
        assert!(outputs.iter().all(|output| output.input.is_none()));
        assert!(outputs[1].children.is_empty());
        assert!(outputs[2].children.is_empty());

        let outputs = &module.summaries(rebuilt)[0].outputs;
        assert_eq!(outputs.len(), 1);
        assert!(!outputs[0].materialized);
        assert!(outputs[0].children.is_empty());
        assert!(outputs[0].input.is_none());
    }
}
