use typecheck::{Limits, check_module};
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let text = std::fs::read_to_string(path).unwrap();
    let file = match parser::parse_strict(&text) { Ok(f) => f, Err(e) => { println!("PARSE {:?}", e.iter().map(|d| d.message.clone()).collect::<Vec<_>>()); return; } };
    let module = match check_module("main", &file, &[], Limits::default()) { Ok(m) => m, Err(e) => { println!("CHECK {e:?}"); return; } };
    let c = module.checked();
    println!("entry {:?}", c.entry);
    for (i, f) in c.functions.iter().enumerate() {
        println!("fn {i} {} params {:?} result {:?} tp {:?}", f.name, f.parameters, f.result, f.type_parameters);
        match &c.bodies[i] {
            None => println!("  <no body>"),
            Some(b) => { println!("  effects {} root {} outputs {:?}", b.effects, b.root, b.outputs);
                for (n, e) in b.expressions.iter().enumerate() { println!("  {n}: {:?} :: {:?}", e.kind, e.ty); } }
        }
    }
    for (i, a) in c.aggregates.iter().enumerate() { println!("agg {i} {} {:?} fields {:?} variants {:?}", a.name, a.kind, a.fields.iter().map(|f| (&f.name, f.ty)).collect::<Vec<_>>(), a.variants); }
    for (i, a) in c.instances.iter().enumerate() { println!("inst {i} {:?} {:?}", a.head, a.arguments); }
}
