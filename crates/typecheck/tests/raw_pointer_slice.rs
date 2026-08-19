use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::transform::{PackageTransformOptions, TransformCtx, transform, transform_package};
use typecheck::{BindBody, DefId, FileId, TypedFileAst};

fn check(source: &str) -> TypedFileAst {
    transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    )
}

fn body(typed: &TypedFileAst, name: &str) -> typecheck::ExprId {
    match &typed.defs[&DefId(Intern::new(name.to_string()))].body {
        BindBody::Expr(expr) => *expr,
        BindBody::Body { exprs, ret } => ret.or_else(|| exprs.last().copied()).unwrap(),
        BindBody::Extern => panic!("expected a body"),
    }
}

fn flaw_codes(typed: &TypedFileAst) -> Vec<String> {
    typed
        .all_flaws()
        .into_iter()
        .map(|(_, flaw)| flaw.code.slug().to_string())
        .collect()
}

#[test]
fn explicit_pointer_types_preserve_the_pointee_argument() {
    let typed = check(
        "Word is in 0...255\n\
         make(value Word) @Word: @value\n",
    );
    let result = &typed.exprs.ty[body(&typed, "make").as_usize()];

    assert_eq!(
        typed
            .type_registry
            .resolved_definition_for_type(result)
            .pointee_ty(),
        Some(&typed.tags[&typecheck::TagId(Intern::from_ref("Word"))].resolved_ty)
    );
    assert!(!flaw_codes(&typed).contains(&"type-raw-pointer-pointee-mismatch".to_string()));
}

#[test]
fn explicit_pointer_types_with_equal_pointees_share_representation() {
    let typed = check(
        "Word is in 0...255\n\
         left(value Word) @Word: @value\n\
         right(value Word) @Word: @value\n",
    );
    let left = &typed.exprs.ty[body(&typed, "left").as_usize()];
    let right = &typed.exprs.ty[body(&typed, "right").as_usize()];

    assert_eq!(
        typecheck::representation::Repr::derive(left, Some(&typed.type_registry)),
        typecheck::representation::Repr::derive(right, Some(&typed.type_registry))
    );
}

#[test]
fn safe_reference_alias_preserves_nominal_identity_and_generic_referent() {
    let typed = check(
        "Word is in 0...255\n\
         RefAlias(x) is ref x\n\
         Alias is RefAlias(Word)\n",
    );
    let canonical = &typed.tags[&typecheck::TagId(Intern::from_ref("RefAlias"))].resolved_ty;
    let alias = &typed.tags[&typecheck::TagId(Intern::from_ref("Alias"))].resolved_ty;
    let word = &typed.tags[&typecheck::TagId(Intern::from_ref("Word"))].resolved_ty;

    assert_eq!(canonical.type_id(), alias.type_id());
    assert_eq!(
        typecheck::ty::reference_referent(canonical),
        &typecheck::ty::Ty::Opaque(Intern::from_ref("x"))
    );
    assert_eq!(typecheck::ty::reference_referent(alias), word);
}

#[test]
fn expected_arguments_and_explicit_pointer_types_materialize_addresses() {
    let typed = check(
        "Word is in 0...255\n\
         take(value @Word) Word: deref value\n\
         make(value Word) @Word: @value\n\
         explicit(value Word) @Word: @value\n",
    );
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
    let typed = check(
        "Word is in 0...255\n\
         Pointer has value Word\n\
         take(value Word) Pointer: @value\n",
    );
    let record = &typed.tags[&typecheck::TagId(Intern::from_ref("Pointer"))].resolved_ty;

    assert!(matches!(
        typecheck::representation::Repr::derive(record, Some(&typed.type_registry)),
        Ok(typecheck::representation::Repr::Product { .. })
    ));
    assert!(flaw_codes(&typed).contains(&"type-raw-pointer-requires-expected-type".to_string()));
}

