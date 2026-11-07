//! Call nodes in checked bodies: callee identity is an index, never a string.
use typecheck::{
    CheckedSourceBodyKind, CheckedSourceCallee, CheckedSourceModule, Limits, check_module,
    check_module_files,
};

const LIBRARY: &str = "five() in 0 to 16: return 5\nid(x in 0 to 16) in 0 to 16: return x\n";

fn module(name: &str, text: &str, dependencies: &[&CheckedSourceModule]) -> CheckedSourceModule {
    let file = parser::parse_strict(text).expect("strict call fixture");
    check_module(name, &file, dependencies, Limits::default()).expect("call fixture checks")
}

fn returned_call(
    module: &CheckedSourceModule,
    function: usize,
) -> (
    CheckedSourceCallee,
    Vec<u32>,
    typecheck::CheckedSourceBody,
) {
    let body = module.checked().bodies[function]
        .clone()
        .expect("call body was recorded");
    let CheckedSourceBodyKind::Block { result, .. } = body.expressions[body.root as usize].kind
    else {
        panic!("root is a block");
    };
    let CheckedSourceBodyKind::Return { value } = body.expressions[result as usize].kind else {
        panic!("block ends in a return");
    };
    let CheckedSourceBodyKind::Call { callee, arguments, .. } =
        body.expressions[value as usize].kind.clone()
    else {
        panic!("return carries a call");
    };
    (callee, arguments, body)
}

#[test]
fn a_local_call_names_its_callee_by_index() {
    let module = module(
        "app",
        "id(x in 0 to 16) in 0 to 16: return x\nagain(x in 0 to 16) in 0 to 16: return id(x)\n",
        &[],
    );
    let (callee, arguments, _) = returned_call(&module, 1);
    assert_eq!(callee, CheckedSourceCallee::Local { function: 0 });
    assert_eq!(arguments.len(), 1);
}

#[test]
fn a_dependency_call_names_the_dependency_slot_and_function() {
    let library = module("lib", LIBRARY, &[]);
    let other = module("other", "zero() in 0 to 1: return 0\n", &[]);
    let app = module(
        "app",
        "use lib id\nwrapped(x in 0 to 16) in 0 to 16: return id(x)\n",
        &[&other, &library],
    );
    let (callee, arguments, body) = returned_call(&app, 0);
    assert_eq!(
        callee,
        CheckedSourceCallee::Dependency {
            dependency: 1,
            function: 1
        }
    );
    assert_eq!(arguments.len(), 1);
    assert_eq!(
        body.expressions[arguments[0] as usize].kind,
        CheckedSourceBodyKind::Parameter { index: 0 }
    );
}

#[test]
fn arguments_may_be_parameters_and_nested_calls() {
    let library = module("lib", LIBRARY, &[]);
    let app = module(
        "app",
        "use lib id\nuse lib five\nthrough(x in 0 to 16) in 0 to 16: return id(id(x))\n",
        &[&library],
    );
    let (_, arguments, body) = returned_call(&app, 0);
    let inner = &body.expressions[arguments[0] as usize];
    assert!(matches!(inner.kind, CheckedSourceBodyKind::Call { .. }));
}

#[test]
fn bodies_stay_aligned_with_functions_for_calls_across_files() {
    let files: Vec<_> = [
        "id(x in 0 to 16) in 0 to 16: return x\n",
        "again(x in 0 to 16) in 0 to 16: return id(x)\n",
    ]
    .iter()
    .enumerate()
    .map(|(index, text)| parser::parse_strict_file(text, index as u32).unwrap())
    .collect();
    let module = check_module_files("app", &files, &[], Limits::default()).unwrap();
    assert!(module.checked().bodies.iter().all(Option::is_some));
    let (callee, _, _) = returned_call(&module, 1);
    assert_eq!(callee, CheckedSourceCallee::Local { function: 0 });
}

#[test]
fn calls_with_range_locals_are_recorded() {
    let module = module(
        "app",
        "five() in 0 to 16: return 5\nbound() in 0 to 16:\n  x in 0 to 16: five()\n  return x\n",
        &[],
    );
    assert!(module.checked().bodies[1].is_some());
}
