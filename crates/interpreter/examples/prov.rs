use typecheck::{Limits, check_module};
fn main() {
    let text = std::fs::read_to_string(std::env::args().nth(1).unwrap()).unwrap();
    let file = parser::parse_strict(&text).unwrap();
    let m = check_module("main", &file, &[], Limits::default()).unwrap();
    let c = m.checked();
    for (i, d) in &c.declines { let s = d.span.map(|s| &text[s.start()..s.end()]); println!("decline fn {} {:?} {:?} {:?}", c.functions[*i as usize].name, d.kind, d.construct, s); }
    for (i, f) in &c.folds { println!("fold in {} -> {:?}", c.functions[*i as usize].name, f); }
    for f in &c.functions { if f.provided_by.is_some() { println!("provided {} by {:?}", f.name, f.provided_by); } }
}
