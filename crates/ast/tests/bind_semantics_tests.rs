//! `:=` constant binds vs `:` rebindable binds; comptime classification decoupled from `:=`.

use flask::CompileTarget;
use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::prepare_parse_ast;

mod support;
use support::transform_prepared;

fn prepare(source: &str) -> ast::FileAst {
    let mut ast = TokenCursor::parse_source(source);
    let _ = prepare_parse_ast(&mut ast, &CompileTarget::Library);
    ast
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
fn colon_rejects_a_visible_binding() {
    let typed = transform_prepared("main:\n  x := 1\n  x: 2\n  return x\n");
    assert!(typed.all_flaws().iter().any(|(_, flaw)| {
        flaw.code.slug() == "type-binding-shadows-visible-name" && flaw.arg("name") == Some("x")
    }));
}

#[test]
fn fresh_initializer_resolves_an_existing_distinct_binding() {
    let typed = transform_prepared("main:\n  value: 1\n  next: value\n  return next\n");
    let typecheck::BindBody::Body { exprs, .. } =
        &typed.defs[&typecheck::DefId::named(Intern::from_ref("main"))].body
    else {
        panic!("expected function body");
    };
    let first_place =
        typed.exprs.facts.ownership_facts.place[exprs[0].as_usize()].expect("first place");
    let second_place =
        typed.exprs.facts.ownership_facts.place[exprs[1].as_usize()].expect("second place");
    let typecheck::TypedExprKind::Bind { body, .. } = typed.exprs.kind[exprs[1].as_usize()] else {
        panic!("expected second bind");
    };

    assert_ne!(first_place, second_place);
    assert_ne!(
        typed.places[first_place.0 as usize].binder,
        typed.places[second_place.0 as usize].binder
    );
    assert_eq!(
        typed.exprs.facts.ownership_facts.place[body.as_usize()],
        Some(first_place)
    );
}

#[test]
fn explicit_rebind_creates_a_new_version_of_the_same_place() {
    let typed = transform_prepared("main:\n  value: 1\n  value:: 2\n  return value\n");
    let typecheck::BindBody::Body {
        exprs,
        ret: Some(returned),
    } = &typed.defs[&typecheck::DefId::named(Intern::from_ref("main"))].body
    else {
        panic!("expected function body");
    };
    let initial = typed.exprs.facts.ownership_facts.place_version[exprs[0].as_usize()]
        .expect("initial version");
    let replacement = typed.exprs.facts.ownership_facts.place_version[exprs[1].as_usize()]
        .expect("replacement version");
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
        typed.exprs.facts.ownership_facts.place_version[returned.as_usize()],
        Some(replacement)
    );
    assert_ne!(
        initial_value.integer_knowledge,
        replacement_value.integer_knowledge
    );
}

