use ast::{Literal, SpanId, SpanTable, Ty};
use diagnostic::code::CallCode;
use internment::Intern;
use typecheck::FileId;
use typecheck::staging::analyze_availability;
use typecheck::typed::{
    Availability, ConstFacts, DefId, ExprFacts, ExprId, OwnershipFacts, SemanticFacts, TypedExpr,
    TypedExprKind, TypedFileAst,
};

fn push(typed: &mut TypedFileAst, kind: TypedExprKind) -> ExprId {
    let id = ExprId(u32::try_from(typed.exprs.kind.len()).unwrap());
    typed.exprs.push(TypedExpr {
        kind,
        ty: Ty::Unit,
        span: SpanId::INVALID,
        facts: ExprFacts {
            const_facts: ConstFacts {
                const_value: None,
                const_object: None,
                sequence_literal: None,
                literal_materialization: None,
            },
            semantic_facts: SemanticFacts {
                integer_knowledge: None,
                result_evidence: None,
                projected_result_evidence: Default::default(),
                availability: Availability::Unknown,
                flaws: vec![],
            },
            ownership_facts: OwnershipFacts {
                references: Default::default(),
                reference_reads: Default::default(),
                target_group: None,
                result_target_groups: vec![],
                place: None,
                place_version: None,
            },
        },
    });
    id
}

fn binding_availability(initializer: TypedExprKind, unassigned: bool) -> Availability {
    let mut typed = TypedFileAst::new(FileId(0), SpanTable::new());
    let body = push(&mut typed, initializer);
    assert_eq!(body, ExprId(0));
    let binding = push(
        &mut typed,
        TypedExprKind::Bind {
            name: Intern::from_ref("value"),
            stmts: vec![],
            body,
            unassigned,
        },
    );
    analyze_availability(&mut typed, &Default::default());
    typed.exprs.facts.semantic_facts.availability[binding.as_usize()]
}

fn runtime() -> TypedExprKind {
    TypedExprKind::FnCall {
        target: DefId::named(Intern::from_ref("runtime")),
        args: None,
        substituted_ty: None,
    }
}

#[test]
fn initialized_binding_preserves_runtime_initializer_at_zero() {
    assert_eq!(
        binding_availability(runtime(), false),
        Availability::Runtime
    );
}

#[test]
fn initialized_binding_preserves_unknown_initializer_at_zero() {
    assert_eq!(
        binding_availability(TypedExprKind::Invalid, false),
        Availability::Unknown
    );
}

#[test]
fn initialized_binding_preserves_constant_initializer_at_zero() {
    assert_eq!(
        binding_availability(TypedExprKind::Lit(Literal::Int(0.into())), false),
        Availability::CompileTime
    );
}

#[test]
fn unassigned_binding_does_not_read_placeholder_zero() {
    assert_eq!(
        binding_availability(runtime(), true),
        Availability::CompileTime
    );
}

fn call_availability(callee_invalid: bool, argument: TypedExprKind) -> Availability {
    use parser::query::SourceParseExt;
    use typecheck::transform::{TransformCtx, transform};
    let parsed =
        "Byte is in 0 to 255\nByte.finish(self):\n    return\nvalue(x Byte) Byte: return x\n"
            .parse_source_full();
    assert!(parsed.symptoms.is_empty());
    let mut typed = transform(&parsed.ast, FileId(0), &TransformCtx::new());
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let target = DefId::named(Intern::from_ref("value"));
    let bind = typed.defs.get_mut(&target).unwrap();
    bind.call_capability = typecheck::typed::CallCapability::StagePolymorphic;
    bind.effects = Default::default();
    if callee_invalid {
        typed
            .defs
            .get_mut(&target)
            .unwrap()
            .push_flaw(diagnostic::Diagnostic::new(
                CallCode::TestCalleeInvalid,
                "invalid callee",
            ));
    }
    let argument_ty = typed.defs[&target].params[0].1.clone();
    let return_ty = typed.defs[&target].return_type.clone();
    let argument = push(&mut typed, argument);
    typed.exprs.ty[argument.as_usize()] = argument_ty;
    let call = push(
        &mut typed,
        TypedExprKind::FnCall {
            target,
            args: Some(vec![argument]),
            substituted_ty: None,
        },
    );
    typed.exprs.ty[call.as_usize()] = return_ty;
    analyze_availability(&mut typed, &Default::default());
    typed.exprs.facts.semantic_facts.availability[call.as_usize()]
}

