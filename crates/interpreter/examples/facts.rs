use typecheck::{Limits, check_module_diagnostic};
fn main() {
    let text = std::fs::read_to_string(std::env::args().nth(1).unwrap()).unwrap();
    let file = parser::parse_strict(&text).unwrap();
    match check_module_diagnostic("main", &file, &[], Limits::default()) {
        Err(f) => {
            println!("ERR {:?} detail {:?}", f.error, f.detail);
            println!("span {:?} culprit {:?}", f.span, f.culprit);
            if let (Some(s), Some(c)) = (f.span, f.culprit) { println!("culprit text: {:?}", &text[c.start()..c.end()]); let _ = s; }
            for x in &f.facts { println!("  {x:?}"); }
        }
        Ok(m) => for n in &m.checked().notes { println!("note {:?} {:?} facts:", n.kind, n.span); for x in &n.facts { println!("  {x:?}"); } },
    }
}