#[test]
fn conditional_return_preserves_the_incoming_version_after_the_branch() {
    let typed = transform_prepared(
        "main(flag is in 0 to 255):\n  value: 0\n  when flag is 0 then\n    value:: 1\n    return value\n  observed: value\n  return observed\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let typecheck::BindBody::Body { exprs, .. } =
        &typed.defs[&typecheck::DefId::named(Intern::from_ref("main"))].body
    else {
        panic!("expected function body");
    };
    let initial = typed.exprs.facts.ownership_facts.place_version[exprs[0].as_usize()]
        .expect("initial version");
    let typecheck::TypedExprKind::Bind { body: observed, .. } =
        typed.exprs.kind[exprs[2].as_usize()]
    else {
        panic!("expected observed bind");
    };
    assert_eq!(
        typed.exprs.facts.ownership_facts.place_version[observed.as_usize()],
        Some(initial)
    );
    let typecheck::TypedExprKind::When(branch) = &typed.exprs.kind[exprs[1].as_usize()] else {
        panic!("expected conditional return");
    };
    let typecheck::TypedWhenArm::Is { body, .. } = &branch.arms[0] else {
        panic!("expected pattern arm");
    };
    let typecheck::TypedExprKind::Block {
        statements,
        value: None,
    } = &typed.exprs.kind[body.as_usize()]
    else {
        panic!("expected branch block");
    };
    let typecheck::TypedExprKind::Return {
        value: Some(returned),
        ..
    } = &typed.exprs.kind[statements[1].as_usize()]
    else {
        panic!("expected branch return");
    };
    let replacement = typed.exprs.facts.ownership_facts.place_version[statements[0].as_usize()]
        .expect("replacement version");
    assert_ne!(replacement, initial);
    assert_eq!(
        typed.exprs.facts.ownership_facts.place_version[returned.as_usize()],
        Some(replacement)
    );
}

#[test]
fn loop_rebind_creates_zero_iteration_phi() {
    let typed = transform_prepared(
        "main(flag is in 0 to 255):\n  value: 0\n  while flag is 0 do\n    value:: 1\n  observed: value\n  return observed\n",
    );
    let typecheck::BindBody::Body { exprs, .. } =
        &typed.defs[&typecheck::DefId::named(Intern::from_ref("main"))].body
    else {
        panic!("expected function body");
    };
    let initial = typed.exprs.facts.ownership_facts.place_version[exprs[0].as_usize()]
        .expect("initial version");
    let typecheck::TypedExprKind::Bind { body: observed, .. } =
        typed.exprs.kind[exprs[2].as_usize()]
    else {
        panic!("expected observed bind");
    };
    let phi =
        typed.exprs.facts.ownership_facts.place_version[observed.as_usize()].expect("loop phi");

    let typecheck::PlaceVersionOrigin::LoopPhi {
        incoming,
        backedges,
    } = &typed.place_versions[phi.0 as usize].origin
    else {
        panic!("expected loop phi");
    };
    assert_eq!(*incoming, initial);
    assert_eq!(backedges.len(), 1);
    let typecheck::PlaceVersionOrigin::Rebind {
        predecessor: Some(predecessor),
        ..
    } = typed.place_versions[backedges[0].0 as usize].origin
    else {
        panic!("expected loop rebind");
    };
    assert!(
        matches!(&typed.place_versions[predecessor.0 as usize].origin,
        typecheck::PlaceVersionOrigin::Join(versions) if versions.as_slice() == [phi])
    );
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
        "main(flag is in 0 to 255):\n  value: 0\n  while flag is 0 do\n    value:: 1\n  return value\n",
    );

    assert!(typed.all_flaws().iter().any(|(_, flaw)| {
        flaw.code.slug() == "type-cyclic-place-requires-explicit-contract"
            && flaw.arg("name") == Some("value")
    }));
}

#[test]
fn explicitly_bounded_loop_carried_place_is_accepted() {
    let typed = transform_prepared(
        "Small is in 0 to 2\nmain(flag is in 0 to 255):\n  value Small: 0\n  while flag is 0 do\n    value:: 1\n  return value\n",
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
        "Int is in 0 to 255\nCoord has\n    x Int, y Int\n    finish(self):\n      return\nmain:\n  point: Coord(x: 1, y: 2)\n  point.x:: 10\n  observed: point.x\n  return observed\n",
    );
    let typecheck::BindBody::Body { exprs, .. } =
        &typed.defs[&typecheck::DefId::named(Intern::from_ref("main"))].body
    else {
        panic!("expected function body");
    };
    let root = typed.exprs.facts.ownership_facts.place[exprs[0].as_usize()].expect("root place");
    let field = typed.exprs.facts.ownership_facts.place[exprs[1].as_usize()].expect("field place");
    let replacement = typed.exprs.facts.ownership_facts.place_version[exprs[1].as_usize()]
        .expect("field version");
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
    assert_eq!(
        typed.exprs.facts.ownership_facts.place[observed.as_usize()],
        Some(field)
    );
    assert_eq!(
        typed.exprs.facts.ownership_facts.place_version[observed.as_usize()],
        Some(replacement)
    );
}

#[test]
fn projected_rebind_cannot_write_through_immutable_root() {
    let typed = transform_prepared(
        "Int is in 0 to 255\nCoord has\n    x Int\n    finish(self):\n      return\nmain:\n  point := Coord(x: 1)\n  point.x:: 2\n  return 0\n",
    );

    assert!(typed.all_flaws().iter().any(|(_, flaw)| {
        flaw.code.slug() == "type-rebind-immutable" && flaw.arg("name") == Some("point")
    }));
}