#[test]
fn invalid_callee_does_not_become_compile_time_with_a_constant_argument() {
    assert_eq!(
        call_availability(true, TypedExprKind::Lit(Literal::Int(0.into()))),
        Availability::Unknown
    );
}

#[test]
fn valid_callee_preserves_argument_availability() {
    for (argument, expected) in [
        (
            TypedExprKind::Lit(Literal::Int(0.into())),
            Availability::CompileTime,
        ),
        (TypedExprKind::Invalid, Availability::Unknown),
        (runtime(), Availability::Runtime),
    ] {
        assert_eq!(call_availability(false, argument), expected);
    }
}

#[test]
fn runtime_argument_preserves_runtime_requirement_with_an_invalid_callee() {
    assert_eq!(call_availability(true, runtime()), Availability::Runtime);
}

#[test]
fn named_type_argument_keeps_its_declaration_type_without_an_initializer() {
    let mut typed = TypedFileAst::new(FileId(0), SpanTable::new());
    let placeholder = push(&mut typed, TypedExprKind::Lit(Literal::Int(0.into())));
    typed.exprs.ty[placeholder.as_usize()] = Ty::bounded_int(0.into(), 0.into());
    let name = Intern::from_ref("element");
    let ty = Ty::bounded_int(0.into(), 255.into());
    let argument = push(
        &mut typed,
        TypedExprKind::Bind {
            name,
            stmts: vec![],
            body: placeholder,
            unassigned: true,
        },
    );
    typed.exprs.ty[argument.as_usize()] = ty.clone();
    let instantiated = typecheck::staging::instantiate_call(
        &[argument],
        &typed,
        &[ast::ParamKind::Type],
        &[(name, Ty::Opaque(name))],
        &Ty::Opaque(name),
    );
    assert_eq!(instantiated.substituted_ty, Some(ty));
    assert_eq!(instantiated.args, Some(vec![]));
}

#[test]
fn named_value_argument_without_an_initializer_does_not_read_its_placeholder() {
    let mut typed = TypedFileAst::new(FileId(0), SpanTable::new());
    let placeholder = push(&mut typed, TypedExprKind::Lit(Literal::Int(0.into())));
    typed.exprs.ty[placeholder.as_usize()] = Ty::bounded_int(0.into(), 0.into());
    let name = Intern::from_ref("value");
    let ty = Ty::bounded_int(0.into(), 255.into());
    let argument = push(
        &mut typed,
        TypedExprKind::Bind {
            name,
            stmts: vec![],
            body: placeholder,
            unassigned: true,
        },
    );
    typed.exprs.ty[argument.as_usize()] = ty.clone();
    let instantiated = typecheck::staging::instantiate_call(
        &[argument],
        &typed,
        &[ast::ParamKind::Value(Box::new(ty.clone()))],
        &[(name, ty)],
        &Ty::Unit,
    );
    assert_eq!(instantiated.args, Some(vec![argument]));
}

#[test]
fn initialized_named_argument_still_uses_its_initializer() {
    let mut typed = TypedFileAst::new(FileId(0), SpanTable::new());
    let body = push(&mut typed, TypedExprKind::Lit(Literal::Int(7.into())));
    let ty = Ty::bounded_int(0.into(), 255.into());
    typed.exprs.ty[body.as_usize()] = ty.clone();
    let name = Intern::from_ref("value");
    let argument = push(
        &mut typed,
        TypedExprKind::Bind {
            name,
            stmts: vec![],
            body,
            unassigned: false,
        },
    );
    typed.exprs.ty[argument.as_usize()] = ty.clone();
    let instantiated = typecheck::staging::instantiate_call(
        &[argument],
        &typed,
        &[ast::ParamKind::Value(Box::new(ty.clone()))],
        &[(name, ty)],
        &Ty::Unit,
    );
    assert_eq!(instantiated.args, Some(vec![body]));
}
