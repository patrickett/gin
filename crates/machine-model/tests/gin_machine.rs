//! The machine model in Gin against the Rust one: same answers on every instruction,
//! operand choice (including aliased ones) and edge value.
use machine_model::concrete::Concrete;
use machine_model::encode::Arm64;
use machine_model::gin_semantics::{GinError, REGISTERS};
use machine_model::instruction::{Instruction, Reg};
use machine_model::program::Data;
use machine_model::semantics::{Machine, execute};

const EDGES: [u64; 7] = [0, 1, 2, 3, u64::MAX, u64::MAX - 1, 1 << 63];

fn rust(instruction: &Instruction, registers: [u64; REGISTERS]) -> [u64; REGISTERS] {
    let mut machine = Concrete::new(&Data::new());
    for (index, value) in registers.into_iter().enumerate() {
        machine.set(Reg(index as u8), value);
    }
    execute(&mut machine, instruction).expect("the Rust model executes it");
    let mut after = [0; REGISTERS];
    for (index, slot) in after.iter_mut().enumerate() {
        *slot = machine.register(Reg(index as u8)).expect("set");
    }
    after
}

fn instructions() -> Vec<Instruction> {
    let registers = (0..REGISTERS as u8).map(Reg);
    let mut all = Vec::new();
    for dest in registers.clone() {
        for left in registers.clone() {
            for right in registers.clone() {
                all.push(Instruction::Add { dest, left, right });
                all.push(Instruction::Sub { dest, left, right });
                all.push(Instruction::Mul { dest, left, right });
            }
        }
        for src in registers.clone() {
            all.push(Instruction::Mov { dest, src });
        }
    }
    all
}

#[test]
fn gin_and_rust_agree_on_every_instruction_and_edge_value() {
    let arm64 = Arm64::load().expect("the Gin library checks");
    let gin = arm64.machine();
    let mut compared = 0;
    for instruction in instructions() {
        for a in EDGES {
            for b in EDGES {
                for c in [0, 7, u64::MAX] {
                    let before = [a, b, c];
                    assert_eq!(
                        gin.step(&instruction, before),
                        Ok(rust(&instruction, before)),
                        "{instruction} on {before:?}"
                    );
                    compared += 1;
                }
            }
        }
    }
    assert_eq!(compared, 90 * 7 * 7 * 3);
}

#[test]
fn addition_wraps_modulo_two_to_the_64() {
    let arm64 = Arm64::load().unwrap();
    let add = Instruction::Add {
        dest: Reg(2),
        left: Reg(0),
        right: Reg(1),
    };
    assert_eq!(
        arm64.machine().step(&add, [u64::MAX, 1, 9]),
        Ok([u64::MAX, 1, 0])
    );
    assert_eq!(
        arm64.machine().step(&add, [u64::MAX, u64::MAX, 9]),
        Ok([u64::MAX, u64::MAX, u64::MAX - 1])
    );
}

#[test]
fn subtraction_and_multiplication_wrap_too() {
    let arm64 = Arm64::load().unwrap();
    let gin = arm64.machine();
    let (dest, left, right) = (Reg(2), Reg(0), Reg(1));
    assert_eq!(
        gin.step(&Instruction::Sub { dest, left, right }, [0, 1, 0]),
        Ok([0, 1, u64::MAX])
    );
    assert_eq!(
        gin.step(
            &Instruction::Mul { dest, left, right },
            [1 << 32, 1 << 32, 5]
        ),
        Ok([1 << 32, 1 << 32, 0])
    );
}

#[test]
fn what_the_gin_machine_does_not_model_is_refused_not_guessed() {
    let arm64 = Arm64::load().unwrap();
    let gin = arm64.machine();
    assert_eq!(
        gin.step(&Instruction::Ret, [0; REGISTERS]),
        Err(GinError::Instruction("Ret".to_owned()))
    );
    let outside = Instruction::Mov {
        dest: Reg(3),
        src: Reg(0),
    };
    assert_eq!(
        gin.step(&outside, [0; REGISTERS]),
        Err(GinError::Register(Reg(3)))
    );
}
