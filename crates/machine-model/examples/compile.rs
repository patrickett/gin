//! `cargo run -p machine-model --example compile -- file.gin`
//!
//! Gin source to arm64: check, lower each function by the rules, prove the lowering
//! against the semantics, and print the instructions with the words the Gin library
//! encodes them to.
use machine_model::encode::Arm64;
use machine_model::lower::lower;
use typecheck::{Limits, check_file};

fn main() {
    let path = std::env::args().nth(1).expect("usage: compile <file.gin>");
    let text = std::fs::read_to_string(&path).expect("readable source");
    let file = parser::parse_strict(&text).expect("source parses");
    let checked = check_file(&file, Limits::default()).expect("source checks");
    let arm64 = Arm64::load().expect("arm64 library checks");
    let convention = arm64.convention().expect("the library states a convention");
    for (function, body) in checked.functions.iter().zip(&checked.bodies) {
        println!("{}:", function.name);
        let Some(body) = body else {
            println!("    (no recorded body)\n");
            continue;
        };
        match lower(&convention, function, body) {
            Ok(lowered) => {
                for instruction in &lowered.instructions {
                    let word = arm64.encode(instruction).expect("encodes").word;
                    println!(
                        "    {word:08X}  {:<24} {instruction}",
                        instruction.mnemonic()
                    );
                }
                let (low, high) = lowered.result;
                println!("    proven: x0 in {low} to {high} at ret\n");
            }
            Err(error) => println!("    not lowered: {error}\n"),
        }
    }
}
