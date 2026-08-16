//! `:=` constant binds vs `:` rebindable binds; comptime classification decoupled from `:=`.

use flask::CompileTarget;
use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::FileId;
use typecheck::prepare_parse_ast;
use typecheck::transform::{TransformCtx, transform};

fn prepare(source: &str) -> ast::FileAst {
    let mut ast = TokenCursor::parse_source(source);
    let _ = prepare_parse_ast(&mut ast, &CompileTarget::Library);
    ast
}

fn transform_prepared(source: &str) -> typecheck::TypedFileAst {
    let file_ast = prepare(source);
    transform(&file_ast, FileId(0), &TransformCtx::new())
}

#[test]
fn constant_bind_sets_is_constant() {
    let ast = prepare("two := 1 + 1\n");
    let bind = ast.defs.get(&Intern::from_ref("two")).expect("two");
    assert!(bind.is_constant(), "`:=` should set is_constant");
}

#[test]
fn rebindable_colon_not_constant() {
    let ast = prepare("x: 1\n");
    let bind = ast.defs.get(&Intern::from_ref("x")).expect("x");
    assert!(!bind.is_constant(), "`:` should not set is_constant");
}

#[test]
fn main_classified_runtime() {
    let ast = prepare("main:\n  return 0\n");
    let bind = ast.defs.get(&Intern::from_ref("main")).expect("main");
    assert!(!bind.is_constant(), "main is a rebindable runtime entry");
}

