use super::*;
use crate::FileId;
use crate::test_support::push_expr;
use ast::{Literal, SpanTable};

fn walk_binding(
    unassigned: bool,
    constant_body: bool,
    stale: bool,
) -> (Option<ConstValue>, Option<ConstValue>, Option<bool>) {
    let mut typed = TypedFileAst::new(FileId(0), SpanTable::new());
    let body = push_expr(
        &mut typed,
        if constant_body {
            TypedExprKind::Lit(Literal::Int(7.into()))
        } else {
            TypedExprKind::Invalid
        },
    );
    assert_eq!(body, ExprId(0));
    if constant_body {
        typed.exprs.facts.const_facts.const_value[body.index()] = Some(ConstValue::Int(7.into()));
    }
    let name = Intern::from_ref("value");
    let binding = push_expr(
        &mut typed,
        TypedExprKind::Bind {
            name,
            stmts: vec![],
            body,
            unassigned,
        },
    );
    let mut flow = FlowContext::new();
    let mut references = RefTracker::new();
    let mut constants = HashMap::new();
    let mut values = HashMap::new();
    if stale && unassigned {
        typed.exprs.facts.const_facts.const_value[binding.index()] =
            Some(ConstValue::Int(41.into()));
    }
    if stale {
        constants.insert(name, true);
        values.insert(name, ConstValue::Int(41.into()));
    }
    let local_types = HashMap::new();
    let shared = HashMap::new();
    let signatures = CallableSignatureScope::new(&shared, []);
    WalkExprCtx {
        owned_parameters: Vec::new(),
        local_scope: None,
        local_ownership: HashMap::new(),
        result_exits: Default::default(),
        payload_inputs: Default::default(),
        when_subjects: Vec::new(),
        next_field_snapshot: 0,
        definition: None,
        terminal_receiver: None,
        projecting: false,
        replacing: None,
        is_constant_definition: false,
        typed: &mut typed,
        flow_ctx: &mut flow,
        local_var_types: &local_types,
        ref_tracker: &mut references,
        const_bindings: &mut constants,
        local_const_values: &mut values,
        callable_signatures: &signatures,
    }
    .walk(binding);
    (
        typed.exprs.const_value_of(binding).cloned().flatten(),
        values.get(&name).cloned(),
        constants.get(&name).copied(),
    )
}

#[test]
fn unassigned_binding_does_not_inherit_placeholder_constant() {
    assert_eq!(walk_binding(true, true, false), (None, None, None));
}

#[test]
fn initialized_binding_tracks_initializer_constant_at_zero() {
    let value = Some(ConstValue::Int(7.into()));
    assert_eq!(
        walk_binding(false, true, false),
        (value.clone(), value, Some(true))
    );
}

#[test]
fn nonconstant_binding_removes_shadowed_constant_facts() {
    assert_eq!(walk_binding(false, false, true), (None, None, None));
}

#[test]
fn unassigned_binding_clears_cached_and_shadowed_constants() {
    assert_eq!(walk_binding(true, true, true), (None, None, None));
}
