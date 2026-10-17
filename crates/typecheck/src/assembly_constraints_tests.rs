use super::*;
use crate::typed::ExprId;

fn checked() -> CheckedAssembly {
    CheckedAssembly::from_spec(
        AssemblySpec {
            target: "x86_64-unknown-linux-gnu".into(),
            dialect: "att".into(),
            inputs: vec![
                AssemblyInput {
                    parameter: "value".into(),
                    register: "rax".into(),
                    tie: true,
                },
                AssemblyInput {
                    parameter: "other".into(),
                    register: "rbx".into(),
                    tie: false,
                },
            ],
            result: Some("rax".into()),
            outputs: Vec::new(),
            clobbers: Vec::new(),
            memory: AssemblyMemory::default(),
            side_effects: false,
            template: String::new(),
        },
        vec![
            (Intern::from_ref("value"), ExprId(0)),
            (Intern::from_ref("other"), ExprId(1)),
        ],
        vec![Some(0), None],
        ExprId(2),
    )
}

#[test]
fn valid_checked_ties_preserve_constraint_order() {
    assert_eq!(checked().llvm_constraints(), vec!["={rax}", "0", "{rbx}"]);
}

#[test]
fn ties_can_reference_a_later_record_output() {
    let mut assembly = checked();
    assembly.result = None;
    assembly.outputs = vec![
        AssemblyOutput {
            name: "first".into(),
            register: "rcx".into(),
        },
        AssemblyOutput {
            name: "second".into(),
            register: "rax".into(),
        },
    ];
    assembly.tied_output_indices[0] = Some(1);
    assert_eq!(
        assembly.llvm_constraints(),
        vec!["={rcx}", "={rax}", "1", "{rbx}"]
    );
}

#[test]
#[should_panic(expected = "checked assembly tie count must match inputs")]
fn missing_tie_entries_do_not_drop_inputs() {
    let mut assembly = checked();
    assembly.tied_output_indices.pop();
    assembly.llvm_constraints();
}

#[test]
#[should_panic(expected = "checked assembly tie count must match inputs")]
fn excess_tie_entries_are_rejected() {
    let mut assembly = checked();
    assembly.tied_output_indices.push(None);
    assembly.llvm_constraints();
}

#[test]
#[should_panic(expected = "checked tied input must identify an output")]
fn missing_tied_output_does_not_default_to_zero() {
    let mut assembly = checked();
    assembly.tied_output_indices[0] = None;
    assembly.llvm_constraints();
}

#[test]
#[should_panic(expected = "checked tied output must match the input register")]
fn nonexistent_tied_output_is_rejected() {
    let mut assembly = checked();
    assembly.tied_output_indices[0] = Some(1);
    assembly.llvm_constraints();
}

#[test]
#[should_panic(expected = "checked tied output must match the input register")]
fn a_tie_to_the_wrong_register_is_rejected() {
    let mut assembly = checked();
    assembly.result = Some("rcx".into());
    assembly.llvm_constraints();
}

#[test]
#[should_panic(expected = "checked untied input must not identify an output")]
fn untied_inputs_do_not_ignore_output_indices() {
    let mut assembly = checked();
    assembly.tied_output_indices[1] = Some(0);
    assembly.llvm_constraints();
}