#[test]
fn projected_rebind_must_discharge_non_discardable_old_value() {
    let typed = transform_prepared(
        "Int is in 0 to 255\nPayload has pointer @Int\nContainer has payload Payload\nmain:\n  pointer: @1\n  container: Container(payload: Payload(pointer: pointer))\n  container.payload:: Payload(pointer: @2)\n  return 0\n",
    );

    assert!(typed.all_flaws().iter().any(|(_, flaw)| {
        flaw.code.slug() == "type-rebind-would-discard-value"
            && flaw.arg("name") == Some("container")
    }));
}

#[test]
fn conditional_return_preserves_the_incoming_field_version_after_the_branch() {
    let typed = transform_prepared(
        "Coord(t) has\n    x t\n    finish(self):\n      return\nmain(flag is in 0 to 255):\n  point: Coord(x: 2)\n  when flag is 0 then\n    point.x:: 1\n    return point.x\n  observed: point.x\n  return observed\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let typecheck::BindBody::Body { exprs, .. } =
        &typed.defs[&typecheck::DefId::named(Intern::from_ref("main"))].body
    else {
        panic!("expected function body");
    };
    let initial = typed.exprs.facts.ownership_facts.place_version[exprs[0].as_usize()]
        .expect("initial root version");
    let typecheck::TypedExprKind::Bind { body: observed, .. } =
        typed.exprs.kind[exprs[2].as_usize()]
    else {
        panic!("expected observed bind");
    };
    let version = typed.exprs.facts.ownership_facts.place_version[observed.as_usize()]
        .expect("field version");
    assert!(matches!(
        typed.place_versions[version.0 as usize].origin,
        typecheck::PlaceVersionOrigin::Projection { base } if base == initial
    ));
    let typecheck::TypedExprKind::When(branch) = &typed.exprs.kind[exprs[1].as_usize()] else {
        panic!("expected conditional return");
    };
    let typecheck::TypedWhenArm::Is { body, .. } = &branch.arms[0] else {
        panic!("expected pattern arm");
    };
    let typecheck::TypedExprKind::Block {
        statements,
        value: None,
    } = &typed.exprs.kind[body.as_usize()]
    else {
        panic!("expected branch block");
    };
    let typecheck::TypedExprKind::Return {
        value: Some(returned),
        ..
    } = &typed.exprs.kind[statements[1].as_usize()]
    else {
        panic!("expected branch return");
    };
    let replacement = typed.exprs.facts.ownership_facts.place_version[statements[0].as_usize()]
        .expect("replacement version");
    assert_ne!(replacement, version);
    assert_eq!(
        typed.exprs.facts.ownership_facts.place_version[returned.as_usize()],
        Some(replacement)
    );
}

#[test]
fn compound_name_rebind_uses_ordinary_dispatch_and_old_version() {
    let typed = transform_prepared(
        r#"Word is in 0 to 255
add_bits(a Word, b Word) Word:
    left is in 0 to 255: a
    right is in 0 to 255: b
    return ((left + right) & 255) as Word
#infix('+')
#inline
word_add(a Word, b Word) Word: return add_bits(a, b)
main(input Word):
  value Word: input
  value+: 2
  return value
"#,
    );
    let typecheck::BindBody::Body { exprs, .. } =
        &typed.defs[&typecheck::DefId::named(Intern::from_ref("main"))].body
    else {
        panic!("expected function body");
    };
    let initial = typed.exprs.facts.ownership_facts.place_version[exprs[0].as_usize()]
        .expect("initial version");
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
            args: Some(args), ..
        }
        | typecheck::TypedExprKind::IntrinsicCall {
            op: typecheck::intrinsic::IntrinsicOp::BitsAdd,
            args,
        } => args,
        kind => panic!("expected ordinary addition dispatch, got {kind:?}"),
    };
    assert_eq!(args.len(), 2);
    assert_eq!(
        typed.exprs.facts.ownership_facts.place_version[args[0].as_usize()],
        Some(initial)
    );
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
fn constant_callable_accepts_runtime_arguments() {
    let typed = transform_prepared(
        "runtime(value): return value\nconstant(value) := return runtime(value)\nmain(input is in 0 to 255): return constant(input)\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(typed.defs[&typecheck::DefId::named(Intern::from_ref("constant"))].is_constant);
}
