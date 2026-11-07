//! Dependency capsules carry the recorded bodies of their own functions, so a call into
//! another module whose arguments are all known is evaluated like a local one.
use i256::I256;
use typecheck::{
    CheckedSourceBodyKind, CheckedSourceCallee, CheckedSourceModule, Limits, check_module,
};

fn module(name: &str, text: &str, dependencies: &[&CheckedSourceModule]) -> CheckedSourceModule {
    let file = parser::parse_strict(text).expect("strict capsule fixture");
    check_module(name, &file, dependencies, Limits::default()).expect("capsule fixture checks")
}

/// The expression the single `return` of function `index` yields.
fn returned(module: &CheckedSourceModule, index: usize) -> CheckedSourceBodyKind {
    let body = module.checked().bodies[index].as_ref().expect("recorded");
    let CheckedSourceBodyKind::Block { result, .. } = body.expressions[body.root as usize].kind
    else {
        panic!("root is a block");
    };
    let CheckedSourceBodyKind::Return { value } = body.expressions[result as usize].kind else {
        panic!("block ends in a return");
    };
    body.expressions[value as usize].kind.clone()
}

const LIBRARY: &str = "id(x in 0 to 16) in 0 to 16: return x\n\
                       double(x in 0 to 8) in 0 to 16: return x + x\n";

#[test]
fn an_imported_call_with_known_arguments_is_its_value() {
    let library = module("lib", LIBRARY, &[]);
    let app = module(
        "app",
        "use lib id\nuse lib double\n\
         seven() in 0 to 16: return id(7)\n\
         nine() in 0 to 17: return double(4) + 1\n",
        &[&library],
    );
    assert_eq!(
        returned(&app, 0),
        CheckedSourceBodyKind::Integer(I256::from(7))
    );
    assert_eq!(
        returned(&app, 1),
        CheckedSourceBodyKind::Integer(I256::from(9))
    );
}

#[test]
fn an_imported_call_with_an_unknown_argument_stays_a_call() {
    let library = module("lib", LIBRARY, &[]);
    let app = module(
        "app",
        "use lib id\nthrough(x in 0 to 16) in 0 to 16: return id(x)\n",
        &[&library],
    );
    assert!(matches!(
        returned(&app, 0),
        CheckedSourceBodyKind::Call {
            callee: CheckedSourceCallee::Dependency { .. },
            ..
        }
    ));
}

#[test]
fn a_callee_that_calls_its_own_module_is_evaluated_through_that_module() {
    // `twice` sits at position 2 of the library, so its call to `id` must resolve
    // relative to the library's table, not the importer's.
    let library = module(
        "lib",
        "id(x in 0 to 16) in 0 to 16: return x\n\
         pad() in 0 to 1: return 0\n\
         twice(x in 0 to 16) in 0 to 32: return id(x) + id(x)\n",
        &[],
    );
    let app = module(
        "app",
        "use lib twice\nr() in 0 to 32: return twice(4)\n",
        &[&library],
    );
    assert_eq!(
        returned(&app, 0),
        CheckedSourceBodyKind::Integer(I256::from(8))
    );
}

#[test]
fn evaluation_stops_at_a_dependency_of_a_dependency() {
    // `wrap` calls `id` from another module; the importer cannot resolve that module's
    // table, so the call stays a call instead of being guessed.
    let util = module("util", LIBRARY, &[]);
    let library = module(
        "lib",
        "use util id\nwrap(x in 0 to 16) in 0 to 16: return id(x)\n",
        &[&util],
    );
    let app = module(
        "app",
        "use lib wrap\nr() in 0 to 16: return wrap(3)\n",
        &[&library],
    );
    assert!(matches!(
        returned(&app, 0),
        CheckedSourceBodyKind::Call { .. }
    ));
}

#[test]
fn editing_a_body_changes_what_dependents_fold_to() {
    // The accepted cost of the rule: the signature is identical, only the body differs,
    // and the dependent's recorded value follows the body.
    let call = "use lib five\nanswer() in 0 to 16: return five()\n";
    let before = module("lib", "five() in 0 to 16: return 5\n", &[]);
    let after = module("lib", "five() in 0 to 16: return 6\n", &[]);
    assert_eq!(
        returned(&module("app", call, &[&before]), 0),
        CheckedSourceBodyKind::Integer(I256::from(5))
    );
    assert_eq!(
        returned(&module("app", call, &[&after]), 0),
        CheckedSourceBodyKind::Integer(I256::from(6))
    );
}
