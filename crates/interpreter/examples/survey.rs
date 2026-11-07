//! `cargo run -p interpreter --example survey <files...>`: for each single-file program,
//! whether it parses, checks, and how many of its functions have a recorded body.
use typecheck::{Limits, check_module};
fn main() {
    let (mut files, mut checked, mut functions, mut bodies) = (0, 0, 0, 0);
    for path in std::env::args().skip(1) {
        files += 1;
        let text = std::fs::read_to_string(&path).unwrap();
        let Ok(file) = parser::parse_strict(&text) else { println!("{path}: parse error"); continue };
        let Ok(module) = check_module("main", &file, &[], Limits::default()) else { println!("{path}: does not check alone"); continue };
        checked += 1;
        let c = module.checked();
        let b = c.bodies.iter().filter(|b| b.is_some()).count();
        let i = c.functions.iter().filter(|f| f.instruction.is_some()).count();
        functions += c.functions.len();
        bodies += b;
        println!("{path}: {} functions, {b} with bodies, {i} instructions", c.functions.len());
    }
    println!("{files} files, {checked} check alone, {bodies}/{functions} functions have bodies");
}