#[test]
fn explicit_pointer_type_is_visible_outside_its_declaration_file() {
    let declarations = TokenCursor::parse_source("Word is in 0...255\n");
    let consumer = TokenCursor::parse_source("make(value Word) @Word: @value\n");
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
    let declarations = TokenCursor::parse_source("Word is in 0...255\nRefAlias(x) is ref x\n");
    let consumer = TokenCursor::parse_source("Alias is RefAlias(Word)\n");
    let outputs = transform_package(
        &[(declarations, FileId(0)), (consumer, FileId(1))],
        &TransformCtx::new(),
        PackageTransformOptions::IDE,
    );

    let canonical = &outputs[0].tags[&typecheck::TagId(Intern::from_ref("RefAlias"))].resolved_ty;
    let alias = &outputs[1].tags[&typecheck::TagId(Intern::from_ref("Alias"))].resolved_ty;

    assert_eq!(canonical.type_id(), alias.type_id());
    assert_eq!(
        typecheck::ty::reference_referent(canonical),
        typecheck::ty::reference_referent(alias),
    );
}

#[test]
fn raw_pointer_does_not_satisfy_safe_reference_contract() {
    let typed = check("Word is in 0...255\nmake(value Word) @Word: @value\n");
    let pointer = &typed.exprs.ty[body(&typed, "make").as_usize()];

    assert!(!pointer.is_ref());
    assert!(!pointer.is_mutable_ref());
}

#[test]
fn shared_reference_arguments_allow_aliasing_without_conflict() {
    let typed = check(
        "inspect(ref{shared} left x, ref{shared} right x): 0\n\
         caller(mut x Word): inspect(ref x, ref x)\n",
    );

    assert!(
        !flaw_codes(&typed)
            .iter()
            .any(|code| code == "type-overlapping-target-group-arguments"),
    );
}

#[test]
fn shared_and_mut_reference_arguments_conflict_on_aliasing() {
    let typed = check(
        "inspect(ref{shared} left x, mut{shared} right x): 0\n\
         caller(mut x Word): inspect(ref x, mut x)\n",
    );
    let call = typed
        .defs
        .get(&typecheck::DefId(Intern::from_ref("caller")))
        .unwrap();
    let caller_body = match &call.body {
        typecheck::BindBody::Expr(expr) => *expr,
        typecheck::BindBody::Body { exprs, ret } => *ret
            .as_ref()
            .or_else(|| exprs.last())
            .unwrap_or(&typecheck::ExprId(0)),
        typecheck::BindBody::Extern => typecheck::ExprId(0),
    };
    let kind = &typed.exprs.kind[caller_body.as_usize()];
    let args = match kind {
        typecheck::TypedExprKind::FnCall {
            args: Some(args), ..
        } => args,
        _ => panic!("did not find call body kind"),
    };
    let arg_groups: Vec<_> = args
        .iter()
        .map(|arg| {
            (
                typed.exprs.target_group[arg.as_usize()]
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
    assert!(flaw_codes(&typed).contains(&"type-overlapping-target-group-arguments".to_string()));
}

#[test]
fn function_reference_return_requires_matching_target_group() {
    let valid = check("selected(ref{a} value Int) ref Int: value\n");

    assert!(
        !flaw_codes(&valid)
            .iter()
            .any(|code| code == "type-reference-return-wrong-target-group"),
    );
    assert!(
        !flaw_codes(&valid)
            .iter()
            .any(|code| code == "type-reference-return-local-target"),
    );

    let invalid = check("wrong(ref{a} left Int, ref{b} right Int) ref{a} Int: right\n");
    assert!(flaw_codes(&invalid).contains(&"type-reference-return-wrong-target-group".to_string()));
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
fn recursive_alias_mutual_references_are_rejected_by_named_identity() {
    let typed = check(
        "Node is Alias\n\
         Alias is Node\n",
    );

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
fn recursive_type_alias_rejected_by_named_identity_when_not_pointer() {
    let typed = check("Node is Node\n");

    assert!(flaw_codes(&typed).contains(&"recursive-type-representation".to_string()));
}

#[test]
fn recursive_type_alias_not_created_for_cross_file_name_aliases() {
    let declarations = TokenCursor::parse_source("Node is in 0...255\n");
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
fn recursive_type_allowed_when_explicit_pointer_field_breaks_the_cycle() {
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
        r#"Word is in 0...255
         choose(ref{r} left Word, ref{r} right Word) ref{r} Word: left
         consume(value Word) Word extern
         consume_ref(ref value Word) Word extern
         main(mut left Word, mut right Word) Word:
             ref selected Word: choose(ref left, ref right)
             consume(left)
             return consume_ref(selected)
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
