mod support;

use internment::Intern;
use parser::cursor::TokenCursor;
use support::{body, check, flaw_codes};
use typecheck::FileId;
use typecheck::transform::{PackageTransformOptions, TransformCtx, transform_package};

#[test]
fn explicit_pointer_types_with_equal_pointees_share_representation() {
    let typed = check(&format!(
        "{}left(value Word) @Word: return @value\nright(value Word) @Word: return @value\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    let left = &typed.exprs.ty[body(&typed, "left").as_usize()];
    let right = &typed.exprs.ty[body(&typed, "right").as_usize()];

    assert_eq!(
        typecheck::representation::Repr::derive(left, Some(&typed.type_registry)),
        typecheck::representation::Repr::derive(right, Some(&typed.type_registry))
    );
}

#[test]
fn safe_reference_alias_preserves_nominal_identity_and_generic_referent() {
    let typed = check(&format!(
        "{}RefAlias(x) is ref x\nAlias is RefAlias(Word)\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    let canonical = &typed.tags[&typecheck::TagId(Intern::from_ref("RefAlias"))].resolved_ty;
    let alias = &typed.tags[&typecheck::TagId(Intern::from_ref("Alias"))].resolved_ty;
    let word = &typed.tags[&typecheck::TagId(Intern::from_ref("Word"))].resolved_ty;

    assert_eq!(canonical.type_id(), alias.type_id());
    assert_eq!(
        typecheck::ty::reference_pointee_for_type(canonical, Some(&typed.type_registry)).unwrap(),
        typecheck::ty::Ty::Opaque(Intern::from_ref("x"))
    );
    assert_eq!(
        typecheck::ty::reference_pointee_for_type(alias, Some(&typed.type_registry)).unwrap(),
        *word
    );
}

#[test]
fn expected_arguments_and_explicit_pointer_types_materialize_addresses() {
    let typed = check(&format!(
        "{}take(value @Word) Word: return deref value\nmake(value Word) @Word: return @value\nexplicit(value Word) @Word: return @value\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    let pointer_ty = &typed.exprs.ty[body(&typed, "make").as_usize()];
    let explicit_ty = &typed.exprs.ty[body(&typed, "explicit").as_usize()];
    assert_eq!(pointer_ty, explicit_ty);
    assert_eq!(
        typed
            .type_registry
            .resolved_definition_for_type(pointer_ty)
            .pointee_ty(),
        Some(&typed.tags[&typecheck::TagId(Intern::from_ref("Word"))].resolved_ty)
    );
    assert!(!flaw_codes(&typed).contains(&"type-raw-pointer-pointee-mismatch".to_string()));
}

#[test]
fn record_named_pointer_does_not_receive_raw_address_behavior() {
    let typed = check(&format!(
        "{}Pointer has value Word\ntake(value Word) Pointer: return @value\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    let record = &typed.tags[&typecheck::TagId(Intern::from_ref("Pointer"))].resolved_ty;

    assert!(matches!(
        typecheck::representation::Repr::derive(record, Some(&typed.type_registry)),
        Ok(typecheck::representation::Repr::Product { .. })
    ));
    assert!(flaw_codes(&typed).contains(&"type-raw-pointer-requires-expected-type".to_string()));
}

#[test]
fn explicit_pointer_type_is_visible_outside_its_declaration_file() {
    let declarations = TokenCursor::parse_source(test_fixtures::preludes::WORD_PRELUDE);
    let consumer = TokenCursor::parse_source("make(value Word) @Word: return @value\n");
    let outputs = transform_package(
        &[(declarations, FileId(0)), (consumer, FileId(1))],
        &TransformCtx::new(),
        PackageTransformOptions::IDE,
    );

    let pointer = &outputs[1].exprs.ty[body(&outputs[1], "make").as_usize()];
    assert!(matches!(
        typecheck::representation::Repr::derive(pointer, Some(&outputs[1].type_registry)),
        Ok(typecheck::representation::Repr::Address { address_space: 0 })
    ));
}

#[test]
fn cross_module_reference_alias_preserves_nominal_identity() {
    let declarations = TokenCursor::parse_source(&format!(
        "{}RefAlias(x) is ref x\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    let consumer = TokenCursor::parse_source("Alias is RefAlias(Word)\n");
    let outputs = transform_package(
        &[(declarations, FileId(0)), (consumer, FileId(1))],
        &TransformCtx::new(),
        PackageTransformOptions::IDE,
    );

    let canonical = &outputs[0].tags[&typecheck::TagId(Intern::from_ref("RefAlias"))].resolved_ty;
    let alias = &outputs[1].tags[&typecheck::TagId(Intern::from_ref("Alias"))].resolved_ty;

    assert_eq!(canonical.type_id(), alias.type_id());
    assert!(matches!(
        typecheck::ty::reference_pointee_for_type(canonical, Some(&outputs[0].type_registry))
            .unwrap(),
        typecheck::ty::Ty::Opaque(name) if name.as_str() == "x"
    ));
    assert!(matches!(
        typecheck::ty::reference_pointee_for_type(alias, Some(&outputs[1].type_registry)).unwrap(),
        typecheck::ty::Ty::Named { name, .. } if name.as_str() == "Word"
    ));
}

#[test]
fn raw_pointer_does_not_satisfy_safe_reference_contract() {
    let typed = check(&format!(
        "{}make(value Word) @Word: return @value\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    let pointer = &typed.exprs.ty[body(&typed, "make").as_usize()];

    assert!(!pointer.is_ref());
    assert!(!pointer.is_mutable_ref());
}

#[test]
fn user_defined_generic_slice_indexes_through_its_pointer() {
    let typed = check(&format!(
        "{}Count is in 0 to 18446744073709551615\nCount has\n    finish(self):\n        return\nSlice(x) has\n    pointer @x, length Count\n    finish(self):\n        return\nread(value Slice(Byte), index Count) Byte:\n    return value.pointer.[index]\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));

    assert!(
        flaw_codes(&typed).is_empty(),
        "unexpected flaws: {:?}",
        flaw_codes(&typed)
    );
    let expression = body(&typed, "read");
    assert!(matches!(
        typed.exprs.kind[expression.as_usize()],
        typecheck::TypedExprKind::BufGet { .. }
    ));
    assert_eq!(
        typed.exprs.ty[expression.as_usize()],
        typed.tags[&typecheck::TagId(Intern::from_ref("Byte"))].resolved_ty
    );
}

#[test]
fn shared_reference_arguments_allow_aliasing_without_conflict() {
    let typed = check(&format!(
        "{}inspect(left ref{{shared}} x, right ref{{shared}} x): return 0\ncaller(x Word): return inspect(x, x)\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));

    assert!(flaw_codes(&typed).is_empty(), "{:#?}", typed.all_flaws());
}

#[test]
fn shared_and_mut_reference_arguments_conflict_on_aliasing() {
    let typed = check(&format!(
        "{}inspect(left ref{{shared}} x, right mut{{shared}} x): return 0\nmain(left mut Word, right mut Word): return inspect(left, right)\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    let args = (0..typed.exprs.kind.len())
        .find_map(|index| {
            let kind = &typed.exprs.kind[index];
            match kind {
                typecheck::TypedExprKind::FnCall {
                    args: Some(args), ..
                } if args.len() == 2 => Some(args),
                _ => None,
            }
        })
        .unwrap_or_else(|| panic!("did not find typed call to inspect"));
    let arg_groups: Vec<_> = args
        .iter()
        .map(|arg| {
            (
                typed.exprs.facts.ownership_facts.target_group[arg.as_usize()]
                    .as_ref()
                    .map(|group| format!("{:?}", group)),
                format!("{:?}", typed.exprs.ty[arg.as_usize()]),
                format!("{:?}", typed.exprs.kind[arg.as_usize()]),
            )
        })
        .collect();
    assert!(
        arg_groups.iter().all(|(group, ..)| group.is_some()),
        "missing argument target group: {:?}",
        arg_groups,
    );
    assert_eq!(
        flaw_codes(&typed),
        ["type-overlapping-target-group-arguments"],
        "{:#?}",
        typed.all_flaws()
    );
}

#[test]
fn function_reference_return_requires_matching_target_group() {
    let valid = check(&format!(
        "{}selected(value ref{{a}} Word) ref Word: return value\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));

    assert!(flaw_codes(&valid).is_empty(), "{:#?}", valid.all_flaws());

    let invalid = check(&format!(
        "{}wrong(left ref{{a}} Word, right ref{{b}} Word) ref{{a}} Word: return right\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    assert_eq!(
        flaw_codes(&invalid),
        ["type-reference-return-wrong-target-group"],
        "{:#?}",
        invalid.all_flaws()
    );
}

#[test]
fn recursive_type_rejected_when_defined_by_value_only() {
    let typed = check("Node has next Node\n");

    assert!(flaw_codes(&typed).contains(&"recursive-type-representation".to_string()));
}

#[test]
fn recursive_alias_direct_self_reference_is_rejected() {
    let typed = check("Node is Node\n");

    assert!(flaw_codes(&typed).contains(&"recursive-type-representation".to_string()));
}

#[test]
fn recursive_alias_chain_preserves_identity_per_declaration() {
    let typed = check(
        "First is Third\n\
         Third is Second\n\
         Second is First\n",
    );

    assert!(flaw_codes(&typed).contains(&"recursive-type-representation".to_string()));
}

#[test]
fn recursive_type_alias_not_created_for_cross_file_name_aliases() {
    let declarations = TokenCursor::parse_source("Node is in 0 to 255\n");
    let consumer = TokenCursor::parse_source("Alias is Node\n");
    let outputs = transform_package(
        &[(declarations, FileId(0)), (consumer, FileId(1))],
        &TransformCtx::new(),
        PackageTransformOptions::IDE,
    );

    assert!(
        !flaw_codes(&outputs[1])
            .iter()
            .any(|code| code == "recursive-type-representation"),
    );
}

#[test]
fn aliases_in_any_order_do_not_change_recursive_alias_identity() {
    let first = check("A is B\nB is A\n");
    assert!(flaw_codes(&first).contains(&"recursive-type-representation".to_string()));

    let second = check("B is A\nA is B\n");
    assert!(flaw_codes(&second).contains(&"recursive-type-representation".to_string()));
}

#[test]
fn recursive_type_alias_with_reference_former_breaks_cycle() {
    let typed = check(
        "RefAlias(x) is ref x\n\
         Node has next @Node\n\
         RefAliasNode is RefAlias(Node)\n",
    );

    assert!(
        !flaw_codes(&typed)
            .iter()
            .any(|code| code == "recursive-type-representation")
    );
}

#[test]
fn recursive_type_allowed_when_explicit_pointer_breaks_cycle() {
    let typed = check("Node has next @Node\n");

    assert!(
        !flaw_codes(&typed)
            .iter()
            .any(|code| code == "recursive-type-representation"),
    );
}

#[test]
fn recursive_type_allowed_when_reference_wrapper_breaks_the_cycle() {
    let typed = check("Node has next ref Node\n");

    assert!(
        !flaw_codes(&typed)
            .iter()
            .any(|code| code == "recursive-type-representation"),
    );
}

#[test]
fn consuming_one_possible_reference_target_invalidates_selected_union_reference() {
    let typed = check(
        r#"Word is in 0 to 255
         choose(left ref{r} Word, right ref{r} Word) ref{r} Word: return left
         consume(value Word) Word: return value
         consume_ref(value ref Word) Word: return deref value
         main(left own Word, right own Word) Word:
             ref selected Word: choose(left, right)
             consume(left)
             consume(right)
             return consume_ref(selected)

Word has
    finish(self):
        return
"#,
    );
    assert!(
        flaw_codes(&typed)
            .iter()
            .any(|code| code == "type-use-of-invalidated-reference"),
        "unexpected flaws: {:?}",
        typed.all_flaws()
    );
}