#[test]
fn explicit_rebind_of_immutable_binding_is_rejected() {
    let typed = transform_prepared("main:\n  x := 1\n  x:: 2\n  return 0\n");
    assert!(
        typed.all_flaws().iter().any(|(_, f)| {
            f.code.slug() == "type-reassign-constant" && f.arg("name") == Some("x")
        }),
        "rebinding `:=` with `::` should report ReassignConstant: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn colon_creates_a_fresh_shadow() {
    let typed = transform_prepared("main:\n  x := 1\n  x: 2\n  return x\n");
    assert!(
        !typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-reassign-constant")
    );
}

#[test]
fn fresh_shadow_initializer_resolves_before_new_place() {
    let typed = transform_prepared("main:\n  value: 1\n  value: value\n  return value\n");
    let typecheck::BindBody::Body { exprs, .. } =
        &typed.defs[&typecheck::DefId(Intern::from_ref("main"))].body
    else {
        panic!("expected function body");
    };
    let first_place = typed.exprs.place[exprs[0].as_usize()].expect("first place");
    let second_place = typed.exprs.place[exprs[1].as_usize()].expect("second place");
    let typecheck::TypedExprKind::Bind { body, .. } = typed.exprs.kind[exprs[1].as_usize()] else {
        panic!("expected second bind");
    };

    assert_ne!(first_place, second_place);
    assert_ne!(
        typed.places[first_place.0 as usize].binder,
        typed.places[second_place.0 as usize].binder
    );
    assert_eq!(typed.exprs.place[body.as_usize()], Some(first_place));
}

#[test]
fn explicit_rebind_creates_a_new_version_of_the_same_place() {
    let typed = transform_prepared("main:\n  value: 1\n  value:: 2\n  return value\n");
    let typecheck::BindBody::Body {
        exprs,
        ret: Some(returned),
    } = &typed.defs[&typecheck::DefId(Intern::from_ref("main"))].body
    else {
        panic!("expected function body");
    };
    let initial = typed.exprs.place_version[exprs[0].as_usize()].expect("initial version");
    let replacement = typed.exprs.place_version[exprs[1].as_usize()].expect("replacement version");
    let initial_value = &typed.place_versions[initial.0 as usize];
    let replacement_value = &typed.place_versions[replacement.0 as usize];

    assert_ne!(initial, replacement);
    assert_eq!(initial_value.place, replacement_value.place);
    assert!(matches!(
        replacement_value.origin,
        typecheck::PlaceVersionOrigin::Rebind {
            predecessor: Some(version),
            ..
        } if version == initial
    ));
    assert_eq!(
        typed.exprs.place_version[returned.as_usize()],
        Some(replacement)
    );
    assert_ne!(
        initial_value.integer_knowledge,
        replacement_value.integer_knowledge
    );
}

#[test]
fn conditional_rebind_joins_incoming_and_written_versions() {
    let typed = transform_prepared(
        "main(flag Int):\n  value: 0\n  if flag is 0\n    value:: 1\n  return\n  observed: value\n  return observed\n",
    );
    let typecheck::BindBody::Body { exprs, .. } =
        &typed.defs[&typecheck::DefId(Intern::from_ref("main"))].body
    else {
        panic!("expected function body");
    };
    let initial = typed.exprs.place_version[exprs[0].as_usize()].expect("initial version");
    let typecheck::TypedExprKind::Bind { body: observed, .. } =
        typed.exprs.kind[exprs[2].as_usize()]
    else {
        panic!("expected observed bind");
    };
    let joined = typed.exprs.place_version[observed.as_usize()].expect("joined version");
    let typecheck::PlaceVersionOrigin::Join(predecessors) =
        &typed.place_versions[joined.0 as usize].origin
    else {
        panic!("expected join version");
    };

    assert_eq!(predecessors.len(), 2);
    assert!(predecessors.contains(&initial));
    assert!(predecessors.iter().any(|version| matches!(
        typed.place_versions[version.0 as usize].origin,
        typecheck::PlaceVersionOrigin::Rebind { .. }
    )));
}

#[test]
fn loop_rebind_creates_zero_iteration_phi() {
    let typed = transform_prepared(
        "main(flag Int):\n  value: 0\n  while flag is 0\n    value:: 1\n  loop\n  observed: value\n  return observed\n",
    );
    let typecheck::BindBody::Body { exprs, .. } =
        &typed.defs[&typecheck::DefId(Intern::from_ref("main"))].body
    else {
        panic!("expected function body");
    };
    let initial = typed.exprs.place_version[exprs[0].as_usize()].expect("initial version");
    let typecheck::TypedExprKind::Bind { body: observed, .. } =
        typed.exprs.kind[exprs[2].as_usize()]
    else {
        panic!("expected observed bind");
    };
    let phi = typed.exprs.place_version[observed.as_usize()].expect("loop phi");

    let typecheck::PlaceVersionOrigin::LoopPhi {
        incoming,
        backedges,
    } = &typed.place_versions[phi.0 as usize].origin
    else {
        panic!("expected loop phi");
    };
    assert_eq!(*incoming, initial);
    assert_eq!(backedges.len(), 1);
    assert!(matches!(
        typed.place_versions[backedges[0].0 as usize].origin,
        typecheck::PlaceVersionOrigin::Rebind {
            predecessor: Some(version),
            ..
        } if version == phi
    ));
    let component = typed
        .place_version_components
        .iter()
        .find(|component| component.versions.contains(&phi))
        .expect("phi component");
    assert!(component.cyclic);
    assert!(component.versions.contains(&backedges[0]));
}

#[test]
fn acyclic_unannotated_place_contract_unions_all_write_domains() {
    let typed = transform_prepared("main:\n  value: 250\n  value:: 260\n  return value\n");
    let place = typed
        .places
        .iter()
        .find(|place| place.name.as_str() == "value")
        .expect("value place");
    let ast::Ty::AnonymousInteger { validity } = &place.ty else {
        panic!("expected inferred integer contract, got {:?}", place.ty);
    };
    let domain = validity.domain();
    let hull = domain.storage_hull().expect("finite hull");

    assert_eq!(hull.min(), i256::I256::from(250));
    assert_eq!(hull.max(), i256::I256::from(260));
    assert!(domain.contains(i256::I256::from(250)));
    assert!(domain.contains(i256::I256::from(260)));
    assert!(!domain.contains(i256::I256::from(255)));
}

#[test]
fn unannotated_loop_carried_place_requires_explicit_contract() {
    let typed = transform_prepared(
        "main(flag Int):\n  value: 0\n  while flag is 0\n    value:: 1\n  loop\n  return value\n",
    );

    assert!(typed.all_flaws().iter().any(|(_, flaw)| {
        flaw.code.slug() == "type-cyclic-place-requires-explicit-contract"
            && flaw.arg("name") == Some("value")
    }));
}

#[test]
fn explicitly_bounded_loop_carried_place_is_accepted() {
    let typed = transform_prepared(
        "Small is in 0...2\nmain(flag Int):\n  value Small: 0\n  while flag is 0\n    value:: 1\n  loop\n  return value\n",
    );

    assert!(
        !typed.all_flaws().iter().any(|(_, flaw)| {
            flaw.code.slug() == "type-cyclic-place-requires-explicit-contract"
        })
    );
}

#[test]
fn projected_rebind_resolves_a_child_place_and_version() {
    let typed = transform_prepared(
        "Int is in 0...255\nCoord has x Int, y Int\nmain:\n  point: Coord(x: 1, y: 2)\n  point.x:: 10\n  observed: point.x\n  return observed\n",
    );
    let typecheck::BindBody::Body { exprs, .. } =
        &typed.defs[&typecheck::DefId(Intern::from_ref("main"))].body
    else {
        panic!("expected function body");
    };
    let root = typed.exprs.place[exprs[0].as_usize()].expect("root place");
    let field = typed.exprs.place[exprs[1].as_usize()].expect("field place");
    let replacement = typed.exprs.place_version[exprs[1].as_usize()].expect("field version");
    let typecheck::TypedExprKind::Bind { body: observed, .. } =
        typed.exprs.kind[exprs[2].as_usize()]
    else {
        panic!("expected observed bind");
    };

    assert_eq!(typed.places[field.0 as usize].parent, Some(root));
    assert_eq!(
        typed.places[field.0 as usize].projection,
        Some(typecheck::PlaceProjection::Field(0))
    );
    assert!(matches!(
        typed.place_versions[replacement.0 as usize].origin,
        typecheck::PlaceVersionOrigin::Rebind {
            predecessor: Some(_),
            ..
        }
    ));
    assert_eq!(typed.exprs.place[observed.as_usize()], Some(field));
    assert_eq!(
        typed.exprs.place_version[observed.as_usize()],
        Some(replacement)
    );
}

#[test]
fn projected_rebind_cannot_write_through_immutable_root() {
    let typed = transform_prepared(
        "Int is in 0...255\nCoord has x Int\nmain:\n  point := Coord(x: 1)\n  point.x:: 2\n  return 0\n",
    );

    assert!(typed.all_flaws().iter().any(|(_, flaw)| {
        flaw.code.slug() == "type-rebind-immutable" && flaw.arg("name") == Some("point")
    }));
}

#[test]
fn projected_rebind_must_discharge_non_discardable_old_value() {
    let typed = transform_prepared(
        "Int is in 0...255\nPayload has pointer Pointer(Int)\nContainer has payload Payload\nmain:\n  pointer: @1\n  container: Container(payload: Payload(pointer: pointer))\n  container.payload:: Payload(pointer: @2)\n  return 0\n",
    );

    assert!(typed.all_flaws().iter().any(|(_, flaw)| {
        flaw.code.slug() == "type-rebind-would-discard-value"
            && flaw.arg("name") == Some("container")
    }));
}

#[test]
fn conditional_projected_rebind_joins_the_child_versions() {
    let typed = transform_prepared(
        "Int is in 0...255\nCoord has x Int\nmain(flag Int):\n  point: Coord(x: 1)\n  if flag is 0\n    point.x:: 2\n  return\n  observed: point.x\n  return observed\n",
    );
    let typecheck::BindBody::Body { exprs, .. } =
        &typed.defs[&typecheck::DefId(Intern::from_ref("main"))].body
    else {
        panic!("expected function body");
    };
    let typecheck::TypedExprKind::Bind { body: observed, .. } =
        typed.exprs.kind[exprs[2].as_usize()]
    else {
        panic!("expected observed bind");
    };
    let joined = typed.exprs.place_version[observed.as_usize()].expect("field join");
    let typecheck::PlaceVersionOrigin::Join(predecessors) =
        &typed.place_versions[joined.0 as usize].origin
    else {
        panic!("expected field join");
    };

    assert_eq!(predecessors.len(), 2);
    assert!(predecessors.iter().any(|version| matches!(
        typed.place_versions[version.0 as usize].origin,
        typecheck::PlaceVersionOrigin::Projection { .. }
    )));
    assert!(predecessors.iter().any(|version| matches!(
        typed.place_versions[version.0 as usize].origin,
        typecheck::PlaceVersionOrigin::Rebind { .. }
    )));
}

#[test]
fn compound_name_rebind_uses_ordinary_dispatch_and_old_version() {
    let typed = transform_prepared(
        "Word is in 0...255\n#intrinsic(BitsAdd)\nadd_bits(a Word, b Word) Word extern\n#operator(Add)\n#inline\nword_add(a Word, b Word) Word: add_bits(a, b)\nmain(input Word):\n  value Word: input\n  value+: 2\n  return value\n",
    );
    let typecheck::BindBody::Body { exprs, .. } =
        &typed.defs[&typecheck::DefId(Intern::from_ref("main"))].body
    else {
        panic!("expected function body");
    };
    let initial = typed.exprs.place_version[exprs[0].as_usize()].expect("initial version");
    let typecheck::TypedExprKind::Reassign {
        value,
        operator: Some(ast::BinOp::Add),
        ..
    } = typed.exprs.kind[exprs[1].as_usize()]
    else {
        panic!("expected compound rebind");
    };
    let args = match &typed.exprs.kind[value.as_usize()] {
        typecheck::TypedExprKind::FnCall {
            operator_role: Some(ast::OperatorRole::Add),
            args: Some(args),
            ..
        }
        | typecheck::TypedExprKind::IntrinsicCall {
            op: typecheck::intrinsic::IntrinsicOp::BitsAdd,
            args,
        } => args,
        kind => panic!("expected ordinary addition dispatch, got {kind:?}"),
    };
    assert_eq!(args.len(), 2);
    assert_eq!(typed.exprs.place_version[args[0].as_usize()], Some(initial));
}

#[test]
fn const_bind_after_declare_warning() {
    let diags = prepare("val Int\nval := 32\n").parse_warnings;
    assert!(
        diags
            .iter()
            .any(|d| { d.code.slug() == "type-const-bind-after-declare" }),
        "expected ConstBindAfterDeclare warning, got {diags:?}"
    );
}

#[test]
fn compile_time_bind_rejects_runtime_call() {
    let source = "\
runtime(value Type) Type: value
bad(ty Type) Type := runtime(ty)
";
    let typed = transform_prepared(source);
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "compile-time-runtime-call"),
        "expected runtime-call diagnostic, got {:?}",
        typed.all_flaws()
    );
}
