//! A `mut` parameter changed on some paths only: each exit (or arm) hands back its own value
//! through a `Bundle`, and the body's outputs are `Part`s of the chosen one.
use typecheck::{CheckedSourceBody, CheckedSourceBodyKind, Limits, check_file};

fn body(text: &str, name: &str) -> Option<CheckedSourceBody> {
    let file = parser::parse_strict(text).expect("strict fixture");
    let checked = check_file(&file, Limits::default()).expect("checks");
    let index = checked.functions.iter().position(|f| f.name == name).unwrap();
    checked.bodies[index].clone()
}

fn outputs_are_parts(body: &CheckedSourceBody) -> bool {
    !body.outputs.is_empty()
        && body
            .outputs
            .iter()
            .all(|id| matches!(body.expressions[*id as usize].kind, CheckedSourceBodyKind::Part { .. }))
}

#[test]
fn a_guard_that_mutates_on_one_path_is_recorded_with_an_output_per_exit() {
    let source = "bump(n mut in 0 to 20):\n  when n < 20 then\n    n :: n + 1\n    return\n  return\n";
    let body = body(source, "bump").expect("recorded");
    assert!(outputs_are_parts(&body));
    assert!(body.expressions.iter().any(|e| matches!(e.kind, CheckedSourceBodyKind::Bundle { .. })));
}

#[test]
fn exits_that_agree_keep_the_single_output_shape() {
    // Negative control: nothing differs between the exits, so no bundle is made.
    let source = "same(n mut in 0 to 20, c in 0 to 10):\n  n :: 0\n  when c < 5 then\n    return\n  return\n";
    let body = body(source, "same").expect("recorded");
    assert!(!outputs_are_parts(&body));
    assert!(!body.expressions.iter().any(|e| matches!(e.kind, CheckedSourceBodyKind::Bundle { .. })));
}

#[test]
fn two_exits_that_mutate_different_parameters_each_hand_back_both() {
    let source = "pick(a mut in 0 to 20, b mut in 0 to 20, c in 0 to 10):\n  when c < 5 then\n    a :: 1\n    return\n  b :: 2\n  return\n";
    let body = body(source, "pick").expect("recorded");
    assert_eq!(body.outputs.len(), 2);
    assert!(outputs_are_parts(&body));
}

#[test]
fn a_conditional_value_whose_arm_assigns_a_mut_parameter_selects_it() {
    let source = "f(a mut in 0 to 20, c in 0 to 10) in 0 to 10:\n  r in 0 to 10: when c < 5 then c else 0\n  a :: when c < 5 then 3 else a\n  return r\n";
    assert!(body(source, "f").is_some());
}
